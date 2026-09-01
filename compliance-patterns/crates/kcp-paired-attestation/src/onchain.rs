//! Full two-party on-chain datasig enforcement via `OP_CHECKSIGFROMSTACK`.
//!
//! This module upgrades kcp-paired-attestation from the v0 off-chain-mating
//! scope-down to **consensus-enforced two-party attestation**: value is locked
//! under a P2SH redeem script that requires valid Schnorr data-signatures from
//! **both** oracle keys over a shared 32-byte attestation commitment hash
//! (`msg_hash`). Neither party can release the locked value without the other's
//! independent signature.
//!
//! ## Design
//!
//! The redeem script embeds `pkA` and `pkB` (x-only 32-byte public keys) and a
//! shared `msgHash` (the 32-byte attestation commitment), then requires two
//! independent Schnorr data-signatures at spend time:
//!
//! ```text
//! Redeem script:
//!   OP_TOALTSTACK           ; move sigB (top) to alt stack
//!   <msgHash> <pkA>
//!   OP_CHECKSIGFROMSTACK    ; pops [sigA, msgHash, pkA] — verifies sigA over msgHash with pkA
//!   OP_VERIFY               ; must be true
//!   OP_FROMALTSTACK         ; bring sigB back
//!   <msgHash> <pkB>
//!   OP_CHECKSIGFROMSTACK    ; pops [sigB, msgHash, pkB] — verifies sigB over msgHash with pkB
//!                           ; result left on stack — engine checks truthiness
//!
//! Satisfier (pushed before redeem in sig_script):
//!   <sigA> <sigB>           ; sigA deepest, sigB on top
//! ```
//!
//! After the P2SH peel (the engine re-runs the redeem script with the
//! satisfier elements on the stack):
//! 1. Stack after satisfier pushes: `[sigA (bottom), sigB (top)]`
//! 2. `OP_TOALTSTACK`: sigB moves to alt; main stack `[sigA]`
//! 3. Push `msgHash`, push `pkA`: main stack `[sigA, msgHash, pkA]`
//! 4. `OP_CHECKSIGFROMSTACK`: pops `pkA` (top), `msgHash`, `sigA`; pushes `bool`
//! 5. `OP_VERIFY`: asserts `bool`; main stack `[]`
//! 6. `OP_FROMALTSTACK`: main stack `[sigB]`
//! 7. Push `msgHash`, push `pkB`: main stack `[sigB, msgHash, pkB]`
//! 8. `OP_CHECKSIGFROMSTACK`: pops `pkB`, `msgHash`, `sigB`; pushes `bool`
//! 9. Engine checks final stack: `[bool]` — must be truthy.
//!
//! ## Signature format
//!
//! `OP_CHECKSIGFROMSTACK` expects a **64-byte raw Schnorr signature** (NOT
//! the 65-byte sighash-type-appended form used by `OP_CHECKSIG`). The
//! `datasig` function in this module produces the correct 64-byte form.
//! Confirmed by the CSFS positive control (FACTS SS-024-v3/v4).
//!
//! ## covenants_enabled
//!
//! `OP_CHECKSIGFROMSTACK` (0xd7) is gated on `covenants_enabled`. The offline
//! engine preflight `kcp_common::p2sh::verify_p2sh_spend_offline` accepts a
//! `covenants_enabled` flag; we always pass `true` for CSFS scripts.
//!
//! The `EngineCtx` already defaults to `EMPTY_COV_CONTEXT`, which suffices for
//! CSFS — the opcode only reads `vm.flags.covenants_enabled` and the sig cache;
//! it does not call into `vm.covenants_ctx`. No additional wiring is required.
//! This was confirmed by inspection of
//! `rusty-kaspa@v2.0.0 crypto/txscript/src/opcodes/mod.rs:1634-1643`.
//!
//! ## Status
//!
//! **v1 — unaudited — testnet first.** Proven by offline engine preflight
//! (see `#[cfg(test)]` below). The full on-chain version is consensus-enforced
//! via direct CSFS opcodes, not the silverscript compiler. FACTS SS-024-v4
//! (CSFS primitive proven on v2.0.0) and the offline tests in this module are
//! the evidentiary basis.

