//! Pay-to-script-hash (P2SH) covenant spend-path plumbing.
//!
//! This module lets a pattern **lock value under an arbitrary redeem script**
//! and later **spend it by satisfying that script** — the step that turns a
//! pattern from "digest anchored in a payload" into "value encumbered by a
//! covenant that consensus enforces".
//!
//! ## Shape
//!
//! - **Lock:** [`p2sh_lock_script`] wraps a redeem script in the P2SH template
//!   (`OP_BLAKE2B <blake2b256(redeem)> OP_EQUAL`). Pay a normal output to that
//!   `ScriptPublicKey` ([`lock_to_p2sh_tx`]).
//! - **Spend:** [`spend_p2sh_tx`] builds a transaction that spends the P2SH
//!   UTXO. The caller supplies a *satisfier builder* closure that receives the
//!   transaction sighash and returns the stack elements the redeem script
//!   needs (e.g. a Schnorr signature for a `CHECKSIG` redeem). The signature
//!   script is assembled as `<satisfier pushes…> <redeem script>`.
//!
//! ## Safety: offline engine preflight
//!
//! Spending a P2SH output with a wrong sighash or a mis-assembled signature
//! script makes the value unspendable. To prevent that, [`spend_p2sh_tx`]
//! runs the **real rusty-kaspa script engine** over the fully-built spend
//! ([`verify_p2sh_spend_offline`]) and refuses to submit if it does not
//! accept. The engine performs genuine Schnorr verification against the real
//! sighash, so a passing preflight means the spend is cryptographically valid.
//!
//! Status: **v0 — unaudited — testnet first.**

use kaspa_addresses::{Address, Prefix};
use kaspa_consensus_core::{
    hashing::{
        sighash::{calc_schnorr_signature_hash, SigHashReusedValuesUnsync},
        sighash_type::SIG_HASH_ALL,
    },
    mass::SigopCount,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        ComputeCommit, PopulatedTransaction, Transaction, TransactionInput, TransactionOutpoint,
        TransactionOutput, UtxoEntry,
    },
};
use kaspa_rpc_core::{api::rpc::RpcApi, model::tx::RpcTransaction};
use kaspa_txscript::{
    caches::Cache, extract_script_pub_key_address, get_sig_op_count_upper_bound,
    pay_to_address_script, pay_to_script_hash_script, script_builder::ScriptBuilder, EngineCtx,
    EngineFlags, TxScriptEngine,
};
use kaspa_wrpc_client::KaspaRpcClient;

use crate::error::{Error, Result};
use crate::tx::CARRIER_FEE_SOMPI;
use crate::wallet::Wallet;

/// Build the P2SH locking `ScriptPublicKey` for `redeem_script`
/// (`OP_BLAKE2B <blake2b256(redeem)> OP_EQUAL`).
pub fn p2sh_lock_script(redeem_script: &[u8]) -> kaspa_consensus_core::tx::ScriptPublicKey {
    pay_to_script_hash_script(redeem_script)
}

/// BLAKE2b-256 hash of a redeem script — the value the P2SH locking script
/// commits to. Extracted from the canonical P2SH template
/// (`OP_BLAKE2B <push32> <hash> OP_EQUAL`) so it always matches the lock.
pub fn redeem_script_hash(redeem_script: &[u8]) -> [u8; 32] {
    let spk = p2sh_lock_script(redeem_script);
    let bytes = spk.script();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes[2..34]);
    out
}

/// Set the per-input compute commitment with an explicit `sigop_count` for
/// version-0 transactions. Version-1+ transactions use a compute-budget instead.
///
/// The `sigop_count` is the number of signature operations the redeem script
/// will consume, as computed by the node's sigop scanner. It must match what
/// the consensus node will compute from the signature script, because the
/// sighash for version-0 transactions binds this field. Use
/// [`p2sh_redeem_sigop_count`] to compute the correct value from a redeem script.
fn set_input_commitments(tx: &mut Transaction, sigop_count: u8) {
    let commit: ComputeCommit = if ComputeCommit::version_expects_compute_budget_field(tx.version) {
        kaspa_consensus_core::mass::ComputeBudget(10).into()
    } else {
        SigopCount(sigop_count).into()
    };
    for input in tx.inputs.iter_mut() {
        input.compute_commit = commit;
    }
}

