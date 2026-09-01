//! In-repo engine proof for the transferable-record covenant.
//!
//! Runs the **committed** artifact
//! (`covenant/transferable-record.compiled.json`) through the real pinned
//! consensus **script VM** — `rusty-kaspa` tag `v2.0.0`, commit `90dbf07` —
//! with `covenants_enabled: true` and a real `CovenantsContext::from_tx`, and
//! checks that TR-1, TR-2 and TR-3 are enforced *by the script VM*, not by this
//! crate's Rust types.
//!
//! Per-state scripts are produced by splicing the state region of the committed
//! artifact ([`kcp_common::covenant`]); the program body is never touched, and
//! `tr_committed_artifact_matches_script_hex` checks that the artifact's
//! `script` field and the committed `.script.hex` agree byte-for-byte and that
//! splicing state leaves everything outside the state window unchanged. There
//! is no `silverscript-lang` dependency (it would float the engine pin) and no
//! fixture holding key material: each test derives a deterministic keypair from
//! a fixed, never-funded seed and splices the matching `controllerPk` into the
//! state.
//!
//! **What is NOT covered.**
//! - *Transaction-level validation.* Only `TxScriptEngine::from_transaction_input`
//!   for input 0 runs here: no transaction mass, no KIP-9 storage mass, no
//!   standardness. The covenant places no floor on the output value, so a caller
//!   must still respect `MIN_CHANGE_SOMPI` and the storage-mass bound itself.
//! - *The live half of `[KCP-TR-003]`.* Submitting a transaction is not
//!   reproducible from a test. The committed-artifact↔deployed-script
//!   correspondence rests on an archived out-of-repo capture; the only in-repo
//!   corroboration is the execution-cost fingerprint asserted by
//!   `*_engine_cost_matches_recorded_live_preflight`, which is evidence, not a
//!   binding. See `docs/EVIDENCE.md`.

use kaspa_bip32::secp256k1::{Keypair, SECP256K1};
use kaspa_consensus_core::{
    constants::TX_VERSION_TOCCATA,
    hashing::sighash::SigHashReusedValuesUnsync,
    mass::ComputeBudget,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        CovenantBinding, PopulatedTransaction, Transaction, TransactionId, TransactionInput,
        TransactionOutpoint, TransactionOutput, UtxoEntry, VerifiableTransaction,
    },
    Hash,
};
use kaspa_txscript::{
    caches::Cache, covenants::CovenantsContext, script_builder::ScriptBuilder, EngineCtx,
    EngineFlags, TxScriptEngine,
};

use kcp_common::covenant::{append_signature_script, push_state_field, CompiledCovenant};
use kcp_common::p2sh::{p2sh_input_sighash, p2sh_lock_script, schnorr_satisfier_sig};
use kcp_common::tx::CARRIER_FEE_SOMPI;

const RECORD_VALUE_SOMPI: u64 = 10_000_000;

/// Placeholder covenant id shared by the input entry and the output binding —
/// the continuation shape `CovenantsContext::from_tx` reconstructs.
fn covenant_id() -> Hash {
    Hash::from_bytes(*b"KCP-TR-ENGINE-TEST-COVENANT-ID01")
}

/// One transferable-record covenant state.
#[derive(Clone)]
struct State {
    record_id: [u8; 32],
    seq: i64,
    controller_pk: [u8; 32],
}

fn artifact() -> CompiledCovenant {
    let path = format!(
        "{}/covenant/transferable-record.compiled.json",
        env!("CARGO_MANIFEST_DIR")
    );
    CompiledCovenant::load(std::path::Path::new(&path)).expect("load committed covenant artifact")
}

/// The state region as silverc lays it out: fixed-width, explicit pushes.
fn state_region(s: &State) -> Vec<u8> {
    let mut out = Vec::new();
    for field in [&s.record_id[..], &s.seq.to_le_bytes(), &s.controller_pk[..]] {
        push_state_field(&mut out, field).expect("state field width");
    }
    out
}

