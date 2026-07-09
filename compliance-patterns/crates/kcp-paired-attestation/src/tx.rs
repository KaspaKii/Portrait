//! On-chain creation and anchoring of paired-attestation UTXOs.
//!
//! This module builds, signs, and submits the two core transactions of the
//! paired-attestation pattern:
//!
//! - [`create_attestation_tx`] — locks [`DEFAULT_ATTESTATION_VALUE_SOMPI`] to
//!   the wallet's address with the `PartyACommit` payload (`seq = 0`) in the
//!   transaction payload field. A change output returns excess funds to the
//!   funding wallet.
//! - [`append_mate_tx`] — spends the attestation UTXO and re-locks the value
//!   (minus fee) back to the same wallet address, carrying the `PartyBMate`
//!   payload (`seq = 1`).
//!
//! ## Single-publisher lineage (v0)
//!
//! In v0, **one wallet anchors both steps**. This is a deliberate scope-down:
//! the full two-party on-chain datasig binding (two independent
//! `checkDataSig` checks inside one covenant entry-point) is proven viable —
//! FACTS SS-024-v4 confirms `OpCheckSigFromStack` binds key+message correctly
//! on v2.0.0 — but it additionally requires P2SH spend-path plumbing not yet
//! in `kcp-common`. That work is the documented next step. In v0, mating is
//! verified **off-chain** by [`crate::mate::verify_mate`] before the wallet
//! anchors the `PartyBMate` transaction.
//!
//! ## Fee handling
//!
//! Both transactions use [`CARRIER_FEE_SOMPI`] from `kcp-common`.
//! `create_attestation_tx` deducts the fee from the change output.
//! `append_mate_tx` deducts the fee from the attestation UTXO value itself.
//!
//! ## Default attestation value
//!
//! [`DEFAULT_ATTESTATION_VALUE_SOMPI`] is `10_000_000` sompi (0.1 KAS).
//! The KIP-0009 storage-mass formula:
//!
//! ```text
//! storage_mass ≈ C · p² / amount,  C = 10¹²
//! ```
//!
//! With a 79-byte payload (`p ≈ 380` mass units for the output) and
//! `amount = 10_000_000`:
//!
//! ```text
//! storage_mass ≈ 10¹² · 380² / 10_000_000 ≈ 14_440
//! ```
//!
//! well below the `10⁶` cap.

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
/// Change below this threshold is folded into the fee: zero-value outputs are
/// non-standard ("dust") and small outputs violate the KIP-0009 storage-mass
/// bound (`mass ≈ 10¹²/amount` must stay `≤ 10⁶`, so `amount ≥ 10⁶ sompi`).
pub const MIN_CHANGE_SOMPI: u64 = 1_000_000;

/// Default attestation UTXO value in sompi (0.1 KAS).
///
/// Storage-mass at 79-byte payload keeps well within the KIP-0009 cap of 10⁶
/// (see module-level docs for the calculation).
pub const DEFAULT_ATTESTATION_VALUE_SOMPI: u64 = 10_000_000;

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

/// Create a new paired-attestation UTXO (Party A's commitment, `seq = 0`).
///
/// Selects a funding UTXO from `wallet`, builds a 2-output transaction:
///
/// - output 0: `value_sompi` locked to `wallet.address`, carrying
///   `payload_bytes` (`PartyACommit`, `seq = 0`) in the transaction payload.
/// - output 1: change returned to `wallet.address` (input amount minus
///   `value_sompi` minus fee). Omitted if change is below [`MIN_CHANGE_SOMPI`].
///
/// Returns the submitted transaction id as a string.
///
/// # Errors
///
/// Returns [`Error::Rpc`] on RPC failure, or if no suitable funding UTXO is
/// found.
pub async fn create_attestation_tx(
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
    let attestation_output =
        TransactionOutput::new(value_sompi, pay_to_address_script(&wallet.address));

    // Only emit a change output when it is KIP-0009-safe. Below the threshold,
    // fold change into fee.
    let mut outputs = vec![attestation_output];
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

/// Append the mate step to an existing paired-attestation UTXO (`seq = 1`).
///
/// Fetches the attestation UTXO from `wallet` by scanning its UTXOs for the
/// given `attestation_outpoint`. Builds a 1-output transaction:
///
/// - input: the attestation UTXO (signed with `wallet.keypair`)
/// - output 0: `(attestation_value - fee)` re-locked to `wallet.address`,
///   carrying `payload_bytes` (`PartyBMate`, `seq = 1`) in the transaction
///   payload field.
///
/// Returns the submitted transaction id as a string.
///
/// # Errors
///
/// Returns [`Error::Rpc`] if the outpoint is not found among the wallet's
/// UTXOs, if the UTXO value does not cover the fee, or if the RPC call fails.
pub async fn append_mate_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    attestation_outpoint: (kaspa_consensus_core::tx::TransactionId, u32),
    payload_bytes: Vec<u8>,
) -> Result<String> {
    let entries = client
        .get_utxos_by_addresses(vec![wallet.address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses: {e}")))?;

    let (target_txid, target_index) = attestation_outpoint;

    let utxo = entries
        .into_iter()
        .find(|e| e.outpoint.transaction_id == target_txid && e.outpoint.index == target_index)
        .ok_or_else(|| {
            Error::Rpc(format!(
                "attestation outpoint {target_txid}:{target_index} not found in wallet {}",
                wallet.address
            ))
        })?;

    let input_amount = utxo.utxo_entry.amount;
    if input_amount <= CARRIER_FEE_SOMPI {
        return Err(Error::Rpc(format!(
            "attestation UTXO value {input_amount} sompi is not above fee {CARRIER_FEE_SOMPI}"
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
