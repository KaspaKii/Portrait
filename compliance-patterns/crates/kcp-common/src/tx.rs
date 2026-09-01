//! Build, sign, and submit Kaspa carrier transactions.
//!
//! A "carrier transaction" is a 1-input/1-output transaction whose payload
//! carries application bytes (e.g. a script digest plus a record commitment).
//! Spending and receiving address are the same — the wallet pays itself,
//! minus a fixed fee. The on-chain result is a real `tx_id` that anyone can
//! look up on a testnet explorer. This is the proven path for capturing
//! testnet evidence; covenant-locked outputs are built per pattern, on top
//! of the same signing and submission plumbing.
//!
//! UTXO selection is naive: pick the smallest UTXO that covers the fee.
//! The fee is hardcoded and conservative for today's testnet; adjust if
//! mempool churn or hard-fork mass rules change.

use kaspa_addresses::Address;
use kaspa_consensus_core::{
    sign::sign,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        ScriptPublicKey, ScriptVec, SignableTransaction, Transaction, TransactionInput,
        TransactionOutpoint, TransactionOutput, UtxoEntry,
    },
};
use kaspa_rpc_core::{api::rpc::RpcApi, model::tx::RpcTransaction, RpcUtxosByAddressesEntry};
use kaspa_txscript::pay_to_address_script;
use kaspa_wrpc_client::KaspaRpcClient;

use crate::error::{Error, Result};
use crate::wallet::Wallet;

/// Flat fee for carrier transactions, in sompi (0.01 KAS).
///
/// rusty-kaspa v2.0.0 enforces a minimum relay fee of 100 sompi per unit of
/// compute mass (observed on testnet-10: a 2,114-mass payload tx required
/// ≥211,400 sompi). Our payload-carrying transactions stay well under
/// 10,000 compute mass, so a flat 1,000,000 sompi clears the floor with
/// headroom. Revisit if payloads grow or the relay-fee rule changes.
pub const CARRIER_FEE_SOMPI: u64 = 1_000_000;

/// Minimum change output value in sompi (0.01 KAS). Change below this is folded
/// into the fee: zero-value outputs are non-standard ("dust") and small outputs
/// violate the KIP-0009 storage-mass bound (mass ≈ 10¹²/amount must stay ≤ 10⁶,
/// so amount must be ≥ 10⁶ sompi).
pub const MIN_CHANGE_SOMPI: u64 = 1_000_000;

/// Script public key version used for raw script wrapping (post-Toccata v0).
pub const SPK_VERSION: u16 = 0;

/// Wrap raw script bytes in a `ScriptPublicKey`.
pub fn to_spk(script: Vec<u8>) -> ScriptPublicKey {
    ScriptPublicKey::new(SPK_VERSION, ScriptVec::from_slice(&script))
}

/// Pick the smallest UTXO entry whose amount strictly exceeds `min_amount`.
pub fn select_smallest_covering(
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

/// Build, sign (Schnorr), and submit a carrier transaction whose payload is
/// `payload`. Returns the on-chain transaction id as a string.
pub async fn submit_carrier_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    payload: Vec<u8>,
) -> Result<String> {
    let address: Address = wallet.address.clone();
    let entries = client
        .get_utxos_by_addresses(vec![address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses: {e}")))?;

    if entries.is_empty() {
        return Err(Error::Rpc(format!(
            "wallet {address} has no UTXOs on this network — fund it from a testnet faucet"
        )));
    }

    let utxo = select_smallest_covering(entries, CARRIER_FEE_SOMPI).ok_or_else(|| {
        Error::Rpc(format!(
            "wallet {address} has UTXOs but none above the {CARRIER_FEE_SOMPI} sompi fee threshold"
        ))
    })?;

    let outpoint = TransactionOutpoint::new(utxo.outpoint.transaction_id, utxo.outpoint.index);
    let input_amount = utxo.utxo_entry.amount;
    let input_spk = utxo.utxo_entry.script_public_key.clone();
    let block_daa_score = utxo.utxo_entry.block_daa_score;
    let is_coinbase = utxo.utxo_entry.is_coinbase;
    // KIP-20: each UTXO carries an optional covenant id. For an unconstrained
    // pay-to-pubkey UTXO this is None; for covenant-tracked UTXOs it threads
    // forward into the next transaction's spending rules.
    let covenant_id = utxo.utxo_entry.covenant_id;

    let input = TransactionInput::new(outpoint, vec![], 0, 0); // sig + sig_op_count filled by sign()
    let output_value = input_amount - CARRIER_FEE_SOMPI;
    let output = TransactionOutput::new(output_value, pay_to_address_script(&address));

    let tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        payload,
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

/// Current balance for `address`, in sompi.
pub async fn balance_for(client: &KaspaRpcClient, address: &Address) -> Result<u64> {
    let bal = client
        .get_balance_by_address(address.clone())
        .await
        .map_err(|e| Error::Rpc(format!("get_balance_by_address: {e}")))?;
    Ok(bal)
}

#[cfg(test)]
mod tests {
    use super::{
        select_smallest_covering, RpcUtxosByAddressesEntry, ScriptPublicKey, TransactionOutpoint,
    };
    use kaspa_consensus_core::tx::TransactionId;
    use kaspa_rpc_core::model::tx::RpcUtxoEntry;

    fn entry(amount: u64) -> RpcUtxosByAddressesEntry {
        RpcUtxosByAddressesEntry {
            address: None,
            outpoint: TransactionOutpoint::new(TransactionId::from_bytes([0u8; 32]), 0).into(),
            utxo_entry: RpcUtxoEntry::new(amount, ScriptPublicKey::default(), 0, false, None),
        }
    }

    fn amounts(entries: &[u64]) -> Vec<RpcUtxosByAddressesEntry> {
        entries.iter().copied().map(entry).collect()
    }

    #[test]
    fn selects_smallest_utxo_that_covers_fee() {
        let picked = select_smallest_covering(amounts(&[50_000, 9_000, 12_000]), 10_000);
        assert_eq!(picked.map(|e| e.utxo_entry.amount), Some(12_000));
    }

    #[test]
    fn none_when_nothing_covers_fee() {
        let picked = select_smallest_covering(amounts(&[9_000, 10_000]), 10_000);
        assert!(picked.is_none());
    }
}