/// Compute the sigop count the node will assign to a P2SH input spending
/// `redeem_script`. This is the value to pass to [`set_input_commitments`]
/// so that the sighash matches the node's expectation.
///
/// The count is derived from the redeem script bytes using the same
/// `get_sig_op_count_upper_bound` logic the node applies when accepting the
/// transaction. It is capped at `u8::MAX` (255), which is far above any
/// realistic redeem script.
pub fn p2sh_redeem_sigop_count(redeem_script: &[u8]) -> u8 {
    // Build a minimal dummy signature_script that puts the redeem as its last
    // push, so get_sig_op_count_upper_bound can extract it as the P2SH redeem.
    // Covenant-enabled flags so large covenant redeem scripts assemble (the
    // post-Toccata 1_000_000 cap, not the pre-Toccata 10_000 one).
    let mut dummy_sig_script = ScriptBuilder::with_flags(EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    });
    let _ = dummy_sig_script.add_data(redeem_script);
    let sig_script_bytes = dummy_sig_script.drain().to_vec();
    let p2sh_spk = p2sh_lock_script(redeem_script);
    let count = get_sig_op_count_upper_bound::<
        kaspa_consensus_core::tx::PopulatedTransaction,
        kaspa_consensus_core::hashing::sighash::SigHashReusedValuesUnsync,
    >(&sig_script_bytes, &p2sh_spk);
    count.min(u64::from(u8::MAX)) as u8
}

/// Compute the Schnorr sighash for `input_index` of `tx`, given the UTXO
/// entries being spent (one per input, same order). `tx` must already have its
/// input commitments set via [`set_input_commitments`] (called internally by
/// [`spend_p2sh_tx`] and [`spend_p2sh_tx_with_locktime`]).
///
/// Exposed publicly so callers that build their own spend transactions (e.g.
/// when a non-zero lock_time is needed for CLTV) can compute the sighash over
/// the exact transaction they build without re-running the full
/// [`spend_p2sh_tx`] flow.
pub fn p2sh_input_sighash(tx: &Transaction, entries: &[UtxoEntry], input_index: usize) -> [u8; 32] {
    let populated = PopulatedTransaction::new(tx, entries.to_vec());
    let reused = SigHashReusedValuesUnsync::new();
    let hash = calc_schnorr_signature_hash(&populated, input_index, SIG_HASH_ALL, &reused);
    hash.as_bytes()
}

/// Produce a satisfier signature element for a `CHECKSIG`/`CSFS`-style redeem:
/// the 64-byte Schnorr signature over `sighash` followed by the sighash-type
/// byte (65 bytes). Push this with [`build_p2sh_signature_script`].
///
/// Delegates to [`crate::cryptography::sign_schnorr`] for the signing step so
/// there is a single canonical 65-byte construction.
pub fn schnorr_satisfier_sig(
    sighash: &[u8; 32],
    keypair: &kaspa_bip32::secp256k1::Keypair,
) -> Vec<u8> {
    crate::cryptography::sign_schnorr(sighash, keypair).to_vec()
}

/// Assemble a P2SH signature script: each satisfier element pushed in order,
/// then the redeem script pushed last.
///
/// Built with covenant-enabled flags so that large covenant witnesses (e.g. a
/// KIP-16 tag-0x21 STARK seal, hundreds of KB) clear the post-Toccata canonical
/// script-length and element-size limits (1_000_000) instead of the pre-Toccata
/// 10_000 cap. Small CHECKSIG/CLTV scripts assemble identically either way.
pub fn build_p2sh_signature_script(
    satisfier_elements: &[Vec<u8>],
    redeem_script: &[u8],
) -> Result<Vec<u8>> {
    let mut builder = ScriptBuilder::with_flags(EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    });
    for element in satisfier_elements {
        builder
            .add_data(element)
            .map_err(|e| Error::Rpc(format!("satisfier push: {e}")))?;
    }
    builder
        .add_data(redeem_script)
        .map_err(|e| Error::Rpc(format!("redeem push: {e}")))?;
    Ok(builder.drain().to_vec())
}