/// The same fields as `newStates[0]` arguments in the signature script —
/// canonical pushes, which is a different encoding from the state region.
fn state_args(s: &State) -> Vec<u8> {
    let mut builder = ScriptBuilder::with_flags(EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    });
    for field in [&s.record_id[..], &s.seq.to_le_bytes(), &s.controller_pk[..]] {
        builder.add_data(field).expect("push state argument");
    }
    builder.drain().to_vec()
}

/// Run one input through the real v2.0.0 script VM with covenants enabled.
/// Returns the consumed script units on accept.
fn covenant_engine_run(tx: &Transaction, entries: &[UtxoEntry]) -> Result<u64, String> {
    let populated = PopulatedTransaction::new(tx, entries.to_vec());
    let cov_ctx = CovenantsContext::from_tx(&populated)
        .map_err(|e| format!("CovenantsContext::from_tx: {e:?}"))?;
    let utxo = populated.utxo(0).ok_or("no utxo for input 0")?;
    let sig_cache = Cache::new(0);
    let reused = SigHashReusedValuesUnsync::new();
    let ctx = EngineCtx::new(&sig_cache)
        .with_reused(&reused)
        .with_covenants_ctx(&cov_ctx);
    let mut vm = TxScriptEngine::from_transaction_input(
        &populated,
        &tx.inputs[0],
        0,
        utxo,
        ctx,
        EngineFlags {
            covenants_enabled: true,
            ..Default::default()
        },
    );
    vm.execute()
        .map_err(|e| format!("covenant engine rejected: {e:?}"))?;
    Ok(vm.used_script_units().0)
}

/// Build the transfer spending `prev`'s covenant UTXO into `next`'s, signed by
/// `signer`, and run it through the script VM.
///
/// Version-1 (Toccata) inputs commit a compute budget that the signature
/// covers, so the budget is measured under `u16::MAX` and the input re-signed
/// at the covering budget — the same two-round dance the live deployment does.
fn run_transfer(prev: &State, next: &State, signer: &Keypair) -> Result<u64, String> {
    let cov = artifact();
    let prev_script = cov.with_state(&state_region(prev));
    let next_script = cov.with_state(&state_region(next));
    let cov_id = covenant_id();

    let mut output = TransactionOutput::new(
        RECORD_VALUE_SOMPI - CARRIER_FEE_SOMPI,
        p2sh_lock_script(&next_script),
    );
    output.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id,
    });

    let outpoint = TransactionOutpoint::new(TransactionId::from_bytes([0xab; 32]), 0);
    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(outpoint, vec![], 0, 0)],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let entry = UtxoEntry::new(
        RECORD_VALUE_SOMPI,
        p2sh_lock_script(&prev_script),
        0,
        false,
        Some(cov_id),
    );

    let sigscript_for = |tx: &Transaction| -> Vec<u8> {
        let sighash = p2sh_input_sighash(tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, signer);
        append_signature_script(&state_args(next), &sig, &[], &prev_script)
            .expect("assemble transfer signature script")
    };

    tx.inputs[0].compute_commit = ComputeBudget(u16::MAX).into();
    tx.inputs[0].signature_script = sigscript_for(&tx);
    let used = covenant_engine_run(&tx, std::slice::from_ref(&entry))?;

    // 1 budget unit = 10_000 script units; +3 units of margin.
    let budget_units = (used / 10_000 + 3).min(u16::MAX as u64) as u16;
    tx.inputs[0].compute_commit = ComputeBudget(budget_units).into();
    tx.inputs[0].signature_script = sigscript_for(&tx);
    covenant_engine_run(&tx, std::slice::from_ref(&entry))
}