#![forbid(unsafe_code)]

use kaspa_addresses::{Address, Prefix};
use kaspa_bip32::secp256k1::{Keypair, Message};
use kaspa_consensus_core::tx::TransactionId;
use kaspa_txscript::{
    opcodes::codes::{OpCheckSigFromStack, OpFromAltStack, OpToAltStack, OpVerify},
    script_builder::ScriptBuilder,
};
use kaspa_wrpc_client::KaspaRpcClient;

use kcp_common::{
    p2sh::{
        build_p2sh_signature_script, covering_sigop_count, lock_to_p2sh_tx,
        measure_p2sh_script_units, p2sh_lock_script, p2sh_redeem_sigop_count,
        verify_p2sh_spend_offline,
    },
    wallet::Wallet,
};

use kaspa_consensus_core::{
    mass::SigopCount,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        ComputeCommit, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput,
        UtxoEntry,
    },
};
use kaspa_rpc_core::{api::rpc::RpcApi, model::tx::RpcTransaction};
use kaspa_txscript::pay_to_address_script;

use crate::error::{Error, Result};

// ── Redeem script ─────────────────────────────────────────────────────────────

/// Build the two-datasig CSFS redeem script for the paired-attestation covenant.
///
/// The script embeds both oracle x-only public keys (`pk_a`, `pk_b`) and the
/// shared 32-byte attestation `msg_hash`. At spend time the spender must supply
/// valid Schnorr data-signatures from **both** keys over `msg_hash` (see
/// [`datasig`] and [`spend_attestation_vault`]).
///
/// ## Redeem script layout
///
/// ```text
/// OP_TOALTSTACK
/// <msg_hash> <pk_a> OP_CHECKSIGFROMSTACK OP_VERIFY
/// OP_FROMALTSTACK
/// <msg_hash> <pk_b> OP_CHECKSIGFROMSTACK
/// ```
///
/// Satisfier (sig_script before the redeem push):
/// ```text
/// <sig_a> <sig_b>   (sig_a deepest, sig_b on top)
/// ```
///
/// ## Opcode semantics (CSFS)
///
/// `OP_CHECKSIGFROMSTACK` pops `[signature, msg_hash, pubkey]` (signature
/// deepest, pubkey on top), requires a 32-byte `msg_hash`, and performs real
/// Schnorr verification. It is gated on `covenants_enabled`. Proven on
/// rusty-kaspa v2.0.0 (FACTS SS-024-v3/v4).
///
/// ## Errors
///
/// Returns [`Error::Rpc`] if the script builder rejects any push (should not
/// happen for valid 32-byte keys/hash).
pub fn two_datasig_redeem_script(
    pk_a: &[u8; 32],
    pk_b: &[u8; 32],
    msg_hash: &[u8; 32],
) -> Result<Vec<u8>> {
    // Satisfier will push: sigA (deepest), sigB (top)
    // Redeem choreography:
    //   OP_TOALTSTACK          → moves sigB to alt; main: [sigA]
    //   <msgHash>              → main: [sigA, msgHash]
    //   <pkA>                  → main: [sigA, msgHash, pkA]
    //   OP_CHECKSIGFROMSTACK   → pops [sigA, msgHash, pkA]; pushes bool; main: [bool_a]
    //   OP_VERIFY              → asserts bool_a; main: []
    //   OP_FROMALTSTACK        → main: [sigB]
    //   <msgHash>              → main: [sigB, msgHash]
    //   <pkB>                  → main: [sigB, msgHash, pkB]
    //   OP_CHECKSIGFROMSTACK   → pops [sigB, msgHash, pkB]; pushes bool_b; main: [bool_b]
    //   (engine checks final stack: [bool_b] must be truthy)
    let script = ScriptBuilder::new()
        .add_op(OpToAltStack)
        .map_err(|e| Error::Rpc(format!("add OpToAltStack: {e}")))?
        .add_data(msg_hash)
        .map_err(|e| Error::Rpc(format!("add msg_hash (A): {e}")))?
        .add_data(pk_a)
        .map_err(|e| Error::Rpc(format!("add pk_a: {e}")))?
        .add_op(OpCheckSigFromStack)
        .map_err(|e| Error::Rpc(format!("add OpCheckSigFromStack (A): {e}")))?
        .add_op(OpVerify)
        .map_err(|e| Error::Rpc(format!("add OpVerify: {e}")))?
        .add_op(OpFromAltStack)
        .map_err(|e| Error::Rpc(format!("add OpFromAltStack: {e}")))?
        .add_data(msg_hash)
        .map_err(|e| Error::Rpc(format!("add msg_hash (B): {e}")))?
        .add_data(pk_b)
        .map_err(|e| Error::Rpc(format!("add pk_b: {e}")))?
        .add_op(OpCheckSigFromStack)
        .map_err(|e| Error::Rpc(format!("add OpCheckSigFromStack (B): {e}")))?
        .drain()
        .to_vec();
    Ok(script)
}