/// Run the real script engine over a fully-built P2SH spend input and return
/// `Ok(())` only if the engine accepts it. This is the pre-submit safety gate.
///
/// `covenants_enabled` should match the target network: testnet-10 post-Toccata
/// runs with covenants enabled, but plain `CHECKSIG`/`CLTV`/`CHECKMULTISIG`
/// redeem scripts validate identically either way.
pub fn verify_p2sh_spend_offline(
    tx: &Transaction,
    input_index: usize,
    utxo_entry: &UtxoEntry,
    covenants_enabled: bool,
) -> Result<()> {
    let populated = PopulatedTransaction::new(tx, vec![utxo_entry.clone()]);
    let sig_cache = Cache::new(0);
    let reused = SigHashReusedValuesUnsync::new();
    let ctx = EngineCtx::new(&sig_cache).with_reused(&reused);
    let flags = EngineFlags {
        covenants_enabled,
        ..Default::default()
    };
    let mut vm = TxScriptEngine::from_transaction_input(
        &populated,
        &tx.inputs[input_index],
        input_index,
        utxo_entry,
        ctx,
        flags,
    );
    vm.execute()
        .map_err(|e| Error::Rpc(format!("p2sh preflight rejected: {e}")))
}

/// Run the script engine over a fully-built P2SH spend and return the number of
/// **script units** it consumes. Used to size the input's committed compute
/// budget for expensive covenant opcodes (e.g. `OP_CHECKSIGFROMSTACK`), which
/// are NOT counted as legacy sig-ops by [`p2sh_redeem_sigop_count`] and so need
/// an explicit budget.
pub fn measure_p2sh_script_units(
    tx: &Transaction,
    input_index: usize,
    utxo_entry: &UtxoEntry,
    covenants_enabled: bool,
) -> Result<u64> {
    let populated = PopulatedTransaction::new(tx, vec![utxo_entry.clone()]);
    let sig_cache = Cache::new(0);
    let reused = SigHashReusedValuesUnsync::new();
    let ctx = EngineCtx::new(&sig_cache).with_reused(&reused);
    let flags = EngineFlags {
        covenants_enabled,
        ..Default::default()
    };
    let mut vm = TxScriptEngine::from_transaction_input(
        &populated,
        &tx.inputs[input_index],
        input_index,
        utxo_entry,
        ctx,
        flags,
    );
    vm.execute()
        .map_err(|e| Error::Rpc(format!("p2sh measure rejected: {e}")))?;
    Ok(vm.used_script_units().0)
}

/// Compute the version-0 `SigopCount` an input must commit to cover
/// `used_script_units` of covenant-opcode execution.
///
/// Each `SigopCount` unit commits 100,000 script units; the first ~9,999 units
/// per input are free. One extra unit of margin is added. Saturates at
/// [`u8::MAX`].
pub fn covering_sigop_count(used_script_units: u64) -> u8 {
    const SCRIPT_UNITS_PER_SIGOP: u64 = 100_000;
    const FREE_UNITS: u64 = 9_999;
    let charged = used_script_units.saturating_sub(FREE_UNITS);
    let units = charged.div_ceil(SCRIPT_UNITS_PER_SIGOP) + 1; // +1 margin
    units.min(u64::from(u8::MAX)) as u8
}

