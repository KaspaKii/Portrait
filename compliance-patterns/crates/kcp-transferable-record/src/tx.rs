//! On-chain creation and transfer of transferable-record UTXOs.
//!
//! This module builds, signs, and submits the two core transactions of the
//! transferable-record pattern:
//!
//! - [`create_record_tx`] — locks `record_value_sompi` to the initial
//!   controller's address, with the record payload in the transaction payload
//!   field. A change output returns excess funds to the funding wallet.
//! - [`transfer_record_tx`] — spends the record UTXO from the current
//!   controller's wallet and re-locks the value (minus fee) to the next
//!   controller's address, with an updated payload.
//!
//! ## Fee handling
//!
//! Both transactions re-use [`CARRIER_FEE_SOMPI`] from `kcp-common`. For
//! `create_record_tx` the fee is deducted from the change output; for
//! `transfer_record_tx` it is deducted from the record value itself.
//!
//! ## v0 limitation
//!
//! The locking script is a plain pay-to-address script of the controller's
//! address (x-only pubkey, `Version::PubKey`). Control is enforced by the key
//! alone; consensus does not yet introspect the payload or verify lineage.

use kaspa_addresses::Address;
use kaspa_consensus_core::{
    sign::sign,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        ScriptPublicKey, SignableTransaction, Transaction, TransactionInput, TransactionOutpoint,
        TransactionOutput, UtxoEntry,
    },
};
use kaspa_rpc_core::{api::rpc::RpcApi, model::tx::RpcTransaction, RpcUtxosByAddressesEntry};
use kaspa_txscript::pay_to_address_script;
use kaspa_wrpc_client::KaspaRpcClient;

use kcp_common::tx::CARRIER_FEE_SOMPI;
use kcp_common::wallet::Wallet;

use crate::error::{Error, Result};

/// Minimum change output value in sompi (0.01 KAS). Change below this is
/// folded into the fee: zero-value outputs are non-standard ("dust") and
/// small outputs violate the KIP-0009 storage-mass bound (mass ≈ 10¹²/amount
/// must stay ≤ 10⁶, so amount must be ≥ 10⁶ sompi).
pub const MIN_CHANGE_SOMPI: u64 = 1_000_000;

/// Build a pay-to-address `ScriptPublicKey` for `address`.
///
/// Returns the locking script suitable for use as a transaction output.
pub fn address_to_spk(address: &Address) -> ScriptPublicKey {
    pay_to_address_script(address)
}

/// Pick the smallest UTXO entry whose amount strictly exceeds `min_amount`.
fn select_smallest_covering(
    entries: Vec<RpcUtxosByAddressesEntry>,
    min_amount: u64,
) -> Option<RpcUtxosByAddressesEntry> {
    let mut candidates: Vec<_> = entries
        .into_iter()
        .filter(|e| e.utxo_entry.amount > min_amount)
        .collect();
    candidates.sort_by_key(|e| e.utxo_entry.amount);
    candidates.into_iter().next()
}

