//! On-chain anchoring of KTT token operation evidence.
//!
//! [`anchor_token_op_tx`] creates a Kaspa carrier transaction that embeds a
//! KTT payload in the transaction payload field, anchoring evidence of a
//! token operation on-chain. The output is a pay-to-address UTXO locked to
//! the wallet address (key-controlled in v0).
//!
//! ## Fee handling
//!
//! Uses [`kcp_common::tx::CARRIER_FEE_SOMPI`] (1,000,000 sompi — the v2.0.0
//! relay-fee floor). Change below [`MIN_CHANGE_SOMPI`] is folded into the fee.
//!
//! ## Default token op value
//!
//! [`DEFAULT_TOKEN_OP_VALUE_SOMPI`] is `10_000_000` sompi (0.1 KAS). Storage-
//! mass stays well within the KIP-0009 cap for the 71-byte KTT payload:
//!
//! ```text
//! storage_mass ≈ C · p² / amount,  C = 10¹²
//! p (output mass) ≈ 300 for a 71-byte payload output
//! storage_mass ≈ 10¹² · 300² / 10_000_000 ≈ 9_000
//! ```
//!
//! ## v0 limitation
//!
//! The output UTXO is locked to a plain pay-to-address script. The token
//! state is only in the *evidence payload*; the UTXO itself is not
//! encumbered by the KCC20 covenant. Binding the UTXO to a
//! `validateOutputStateWithTemplate` covenant is the documented next step.

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

/// Minimum change output value in sompi (0.01 KAS).
///
/// Change below this threshold is folded into the fee to avoid dust /
/// KIP-0009 storage-mass violations.
pub const MIN_CHANGE_SOMPI: u64 = 1_000_000;

/// Default token operation UTXO value in sompi (0.1 KAS).
///
/// Storage-mass at the 71-byte payload stays well within the KIP-0009 cap
/// (see module-level documentation for the calculation).
pub const DEFAULT_TOKEN_OP_VALUE_SOMPI: u64 = 10_000_000;

/// Select the smallest UTXO entry whose amount strictly exceeds `min_amount`.
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

/// Anchor a KTT token operation evidence payload on-chain.
///
/// Selects a funding UTXO from `wallet`, builds a transaction with:
/// - output 0: `value_sompi` locked to `wallet.address` (pay-to-address in
///   v0), carrying `payload_bytes` in the transaction payload field.
/// - output 1 (optional): change returned to `wallet.address` when
///   `(input - value - fee) >= MIN_CHANGE_SOMPI`.
///
/// Returns the submitted transaction id as a string.
///
/// # Errors
///
/// Returns [`Error::Rpc`] on RPC failure, or if no suitable funding UTXO is
/// found (e.g. wallet is unfunded).
pub async fn anchor_token_op_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    payload_bytes: Vec<u8>,
    value_sompi: u64,
) -> Result<String> {
    let required = value_sompi + CARRIER_FEE_SOMPI;

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
                 (value {} + fee {})",
            wallet.address, required, value_sompi, CARRIER_FEE_SOMPI
        ))
    })?;

    let outpoint = TransactionOutpoint::new(utxo.outpoint.transaction_id, utxo.outpoint.index);
    let input_amount = utxo.utxo_entry.amount;
    let input_spk = utxo.utxo_entry.script_public_key.clone();
    let block_daa_score = utxo.utxo_entry.block_daa_score;
    let is_coinbase = utxo.utxo_entry.is_coinbase;
    // KIP-20: thread the covenant_id through exactly as kcp-common/tx.rs does.
    let covenant_id = utxo.utxo_entry.covenant_id;

    let change_amount = input_amount - value_sompi - CARRIER_FEE_SOMPI;

    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let token_output = TransactionOutput::new(value_sompi, pay_to_address_script(&wallet.address));

    // Only emit a change output when it is KIP-0009-safe. A tiny (or zero)
    // change output is non-standard ("dust") and small outputs risk the
    // storage-mass limit. Below the threshold, fold change into fee.
    let mut outputs = vec![token_output];
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