/// Lock `value_sompi` under `redeem_script` (P2SH). Builds a 1-input/2-output
/// transaction from `wallet`: output 0 is the P2SH-locked value, output 1 is
/// change (omitted when below the dust-safe minimum). Returns the tx id.
pub async fn lock_to_p2sh_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    redeem_script: &[u8],
    value_sompi: u64,
) -> Result<String> {
    use kaspa_consensus_core::{sign::sign, tx::SignableTransaction};

    let required = value_sompi
        .checked_add(CARRIER_FEE_SOMPI)
        .ok_or_else(|| Error::Rpc(format!("lock value {value_sompi} + fee overflows u64")))?;
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
    let mut candidates: Vec<_> = entries
        .into_iter()
        .filter(|e| e.utxo_entry.amount > required)
        .collect();
    candidates.sort_by_key(|e| e.utxo_entry.amount);
    let utxo = candidates.into_iter().next().ok_or_else(|| {
        Error::Rpc(format!(
            "wallet {} has no UTXO covering {required} sompi",
            wallet.address
        ))
    })?;

    let outpoint = TransactionOutpoint::new(utxo.outpoint.transaction_id, utxo.outpoint.index);
    let input_amount = utxo.utxo_entry.amount;
    let input_spk = utxo.utxo_entry.script_public_key.clone();
    let daa = utxo.utxo_entry.block_daa_score;
    let is_coinbase = utxo.utxo_entry.is_coinbase;
    let covenant_id = utxo.utxo_entry.covenant_id;

    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let lock_output = TransactionOutput::new(value_sompi, p2sh_lock_script(redeem_script));
    let change = input_amount.checked_sub(required).ok_or_else(|| {
        Error::Rpc(format!(
            "selected UTXO {input_amount} does not cover value+fee {required}"
        ))
    })?;
    let mut outputs = vec![lock_output];
    if change >= crate::tx::MIN_CHANGE_SOMPI {
        outputs.push(TransactionOutput::new(
            change,
            pay_to_address_script(&wallet.address),
        ));
    }
    let tx = Transaction::new(0, vec![input], outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);
    let entry = UtxoEntry::new(input_amount, input_spk, daa, is_coinbase, covenant_id);
    let signable = SignableTransaction::with_entries(tx, vec![entry]);
    let signed = sign(signable, wallet.keypair);
    let rpc_tx: RpcTransaction = (&signed.tx).into();
    let tx_id = client
        .submit_transaction(rpc_tx, false)
        .await
        .map_err(|e| Error::Rpc(format!("submit lock: {e}")))?;
    Ok(tx_id.to_string())
}

/// Spend a P2SH UTXO by satisfying `redeem_script`.
///
/// `satisfier_builder` receives the transaction sighash and returns the stack
/// elements the redeem script consumes (in push order, redeem script appended
/// automatically). The spend is engine-checked before submission.
///
/// The P2SH UTXO is located by deriving its address from `redeem_script` and
/// matching `p2sh_outpoint` among that address's UTXOs (so the caller needs
/// only the outpoint, not the amount/DAA score).
// Each parameter is a distinct, irreducible input to a covenant spend
// (node, script, outpoint, destination, network prefix, fee, engine flag,
// satisfier); bundling them would obscure rather than clarify.
#[allow(clippy::too_many_arguments)]
pub async fn spend_p2sh_tx<F>(
    client: &KaspaRpcClient,
    redeem_script: &[u8],
    p2sh_outpoint: (kaspa_consensus_core::tx::TransactionId, u32),
    dest_address: &Address,
    prefix: Prefix,
    fee_sompi: u64,
    covenants_enabled: bool,
    satisfier_builder: F,
) -> Result<String>
where
    F: FnOnce(&[u8; 32]) -> Result<Vec<Vec<u8>>>,
{
    let p2sh_spk = p2sh_lock_script(redeem_script);
    let p2sh_address = extract_script_pub_key_address(&p2sh_spk, prefix)
        .map_err(|e| Error::Rpc(format!("p2sh address: {e}")))?;
    let entries = client
        .get_utxos_by_addresses(vec![p2sh_address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses(p2sh): {e}")))?;
    let (target_txid, target_index) = p2sh_outpoint;
    let utxo = entries
        .into_iter()
        .find(|e| e.outpoint.transaction_id == target_txid && e.outpoint.index == target_index)
        .ok_or_else(|| {
            Error::Rpc(format!(
                "p2sh outpoint {target_txid}:{target_index} not found at {p2sh_address}"
            ))
        })?;
    let amount = utxo.utxo_entry.amount;
    let daa = utxo.utxo_entry.block_daa_score;
    let covenant_id = utxo.utxo_entry.covenant_id;

    let outpoint = TransactionOutpoint::new(target_txid, target_index);
    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let spend_value = amount.checked_sub(fee_sompi).ok_or_else(|| {
        Error::Rpc(format!(
            "fee {fee_sompi} exceeds the P2SH UTXO amount {amount}"
        ))
    })?;
    let output = TransactionOutput::new(spend_value, pay_to_address_script(dest_address));
    let mut tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let sigops = p2sh_redeem_sigop_count(redeem_script);
    set_input_commitments(&mut tx, sigops);

    // The input UTXO entry carries the P2SH locking script — this is what the
    // sighash binds and what the engine checks the redeem hash against.
    let input_entry = UtxoEntry::new(amount, p2sh_spk, daa, false, covenant_id);

    let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&input_entry), 0);
    let satisfier = satisfier_builder(&sighash)?;
    tx.inputs[0].signature_script = build_p2sh_signature_script(&satisfier, redeem_script)?;

    // Pre-submit gate: run the real engine over the assembled spend.
    verify_p2sh_spend_offline(&tx, 0, &input_entry, covenants_enabled)?;

    let rpc_tx: RpcTransaction = (&tx).into();
    let tx_id = client
        .submit_transaction(rpc_tx, false)
        .await
        .map_err(|e| Error::Rpc(format!("submit p2sh spend: {e}")))?;
    Ok(tx_id.to_string())
}