/// Two-sided oracle for a negative case.
///
/// `violating` must be rejected — and by the covenant's own `require`
/// (`VerifyError` / `EvalFalse`), not by a malformed script or a broken
/// transaction shape. `control` is the *same* case with only the offending
/// field restored, and must be accepted. Together these pin the rejection to
/// that field: a `VerifyError` alone cannot say which `require` fired (every
/// `require`, the P2SH redeem-hash check, `OpCheckSigVerify` and the
/// wrong-selector fallthrough all produce one), and the pair stays valid if the
/// covenant's checks are ever reordered.
fn assert_rejected_for(
    invariant: &str,
    violating: Result<u64, String>,
    control: Result<u64, String>,
) {
    let err = match violating {
        Ok(units) => {
            panic!("{invariant} violation was ACCEPTED by the script VM ({units} script units)")
        }
        Err(err) => err,
    };
    assert!(
        err.ends_with("VerifyError") || err.ends_with("EvalFalse"),
        "{invariant} must be rejected by the covenant itself, got: {err}"
    );
    control.unwrap_or_else(|e| {
        panic!(
            "{invariant} control (same case, offending field restored) must be ACCEPTED, got: {e}"
        )
    });
}

/// A deterministic, never-funded test keypair. Fixed seeds keep the accept and
/// reject runs of a case byte-comparable.
fn keypair(seed: u8) -> Keypair {
    Keypair::from_seckey_slice(SECP256K1, &[seed; 32]).expect("valid secret key")
}

fn x_only(kp: &Keypair) -> [u8; 32] {
    kp.x_only_public_key().0.serialize()
}

/// Genesis state (`seq = 0`) held by `from`, and the valid transfer to `to`.
fn genesis_and_transfer(from: &Keypair, to: &Keypair) -> (State, State) {
    let record_id = [0x11u8; 32];
    (
        State {
            record_id,
            seq: 0,
            controller_pk: x_only(from),
        },
        State {
            record_id,
            seq: 1,
            controller_pk: x_only(to),
        },
    )
}

