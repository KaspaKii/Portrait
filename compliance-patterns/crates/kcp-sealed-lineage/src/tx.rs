//! On-chain creation and appending of sealed-lineage UTXOs.
//!
//! This module builds, signs, and submits the two core transactions of the
//! sealed-lineage pattern:
//!
//! - [`create_lineage_tx`] — locks `lineage_value_sompi` to the publisher's
//!   address with the genesis payload in the transaction payload field. A
//!   change output returns excess funds to the funding wallet.
//! - [`append_lineage_tx`] — spends the lineage UTXO from the publisher's
//!   wallet and re-locks the value (minus fee) back to the same publisher
//!   address, carrying an updated payload.
//!
//! ## Fee handling
//!
//! Both transactions re-use [`CARRIER_FEE_SOMPI`] from `kcp-common`. For
//! `create_lineage_tx` the fee is deducted from the change output; for
//! `append_lineage_tx` it is deducted from the lineage value itself.
//!
//! ## Default lineage value
//!
//! [`DEFAULT_LINEAGE_VALUE_SOMPI`] is `10_000_000` sompi (0.1 KAS). The
//! storage-mass formula from KIP-0009 is:
//!
//! ```text
//! storage_mass ≈ C · p² / amount,  C = 10¹²
//! ```
//!
//! With a 87-byte payload (`p ≈ 400` in mass units for the output) and
//! `amount = 10_000_000`:
//!
//! ```text
//! storage_mass ≈ 10¹² · 400² / 10_000_000 ≈ 16_000
//! ```
//!
//! well below the `10⁶` cap, with headroom for fee deductions across many
//! appends.
//!
//! ## v0 limitation
//!
//! The locking script is a plain pay-to-address script of the publisher's
//! address (x-only pubkey, `Version::PubKey`). Control is enforced by the
//! key alone; consensus does not yet introspect the payload or verify lineage
//! invariants.

use kaspa_consensus_core::{
    sign::sign,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        SignableTransaction, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput,
        UtxoEntry,
    },
};
use kaspa_rpc_core::{api::rpc::RpcApi, model::tx::RpcTransaction};
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

/// Default lineage UTXO value in sompi (0.1 KAS).
///
/// Storage-mass at 87-byte payload keeps well within the KIP-0009 cap of 10⁶
/// (see module-level docs for the calculation).
pub const DEFAULT_LINEAGE_VALUE_SOMPI: u64 = 10_000_000;

/// Pick the smallest UTXO entry whose amount strictly exceeds `min_amount`.
fn select_smallest_covering(
    entries: Vec<kaspa_rpc_core::RpcUtxosByAddressesEntry>,
    min_amount: u64,
) -> Option<kaspa_rpc_core::RpcUtxosByAddressesEntry> {
    let mut candidates: Vec<_> = entries
        .into_iter()
        .filter(|e| e.utxo_entry.amount > min_amount)
        .collect();
    candidates.sort_by_key(|e| e.utxo_entry.amount);
    candidates.into_iter().next()
}

/// Create a new sealed-lineage UTXO.
///
/// Selects a funding UTXO from `wallet`, builds a 2-output transaction:
/// - output 0: `lineage_value_sompi` locked to `wallet.address`, carrying
///   `payload_bytes` in the transaction payload field (genesis event).
/// - output 1: change returned to `wallet.address` (input amount minus
///   `lineage_value_sompi` minus fee).
///
/// Returns the submitted transaction id as a string.
///
/// # Errors
///
/// Returns [`Error::Rpc`] on RPC failure, or if no suitable funding UTXO
/// is found.
pub async fn create_lineage_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    payload_bytes: Vec<u8>,
    lineage_value_sompi: u64,
) -> Result<String> {
    let required = lineage_value_sompi + CARRIER_FEE_SOMPI;

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
             (lineage_value {} + fee {})",
            wallet.address, required, lineage_value_sompi, CARRIER_FEE_SOMPI
        ))
    })?;

    let outpoint = TransactionOutpoint::new(utxo.outpoint.transaction_id, utxo.outpoint.index);
    let input_amount = utxo.utxo_entry.amount;
    let input_spk = utxo.utxo_entry.script_public_key.clone();
    let block_daa_score = utxo.utxo_entry.block_daa_score;
    let is_coinbase = utxo.utxo_entry.is_coinbase;
    // KIP-20: thread the covenant_id through exactly as kcp-common/tx.rs does.
    let covenant_id = utxo.utxo_entry.covenant_id;

    let change_amount = input_amount - lineage_value_sompi - CARRIER_FEE_SOMPI;

    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let lineage_output =
        TransactionOutput::new(lineage_value_sompi, pay_to_address_script(&wallet.address));

    // Only emit a change output when it is KIP-0009-safe. A tiny (or zero)
    // change output is non-standard ("payment of 0 is dust") and a small one
    // risks the storage-mass limit (mass ≈ 10¹²/amount must stay ≤ 10⁶, so
    // amount must be ≥ 10⁶ sompi). Below the threshold, fold change into fee.
    let mut outputs = vec![lineage_output];
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
        payload_bytes,
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

/// Append to an existing sealed-lineage UTXO.
///
/// Fetches the lineage UTXO from `wallet` by scanning its UTXOs for the given
/// `lineage_outpoint`. Builds a 1-output transaction:
/// - input: the lineage UTXO (signed with `wallet.keypair`)
/// - output 0: `(lineage_value - fee)` re-locked to `wallet.address`,
///   carrying `payload_bytes` in the transaction payload field.
///
/// Returns the submitted transaction id as a string.
///
/// # Errors
///
/// Returns [`Error::Rpc`] if the outpoint is not found among the wallet's
/// UTXOs or if the RPC call fails.
pub async fn append_lineage_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    lineage_outpoint: (kaspa_consensus_core::tx::TransactionId, u32),
    payload_bytes: Vec<u8>,
) -> Result<String> {
    let entries = client
        .get_utxos_by_addresses(vec![wallet.address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses: {e}")))?;

    let (target_txid, target_index) = lineage_outpoint;

    let utxo = entries
        .into_iter()
        .find(|e| e.outpoint.transaction_id == target_txid && e.outpoint.index == target_index)
        .ok_or_else(|| {
            Error::Rpc(format!(
                "lineage outpoint {target_txid}:{target_index} not found in wallet {}",
                wallet.address
            ))
        })?;

    let input_amount = utxo.utxo_entry.amount;
    if input_amount <= CARRIER_FEE_SOMPI {
        return Err(Error::Rpc(format!(
            "lineage UTXO value {input_amount} sompi is not above fee {CARRIER_FEE_SOMPI}"
        )));
    }
    let output_amount = input_amount - CARRIER_FEE_SOMPI;

    let outpoint = TransactionOutpoint::new(target_txid, target_index);
    let input_spk = utxo.utxo_entry.script_public_key.clone();
    let block_daa_score = utxo.utxo_entry.block_daa_score;
    let is_coinbase = utxo.utxo_entry.is_coinbase;
    let covenant_id = utxo.utxo_entry.covenant_id;

    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let output = TransactionOutput::new(output_amount, pay_to_address_script(&wallet.address));

    let tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        payload_bytes,
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