/// Spend a P2SH UTXO, committing an explicit `sigop_count` for the input.
///
/// Identical to [`spend_p2sh_tx`] except the input's version-0 compute budget is
/// committed from the caller-supplied `sigop_count` (each unit covers 100_000
/// script units) instead of the legacy-sigop estimate from
/// [`p2sh_redeem_sigop_count`].
///
/// This is required for redeem scripts whose cost comes from **covenant
/// opcodes** (e.g. `OpZkPrecompile`, a KIP-16 tag-0x21 STARK verification at
/// ~25M script units). Those are not counted as legacy sigops, so the default
/// estimate leaves only the free ~9_999 units and the node rejects the spend
/// with "script units exceeded the amount committed". Pass `255` (the u8 max,
/// committing ~25.5M units) for tag-0x21 verification —
/// `kcp_pq_anchor::sigop::sigop_count_for_pq_verify()`.
///
/// `skip_preflight = false` runs the real engine over the assembled spend before
/// submission (the safety gate; recommended). Pass `true` ONLY to deliberately
/// submit a spend the local engine would reject, in order to observe the LIVE
/// NODE's rejection (e.g. an on-chain negative control proving consensus
/// enforcement). A `true` preflight-skipped spend with an invalid witness will
/// be rejected by the node, returning the node's error string.
// Each parameter is a distinct, irreducible input to a covenant spend; bundling
// them would obscure rather than clarify.
#[allow(clippy::too_many_arguments)]
pub async fn spend_p2sh_tx_with_sigops<F>(
    client: &KaspaRpcClient,
    redeem_script: &[u8],
    p2sh_outpoint: (kaspa_consensus_core::tx::TransactionId, u32),
    dest_address: &Address,
    prefix: Prefix,
    fee_sompi: u64,
    covenants_enabled: bool,
    sigop_count: u8,
    skip_preflight: bool,
    satisfier_builder: F,
) -> Result<String>
where
    F: FnOnce(&[u8; 32]) -> Result<Vec<Vec<u8>>>,
{
    let p2sh_spk = p2sh_lock_script(redeem_script);
    let p2sh_address = extract_script_pub_key_address(&p2sh_spk, prefix)
        .map_err(|e| Error::Rpc(format!("p2sh address: {e}")))?;
    let entries = client
        .get_utxos_by_addresses(vec![p2sh_address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses(p2sh): {e}")))?;
    let (target_txid, target_index) = p2sh_outpoint;
    let utxo = entries
        .into_iter()
        .find(|e| e.outpoint.transaction_id == target_txid && e.outpoint.index == target_index)
        .ok_or_else(|| {
            Error::Rpc(format!(
                "p2sh outpoint {target_txid}:{target_index} not found at {p2sh_address}"
            ))
        })?;
    let amount = utxo.utxo_entry.amount;
    let daa = utxo.utxo_entry.block_daa_score;
    let covenant_id = utxo.utxo_entry.covenant_id;

    let outpoint = TransactionOutpoint::new(target_txid, target_index);
    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let spend_value = amount.checked_sub(fee_sompi).ok_or_else(|| {
        Error::Rpc(format!(
            "fee {fee_sompi} exceeds the P2SH UTXO amount {amount}"
        ))
    })?;
    let output = TransactionOutput::new(spend_value, pay_to_address_script(dest_address));
    let mut tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    set_input_commitments(&mut tx, sigop_count);

    let input_entry = UtxoEntry::new(amount, p2sh_spk, daa, false, covenant_id);

    let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&input_entry), 0);
    let satisfier = satisfier_builder(&sighash)?;
    tx.inputs[0].signature_script = build_p2sh_signature_script(&satisfier, redeem_script)?;

    if !skip_preflight {
        verify_p2sh_spend_offline(&tx, 0, &input_entry, covenants_enabled)?;
    }

    let rpc_tx: RpcTransaction = (&tx).into();
    let tx_id = client
        .submit_transaction(rpc_tx, false)
        .await
        .map_err(|e| Error::Rpc(format!("submit p2sh spend: {e}")))?;
    Ok(tx_id.to_string())
}