/// The committed artifact is internally consistent, and splicing a state
/// rewrites only the state window.
///
/// This proves three things and no more: (1) the `script` field of
/// `transferable-record.compiled.json` and `transferable-record.script.hex` are
/// the same bytes, so the two committed representations cannot drift apart; (2)
/// `with_state` rewrites exactly `[state_start, state_start + state_len)` at the
/// artifact's real widths, leaving the program body byte-identical; and (3)
/// [`state_region`] reproduces the artifact's own genesis-template state region
/// byte-for-byte, so this file's encoder agrees with what silverc emitted. It
/// says nothing about what was deployed on-chain.
#[test]
fn tr_committed_artifact_matches_script_hex() {
    let committed = hex::decode(
        std::fs::read_to_string(format!(
            "{}/covenant/transferable-record.script.hex",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("read committed script hex")
        .trim(),
    )
    .expect("decode committed script hex");

    let cov = artifact();
    assert_eq!(
        cov.script, committed,
        "compiled.json `script` != .script.hex"
    );

    let (_, transfer) = genesis_and_transfer(&keypair(0x11), &keypair(0x12));
    let spliced = cov.with_state(&state_region(&transfer));
    let body = cov.state_start + cov.state_len;
    assert_eq!(spliced.len(), committed.len());
    assert_eq!(&spliced[..cov.state_start], &committed[..cov.state_start]);
    assert_eq!(
        &spliced[body..],
        &committed[body..],
        "splicing state must leave the program body untouched"
    );
    assert_ne!(
        &spliced[cov.state_start..body],
        &committed[cov.state_start..body],
        "the state window must actually change, or the test above is vacuous"
    );

    // The artifact was compiled with the all-zero genesis template;
    // re-encoding it must reproduce the artifact exactly, which pins this
    // file's field order, widths and push encoding.
    let template = State {
        record_id: [0u8; 32],
        seq: 0,
        controller_pk: [0u8; 32],
    };
    assert_eq!(
        cov.with_state(&state_region(&template)),
        committed,
        "state_region must reproduce the artifact's own genesis-template region"
    );
}

/// ACCEPT baseline: the current controller transfers the record onward.
#[test]
fn tr_engine_accepts_valid_first_transfer() {
    let alice = keypair(0x21);
    let bob = keypair(0x22);
    let (prev, next) = genesis_and_transfer(&alice, &bob);
    run_transfer(&prev, &next, &alice).expect("valid first transfer must be ACCEPTED");
}

/// ACCEPT: the chain continues past the first hop — bob then transfers onward.
#[test]
fn tr_engine_accepts_second_transfer() {
    let bob = keypair(0x29);
    let carol = keypair(0x2a);
    let (_, held_by_bob) = genesis_and_transfer(&keypair(0x28), &bob);
    let mut next = held_by_bob.clone();
    next.seq = held_by_bob.seq + 1;
    next.controller_pk = x_only(&carol);
    run_transfer(&held_by_bob, &next, &bob).expect("valid second transfer must be ACCEPTED");
}

/// TR-1: `newState.seq == prevState.seq + 1`.
#[test]
fn tr_engine_rejects_seq_not_incremented() {
    let alice = keypair(0x23);
    let bob = keypair(0x24);
    let (prev, next) = genesis_and_transfer(&alice, &bob);
    let mut bad = next.clone();
    bad.seq = prev.seq;
    assert_rejected_for(
        "TR-1 (seq not incremented)",
        run_transfer(&prev, &bad, &alice),
        run_transfer(&prev, &next, &alice),
    );
}

/// TR-1: a skipped sequence number is rejected just as a repeated one is.
#[test]
fn tr_engine_rejects_seq_skip() {
    let alice = keypair(0x2b);
    let bob = keypair(0x2c);
    let (prev, next) = genesis_and_transfer(&alice, &bob);
    let mut bad = next.clone();
    bad.seq = prev.seq + 2;
    assert_rejected_for(
        "TR-1 (seq skip)",
        run_transfer(&prev, &bad, &alice),
        run_transfer(&prev, &next, &alice),
    );
}

/// TR-2: `newState.record_id == prevState.record_id`.
#[test]
fn tr_engine_rejects_record_id_change() {
    let alice = keypair(0x25);
    let bob = keypair(0x26);
    let (prev, next) = genesis_and_transfer(&alice, &bob);
    let mut bad = next.clone();
    bad.record_id = [0x88u8; 32];
    assert_rejected_for(
        "TR-2 (record_id change)",
        run_transfer(&prev, &bad, &alice),
        run_transfer(&prev, &next, &alice),
    );
}

/// TR-3: only the CURRENT controller authorises a transfer — the incoming
/// controller cannot pull the record to itself.
#[test]
fn tr_engine_rejects_wrong_signature() {
    let alice = keypair(0x27);
    let bob = keypair(0x28);
    let (prev, next) = genesis_and_transfer(&alice, &bob);
    assert_rejected_for(
        "TR-3 (wrong signer)",
        run_transfer(&prev, &next, &bob),
        run_transfer(&prev, &next, &alice),
    );
}

/// The one link in this file that reaches the **deployed** script.
///
/// The live `KCP-TR-003` deployment's engine preflight is recorded in the evidence
/// register as consuming exactly `105047` script units. Script units are a
/// deterministic function of the executed opcode trace and the operand widths,
/// so re-running the *committed* artifact here and consuming the same count
/// corroborates that the deployed program body is this program body.
///
/// **This is corroboration, not a binding.** The count is insensitive to the
/// state *values* (the fields are fixed-width), the recorded number is itself
/// out-of-repo, and the on-chain `covenant_id` is derived from the funding
/// outpoint rather than the script — so it cannot be recomputed here. See
/// `docs/EVIDENCE.md`.
const LIVE_PREFLIGHT_SCRIPT_UNITS: u64 = 105047;

#[test]
fn tr_engine_cost_matches_recorded_live_preflight() {
    {
        let alice = keypair(0x51);
        let bob = keypair(0x52);
        let (prev, next) = genesis_and_transfer(&alice, &bob);
        let used = run_transfer(&prev, &next, &alice).expect("valid transfer must be ACCEPTED");
        assert_eq!(
            used, LIVE_PREFLIGHT_SCRIPT_UNITS,
            "committed artifact no longer costs what the live KCP-TR-003 preflight cost"
        );
    }
}