// ── Data-signature ────────────────────────────────────────────────────────────

/// Produce a 64-byte raw Schnorr data-signature over a 32-byte `msg_hash`.
///
/// `OP_CHECKSIGFROMSTACK` expects a **64-byte** raw Schnorr signature (not the
/// 65-byte sighash-type form used by `OP_CHECKSIG`). This function produces the
/// correct format. Both oracles call this independently over the same `msg_hash`
/// with their respective key-pairs.
///
/// ## Not a transaction sighash
///
/// The `msg_hash` is an arbitrary 32-byte attestation commitment, independent
/// of any spending transaction. The two oracles can pre-sign the attestation
/// off-band; the covenant enforces both signatures on-chain at spend time.
/// This is in contrast to `OP_CHECKSIG` / `schnorr_satisfier_sig`, which sign
/// the transaction sighash.
pub fn datasig(msg_hash: &[u8; 32], keypair: &Keypair) -> Vec<u8> {
    let message = Message::from_digest_slice(msg_hash).expect("32-byte msg_hash is always valid");
    let sig: [u8; 64] = *keypair.sign_schnorr(message).as_ref();
    sig.to_vec()
}

// ── Lock ──────────────────────────────────────────────────────────────────────

/// Lock `value_sompi` under the two-datasig CSFS P2SH covenant.
///
/// Builds the redeem script from `pk_a`, `pk_b`, and `msg_hash`, wraps it in
/// a P2SH locking script, and funds the output from `wallet`. Returns the
/// submitted transaction id.
///
/// The locked value can only be released by providing valid Schnorr
/// data-signatures from **both** `pk_a` and `pk_b` over the same `msg_hash`
/// (see [`spend_attestation_vault`]). This is enforced by the Kaspa consensus
/// engine via `OP_CHECKSIGFROMSTACK`.
///
/// ## Errors
///
/// Returns [`Error::Rpc`] on node failure or if `wallet` has no suitable UTXO.
// Parameters are each irreducible inputs to a two-datasig covenant lock.
#[allow(clippy::too_many_arguments)]
pub async fn lock_attestation_vault(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    pk_a: &[u8; 32],
    pk_b: &[u8; 32],
    msg_hash: &[u8; 32],
    value_sompi: u64,
) -> Result<String> {
    let redeem = two_datasig_redeem_script(pk_a, pk_b, msg_hash)?;
    lock_to_p2sh_tx(client, wallet, &redeem, value_sompi)
        .await
        .map_err(|e| Error::Rpc(format!("lock_attestation_vault: {e}")))
}

// ── Spend ─────────────────────────────────────────────────────────────────────