/// Spend a P2SH UTXO with an explicit `lock_time` and `sequence` on the input.
///
/// This is a thin generalization of [`spend_p2sh_tx`]: the only difference is
/// that the transaction's `lock_time` field and the input `sequence` field are
/// caller-controlled instead of defaulting to zero.
///
/// ## When to use this
///
/// `OP_CHECKLOCKTIMEVERIFY` (CLTV) requires:
/// 1. The transaction's `lock_time` field ≥ the deadline encoded in the script.
/// 2. The input's `sequence` must NOT be `0xffffffffffffffff`
///    (`MAX_TX_IN_SEQUENCE_NUM`), otherwise the engine treats the input as
///    finalised and rejects the opcode.
///
/// For a height-based deadline: `lock_time` must be `< LOCK_TIME_THRESHOLD`
/// (500_000_000_000) and equal to or greater than the deadline.
/// For a unix-seconds deadline: `lock_time` must be `>= LOCK_TIME_THRESHOLD`
/// and equal to or greater than the deadline.
///
/// The [`spend_p2sh_tx`] function is a wrapper that calls this with
/// `lock_time = 0` and `sequence = 0`, which is correct for `CHECKSIG`/
/// `CHECKMULTISIG` redeem scripts that carry no lock-time constraint.
// Each parameter is a distinct, irreducible input to a covenant spend;
// bundling them would obscure rather than clarify.
#[allow(clippy::too_many_arguments)]
pub async fn spend_p2sh_tx_with_locktime<F>(
    client: &KaspaRpcClient,
    redeem_script: &[u8],
    p2sh_outpoint: (kaspa_consensus_core::tx::TransactionId, u32),
    dest_address: &Address,
    prefix: Prefix,
    fee_sompi: u64,
    covenants_enabled: bool,
    lock_time: u64,
    sequence: u64,
    satisfier_builder: F,
) -> Result<String>
where
    F: FnOnce(&[u8; 32]) -> Result<Vec<Vec<u8>>>,
{
    let p2sh_spk = p2sh_lock_script(redeem_script);
    let p2sh_address = extract_script_pub_key_address(&p2sh_spk, prefix)
        .map_err(|e| Error::Rpc(format!("p2sh address: {e}")))?;
    let entries = client
        .get_utxos_by_addresses(vec![p2sh_address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses(p2sh): {e}")))?;
    let (target_txid, target_index) = p2sh_outpoint;
    let utxo = entries
        .into_iter()
        .find(|e| e.outpoint.transaction_id == target_txid && e.outpoint.index == target_index)
        .ok_or_else(|| {
            Error::Rpc(format!(
                "p2sh outpoint {target_txid}:{target_index} not found at {p2sh_address}"
            ))
        })?;
    let amount = utxo.utxo_entry.amount;
    let daa = utxo.utxo_entry.block_daa_score;
    let covenant_id = utxo.utxo_entry.covenant_id;

    let outpoint = TransactionOutpoint::new(target_txid, target_index);
    let input = TransactionInput::new(outpoint, vec![], sequence, 0);
    let spend_value = amount.checked_sub(fee_sompi).ok_or_else(|| {
        Error::Rpc(format!(
            "fee {fee_sompi} exceeds the P2SH UTXO amount {amount}"
        ))
    })?;
    let output = TransactionOutput::new(spend_value, pay_to_address_script(dest_address));
    let mut tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        lock_time,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let sigops = p2sh_redeem_sigop_count(redeem_script);
    set_input_commitments(&mut tx, sigops);

    let input_entry = UtxoEntry::new(amount, p2sh_spk, daa, false, covenant_id);

    let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&input_entry), 0);
    let satisfier = satisfier_builder(&sighash)?;
    tx.inputs[0].signature_script = build_p2sh_signature_script(&satisfier, redeem_script)?;

    verify_p2sh_spend_offline(&tx, 0, &input_entry, covenants_enabled)?;

    let rpc_tx: RpcTransaction = (&tx).into();
    let tx_id = client
        .submit_transaction(rpc_tx, false)
        .await
        .map_err(|e| Error::Rpc(format!("submit p2sh spend: {e}")))?;
    Ok(tx_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_bip32::secp256k1::{Keypair, SECP256K1};
    use kaspa_consensus_core::tx::{ScriptVec, TransactionId};
    use kaspa_txscript::opcodes::codes::OpCheckSig;
    use kaspa_txscript::script_builder::ScriptBuilder as SB;

    fn test_keypair(byte: u8) -> Keypair {
        Keypair::from_seckey_slice(SECP256K1, &[byte; 32]).unwrap()
    }

    /// Redeem script `<x-only pubkey> OP_CHECKSIG` (a P2SH-wrapped single-sig).
    fn checksig_redeem(kp: &Keypair) -> Vec<u8> {
        let xonly = kp.x_only_public_key().0.serialize();
        SB::new()
            .add_data(&xonly)
            .unwrap()
            .add_op(OpCheckSig)
            .unwrap()
            .drain()
            .to_vec()
    }

    /// Build a spend tx for a synthetic P2SH UTXO and return (tx, input_entry).
    fn build_spend(redeem: &[u8], amount: u64) -> (Transaction, UtxoEntry) {
        let p2sh_spk = p2sh_lock_script(redeem);
        let prev = TransactionOutpoint::new(TransactionId::from_slice(&[7u8; 32]), 0);
        let dest = ScriptPublicKeyHelper::pay_to_self();
        let input = TransactionInput::new(prev, vec![], 0, 0);
        let output = TransactionOutput::new(amount - 1_000_000, dest);
        let mut tx = Transaction::new(
            0,
            vec![input],
            vec![output],
            0,
            SUBNETWORK_ID_NATIVE,
            0,
            vec![],
        );
        let sigops = p2sh_redeem_sigop_count(redeem);
        set_input_commitments(&mut tx, sigops);
        let entry = UtxoEntry::new(amount, p2sh_spk, 0, false, None);
        (tx, entry)
    }

    // Minimal helper to make an arbitrary destination spk for tests.
    struct ScriptPublicKeyHelper;
    impl ScriptPublicKeyHelper {
        fn pay_to_self() -> kaspa_consensus_core::tx::ScriptPublicKey {
            kaspa_consensus_core::tx::ScriptPublicKey::new(0, ScriptVec::from_slice(&[0x51]))
            // OP_TRUE
        }
    }

    #[test]
    fn p2sh_lock_script_commits_blake2b_of_redeem() {
        let kp = test_keypair(0x11);
        let redeem = checksig_redeem(&kp);
        let spk = p2sh_lock_script(&redeem);
        // Template: OP_BLAKE2B(0xaa) OP_DATA32(0x20) <32 bytes> OP_EQUAL(0x87)
        let bytes = spk.script();
        assert_eq!(bytes[0], 0xaa, "OP_BLAKE2B");
        assert_eq!(bytes[1], 0x20, "push-32");
        assert_eq!(bytes[34], 0x87, "OP_EQUAL");
        assert_eq!(bytes.len(), 35);
    }

    #[test]
    fn p2sh_single_sig_lock_spend_executes_on_engine() {
        let kp = test_keypair(0x11);
        let redeem = checksig_redeem(&kp);
        let amount = 100_000_000u64;
        let (mut tx, entry) = build_spend(&redeem, amount);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &kp);
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig], &redeem).unwrap();

        // The real engine must accept a correctly-signed P2SH single-sig spend.
        verify_p2sh_spend_offline(&tx, 0, &entry, false).expect("engine should accept valid spend");
    }

    #[test]
    fn p2sh_wrong_sig_rejects_on_engine() {
        let kp = test_keypair(0x11);
        let wrong = test_keypair(0x22);
        let redeem = checksig_redeem(&kp);
        let amount = 100_000_000u64;
        let (mut tx, entry) = build_spend(&redeem, amount);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &wrong); // signed by the wrong key
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject a spend signed by the wrong key"
        );
    }
}