/// Create a new transferable-record UTXO.
///
/// Selects a funding UTXO from `wallet`, builds a 2-output transaction:
/// - output 0: `record_value_sompi` locked to `gate_address` (the initial
///   controller), carrying `record_payload` in the transaction payload field.
/// - output 1: change returned to `wallet.address` (input amount minus
///   `record_value_sompi` minus fee).
///
/// Returns the submitted transaction id as a string.
///
/// # Errors
///
/// Returns [`Error::Rpc`] on RPC failure, or if no suitable UTXO is found.
pub async fn create_record_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    gate_address: &Address,
    record_payload: Vec<u8>,
    record_value_sompi: u64,
) -> Result<String> {
    let required = record_value_sompi + CARRIER_FEE_SOMPI;

    let entries = client
        .get_utxos_by_addresses(vec![wallet.address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses: {e}")))?;

    if entries.is_empty() {
        return Err(Error::Rpc(format!(
            "wallet {} has no UTXOs — fund it from a testnet faucet",
            wallet.address
        )));
    }

    let utxo = select_smallest_covering(entries, required.saturating_sub(1)).ok_or_else(|| {
        Error::Rpc(format!(
            "wallet {} has UTXOs but none above the required {} sompi \
                 (record_value {} + fee {})",
            wallet.address, required, record_value_sompi, CARRIER_FEE_SOMPI
        ))
    })?;

    let outpoint = TransactionOutpoint::new(utxo.outpoint.transaction_id, utxo.outpoint.index);
    let input_amount = utxo.utxo_entry.amount;
    let input_spk = utxo.utxo_entry.script_public_key.clone();
    let block_daa_score = utxo.utxo_entry.block_daa_score;
    let is_coinbase = utxo.utxo_entry.is_coinbase;
    // KIP-20: thread the covenant_id through exactly as kcp-common/tx.rs does.
    let covenant_id = utxo.utxo_entry.covenant_id;

    let change_amount = input_amount - record_value_sompi - CARRIER_FEE_SOMPI;

    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let record_output =
        TransactionOutput::new(record_value_sompi, pay_to_address_script(gate_address));

    // Only emit a change output when it is KIP-0009-safe. A tiny (or zero)
    // change output is non-standard ("payment of 0 is dust") and a small one
    // risks the storage-mass limit (mass ≈ 10¹²/amount must stay ≤ 10⁶, so
    // amount must be ≥ 10⁶ sompi). Below the threshold, fold change into fee.
    let mut outputs = vec![record_output];
    if change_amount >= MIN_CHANGE_SOMPI {
        outputs.push(TransactionOutput::new(
            change_amount,
            pay_to_address_script(&wallet.address),
        ));
    }

    let tx = Transaction::new(
        0,
        vec![input],
        outputs,
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        record_payload,
    );

    let utxo_entry = UtxoEntry::new(
        input_amount,
        input_spk,
        block_daa_score,
        is_coinbase,
        covenant_id,
    );
    let signable = SignableTransaction::with_entries(tx, vec![utxo_entry]);
    let signed = sign(signable, wallet.keypair);

    let rpc_tx: RpcTransaction = (&signed.tx).into();
    let tx_id = client
        .submit_transaction(rpc_tx, false)
        .await
        .map_err(|e| Error::Rpc(format!("submit_transaction: {e}")))?;
    Ok(tx_id.to_string())
}

/// Transfer a record UTXO to a new controller.
///
/// Fetches the record UTXO from `current_controller_wallet` by scanning its
/// UTXOs for the given `record_outpoint`. Builds a 1-output transaction:
/// - input: the record UTXO (signed with `current_controller_wallet.keypair`)
/// - output 0: `(record_value - fee)` locked to `next_controller_address`,
///   carrying `transfer_payload` in the transaction payload field.
///
/// Returns the submitted transaction id as a string.
///
/// # Errors
///
/// Returns [`Error::Rpc`] if the outpoint is not found among the wallet's UTXOs
/// or if the RPC call fails.
pub async fn transfer_record_tx(
    client: &KaspaRpcClient,
    current_controller_wallet: &Wallet,
    record_outpoint: (kaspa_consensus_core::tx::TransactionId, u32),
    next_controller_address: &Address,
    transfer_payload: Vec<u8>,
) -> Result<String> {
    let entries = client
        .get_utxos_by_addresses(vec![current_controller_wallet.address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses: {e}")))?;

    let (target_txid, target_index) = record_outpoint;

    let utxo = entries
        .into_iter()
        .find(|e| e.outpoint.transaction_id == target_txid && e.outpoint.index == target_index)
        .ok_or_else(|| {
            Error::Rpc(format!(
                "record outpoint {target_txid}:{target_index} not found in wallet {}",
                current_controller_wallet.address
            ))
        })?;

    let input_amount = utxo.utxo_entry.amount;
    if input_amount <= CARRIER_FEE_SOMPI {
        return Err(Error::Rpc(format!(
            "record UTXO value {input_amount} sompi is not above fee {CARRIER_FEE_SOMPI}"
        )));
    }
    let output_amount = input_amount - CARRIER_FEE_SOMPI;

    let outpoint = TransactionOutpoint::new(target_txid, target_index);
    let input_spk = utxo.utxo_entry.script_public_key.clone();
    let block_daa_score = utxo.utxo_entry.block_daa_score;
    let is_coinbase = utxo.utxo_entry.is_coinbase;
    let covenant_id = utxo.utxo_entry.covenant_id;

    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let output = TransactionOutput::new(
        output_amount,
        pay_to_address_script(next_controller_address),
    );

    let tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        transfer_payload,
    );

    let utxo_entry = UtxoEntry::new(
        input_amount,
        input_spk,
        block_daa_score,
        is_coinbase,
        covenant_id,
    );
    let signable = SignableTransaction::with_entries(tx, vec![utxo_entry]);
    let signed = sign(signable, current_controller_wallet.keypair);

    let rpc_tx: RpcTransaction = (&signed.tx).into();
    let tx_id = client
        .submit_transaction(rpc_tx, false)
        .await
        .map_err(|e| Error::Rpc(format!("submit_transaction: {e}")))?;
    Ok(tx_id.to_string())
}