/// Spend a two-datasig attestation vault UTXO by providing both data-signatures.
///
/// The vault must have been locked with [`lock_attestation_vault`] using the
/// same `pk_a`, `pk_b`, and `msg_hash`. `sig_a` and `sig_b` must each be a
/// 64-byte raw Schnorr data-signature (produced by [`datasig`]) over `msg_hash`
/// with the corresponding oracle key-pair.
///
/// The spend is **engine-preflighted** with `covenants_enabled = true` via
/// [`kcp_common::p2sh::verify_p2sh_spend_offline`] before any RPC submission.
/// If the engine rejects the spend (wrong signature, wrong key, tampered
/// `msg_hash`), the function returns an error without submitting.
///
/// ## Satisfier layout
///
/// The signature script pushes `[sig_a, sig_b]` before the redeem script:
/// `sig_a` is deepest (pushed first), `sig_b` is on top (pushed second).
/// This is the order the redeem script expects:
/// - `OP_TOALTSTACK` moves `sig_b` to the alt stack.
/// - `OP_CHECKSIGFROMSTACK` pops `[sig_a, msgHash, pkA]` from the main stack.
/// - `OP_FROMALTSTACK` brings `sig_b` back, then `OP_CHECKSIGFROMSTACK` pops `[sig_b, msgHash, pkB]`.
///
/// ## Errors
///
/// Returns [`Error::Rpc`] if the engine preflight rejects the spend, if the
/// vault UTXO is not found at `vault_outpoint`, or on RPC failure.
// Parameters are each irreducible inputs to a two-datasig covenant spend.
#[allow(clippy::too_many_arguments)]
pub async fn spend_attestation_vault(
    client: &KaspaRpcClient,
    pk_a: &[u8; 32],
    pk_b: &[u8; 32],
    msg_hash: &[u8; 32],
    vault_outpoint: (TransactionId, u32),
    sig_a: Vec<u8>,
    sig_b: Vec<u8>,
    dest: &Address,
    prefix: Prefix,
    fee_sompi: u64,
) -> Result<String> {
    let redeem = two_datasig_redeem_script(pk_a, pk_b, msg_hash)?;

    // Locate the vault UTXO via its P2SH address.
    let p2sh_spk = p2sh_lock_script(&redeem);
    let p2sh_address = kaspa_txscript::extract_script_pub_key_address(&p2sh_spk, prefix)
        .map_err(|e| Error::Rpc(format!("p2sh address: {e}")))?;
    let entries = client
        .get_utxos_by_addresses(vec![p2sh_address.clone()])
        .await
        .map_err(|e| Error::Rpc(format!("get_utxos_by_addresses(p2sh): {e}")))?;

    let (target_txid, target_index) = vault_outpoint;
    let utxo = entries
        .into_iter()
        .find(|e| e.outpoint.transaction_id == target_txid && e.outpoint.index == target_index)
        .ok_or_else(|| {
            Error::Rpc(format!(
                "vault outpoint {target_txid}:{target_index} not found at {p2sh_address}"
            ))
        })?;

    let amount = utxo.utxo_entry.amount;
    let daa = utxo.utxo_entry.block_daa_score;
    let covenant_id = utxo.utxo_entry.covenant_id;

    // Build the spend transaction.
    let outpoint = TransactionOutpoint::new(target_txid, target_index);
    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let output = TransactionOutput::new(amount - fee_sompi, pay_to_address_script(dest));
    let mut tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );

    let input_entry = UtxoEntry::new(amount, p2sh_spk, daa, false, covenant_id);

    // Assemble satisfier: [sig_a (deepest), sig_b (top)] then redeem.
    // CSFS does NOT use the tx sighash — each sig is over msg_hash directly —
    // so the input's compute commitment can be set without invalidating the
    // signatures (unlike a CHECKSIG spend).
    let satisfier = vec![sig_a, sig_b];
    tx.inputs[0].signature_script = build_p2sh_signature_script(&satisfier, &redeem)
        .map_err(|e| Error::Rpc(format!("build sig_script: {e}")))?;

    // CSFS is a covenant opcode: it is NOT counted as a legacy sig-op, so the
    // node grants only the free per-input script-unit allowance unless the
    // input commits a compute budget. Measure the actual covenant cost via the
    // engine and commit a SigopCount that covers it. Provisionally set 0 so the
    // measurement runs, then commit the covering value.
    let commit0: ComputeCommit = SigopCount(p2sh_redeem_sigop_count(&redeem)).into();
    for inp in tx.inputs.iter_mut() {
        inp.compute_commit = commit0;
    }
    let used_units = measure_p2sh_script_units(&tx, 0, &input_entry, true)
        .map_err(|e| Error::Rpc(format!("measure script units: {e}")))?;
    let commit: ComputeCommit = SigopCount(covering_sigop_count(used_units)).into();
    for inp in tx.inputs.iter_mut() {
        inp.compute_commit = commit;
    }

    // Engine preflight with covenants_enabled=true (required for CSFS).
    // verify_p2sh_spend_offline uses EngineCtx::new().with_reused() which
    // defaults to EMPTY_COV_CONTEXT — sufficient for CSFS which only checks
    // vm.flags.covenants_enabled and performs schnorr verification.
    verify_p2sh_spend_offline(&tx, 0, &input_entry, true)
        .map_err(|e| Error::Rpc(format!("engine preflight rejected: {e}")))?;

    // Submit.
    let rpc_tx: RpcTransaction = (&tx).into();
    let tx_id = client
        .submit_transaction(rpc_tx, false)
        .await
        .map_err(|e| Error::Rpc(format!("submit spend: {e}")))?;
    Ok(tx_id.to_string())
}

// ── Offline engine tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_bip32::secp256k1::{Keypair, SECP256K1};
    use kaspa_consensus_core::{
        subnets::SUBNETWORK_ID_NATIVE,
        tx::{
            ScriptPublicKey, ScriptVec, Transaction, TransactionId, TransactionInput,
            TransactionOutpoint, TransactionOutput, UtxoEntry,
        },
    };
    use kcp_common::tx::CARRIER_FEE_SOMPI;

    /// Build a test keypair from a single repeated byte (deterministic, NOT secret).
    fn test_keypair(byte: u8) -> Keypair {
        Keypair::from_seckey_slice(SECP256K1, &[byte; 32]).unwrap()
    }

    /// A minimal destination `ScriptPublicKey` for test outputs (`OP_TRUE`).
    fn op_true_spk() -> ScriptPublicKey {
        ScriptPublicKey::new(0, ScriptVec::from_slice(&[0x51]))
    }

    /// Build a synthetic P2SH spend transaction for the two-datasig redeem.
    ///
    /// Returns `(tx, input_utxo_entry)` with input commitments set.
    /// The `sig_script` is intentionally left empty; callers set it after
    /// computing the satisfier elements.
    fn build_spend_tx(redeem: &[u8], amount: u64) -> (Transaction, UtxoEntry) {
        let p2sh_spk = p2sh_lock_script(redeem);
        let prev = TransactionOutpoint::new(TransactionId::from_slice(&[0xabu8; 32]), 0);
        let input = TransactionInput::new(prev, vec![], 0, 0);
        let output = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, op_true_spk());
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
        let commit: ComputeCommit = SigopCount(sigops).into();
        for inp in tx.inputs.iter_mut() {
            inp.compute_commit = commit;
        }
        let entry = UtxoEntry::new(amount, p2sh_spk, 0, false, None);
        (tx, entry)
    }

    // ── Valid case ─────────────────────────────────────────────────────────────

    /// Both valid data-signatures are accepted by the real engine with
    /// `covenants_enabled = true`.
    ///
    /// This is the key deliverable: the engine enforces that BOTH pkA and pkB
    /// signed the same `msg_hash`, not just any signature.
    #[test]
    fn two_datasig_valid_both_sigs_accepted_by_engine() {
        let kp_a = test_keypair(0x11);
        let kp_b = test_keypair(0x22);
        let pk_a = kp_a.x_only_public_key().0.serialize();
        let pk_b = kp_b.x_only_public_key().0.serialize();
        let msg_hash: [u8; 32] = [0xABu8; 32];

        let redeem = two_datasig_redeem_script(&pk_a, &pk_b, &msg_hash)
            .expect("two_datasig_redeem_script failed");

        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount);

        // Oracle A and Oracle B each sign msg_hash independently (64-byte sigs).
        let sig_a = datasig(&msg_hash, &kp_a);
        let sig_b = datasig(&msg_hash, &kp_b);

        // Satisfier: sig_a deepest, sig_b on top — matches the choreography in
        // two_datasig_redeem_script.
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_a, sig_b], &redeem).unwrap();

        // Engine must accept — covenants_enabled=true enables CSFS.
        verify_p2sh_spend_offline(&tx, 0, &entry, true)
            .expect("engine must ACCEPT both valid data-signatures");
    }

    // ── Wrong sig_a ───────────────────────────────────────────────────────────

    /// Engine REJECTS a spend where sig_a is signed by the wrong key.
    ///
    /// A key not in the script cannot forge a valid sig for pkA's slot.
    #[test]
    fn two_datasig_wrong_sig_a_rejected() {
        let kp_a = test_keypair(0x11);
        let kp_b = test_keypair(0x22);
        let kp_wrong = test_keypair(0x99);
        let pk_a = kp_a.x_only_public_key().0.serialize();
        let pk_b = kp_b.x_only_public_key().0.serialize();
        let msg_hash: [u8; 32] = [0xABu8; 32];

        let redeem = two_datasig_redeem_script(&pk_a, &pk_b, &msg_hash)
            .expect("two_datasig_redeem_script failed");

        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount);

        // sig_a is signed by kp_wrong instead of kp_a.
        let sig_a_bad = datasig(&msg_hash, &kp_wrong);
        let sig_b = datasig(&msg_hash, &kp_b);

        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_a_bad, sig_b], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, true).is_err(),
            "engine must REJECT spend with wrong sig_a (wrong key for pkA slot)"
        );
    }

    // ── Wrong sig_b ───────────────────────────────────────────────────────────

    /// Engine REJECTS a spend where sig_b is signed by the wrong key.
    #[test]
    fn two_datasig_wrong_sig_b_rejected() {
        let kp_a = test_keypair(0x11);
        let kp_b = test_keypair(0x22);
        let kp_wrong = test_keypair(0x99);
        let pk_a = kp_a.x_only_public_key().0.serialize();
        let pk_b = kp_b.x_only_public_key().0.serialize();
        let msg_hash: [u8; 32] = [0xABu8; 32];

        let redeem = two_datasig_redeem_script(&pk_a, &pk_b, &msg_hash)
            .expect("two_datasig_redeem_script failed");

        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount);

        let sig_a = datasig(&msg_hash, &kp_a);
        // sig_b is signed by kp_wrong instead of kp_b.
        let sig_b_bad = datasig(&msg_hash, &kp_wrong);

        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_a, sig_b_bad], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, true).is_err(),
            "engine must REJECT spend with wrong sig_b (wrong key for pkB slot)"
        );
    }

    // ── Swapped sigs ──────────────────────────────────────────────────────────

    /// Engine REJECTS a spend where sig_a and sig_b are swapped.
    ///
    /// Even though both signatures are valid over `msg_hash`, each is bound to
    /// a specific key slot. Swapping them fails both CSFS checks.
    #[test]
    fn two_datasig_swapped_sigs_rejected() {
        let kp_a = test_keypair(0x11);
        let kp_b = test_keypair(0x22);
        let pk_a = kp_a.x_only_public_key().0.serialize();
        let pk_b = kp_b.x_only_public_key().0.serialize();
        let msg_hash: [u8; 32] = [0xABu8; 32];

        let redeem = two_datasig_redeem_script(&pk_a, &pk_b, &msg_hash)
            .expect("two_datasig_redeem_script failed");

        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount);

        let sig_a = datasig(&msg_hash, &kp_a);
        let sig_b = datasig(&msg_hash, &kp_b);

        // Provide sig_b where sig_a is expected and vice versa.
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_b, sig_a], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, true).is_err(),
            "engine must REJECT spend with swapped sigs (sig_b in pkA slot, sig_a in pkB slot)"
        );
    }
}
