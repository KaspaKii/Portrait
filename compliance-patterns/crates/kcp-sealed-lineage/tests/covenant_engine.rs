//! In-repo engine proof for the sealed-lineage covenant.
//!
//! Runs the **committed** artifact (`covenant/sealed-lineage.compiled.json`)
//! through the real pinned consensus **script VM** — `rusty-kaspa` tag
//! `v2.0.0`, commit `90dbf07` — with `covenants_enabled: true` and a real
//! `CovenantsContext::from_tx`, and checks that L-1, L-2, L-3, L-4 and the
//! ownership rule are enforced *by the script VM*, not by this crate's Rust
//! types.
//!
//! Per-state scripts are produced by splicing the state region of the committed
//! artifact ([`kcp_common::covenant`]); the program body is never touched, and
//! `sl_committed_artifact_matches_script_hex` checks that the artifact's
//! `script` field and the committed `.script.hex` agree byte-for-byte and that
//! splicing state leaves everything outside the state window unchanged. There
//! is no `silverscript-lang` dependency (it would float the engine pin) and no
//! fixture holding key material: each test derives a deterministic keypair from
//! a fixed, never-funded seed and splices the matching `publisherPk` into the
//! state.
//!
//! **What is NOT covered.**
//! - *Transaction-level validation.* Only `TxScriptEngine::from_transaction_input`
//!   for input 0 runs here: no transaction mass, no KIP-9 storage mass, no
//!   standardness. The covenant places no floor on the output value, so a caller
//!   must still respect `MIN_CHANGE_SOMPI` and the storage-mass bound itself.
//! - *The live half of `[KCP-SL-003]`.* Submitting a transaction is not
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

/// Event class: Genesis (`seq = 0`).
const GENESIS_CLASS: u8 = 0x00;
/// Event class: Append.
const APPEND_CLASS: u8 = 0x01;
/// Event class: Close — terminal; no further append is possible (L-3).
const CLOSE_CLASS: u8 = 0x02;
/// 90 days in seconds — the L-4 envelope maximum step.
const T_BUCKET_MAX_STEP: i64 = 7_776_000;

const LINEAGE_VALUE_SOMPI: u64 = 10_000_000;

/// Placeholder covenant id shared by the input entry and the output binding —
/// the continuation shape `CovenantsContext::from_tx` reconstructs.
fn covenant_id() -> Hash {
    Hash::from_bytes(*b"KCP-SL-ENGINE-TEST-COVENANT-ID01")
}

/// One sealed-lineage covenant state.
#[derive(Clone)]
struct State {
    lineage_id: [u8; 32],
    seq: i64,
    event_class: u8,
    t_bucket: i64,
    publisher_pk: [u8; 32],
}

fn artifact() -> CompiledCovenant {
    let path = format!(
        "{}/covenant/sealed-lineage.compiled.json",
        env!("CARGO_MANIFEST_DIR")
    );
    CompiledCovenant::load(std::path::Path::new(&path)).expect("load committed covenant artifact")
}

/// The state region as silverc lays it out: fixed-width, explicit pushes.
fn state_region(s: &State) -> Vec<u8> {
    let mut out = Vec::new();
    for field in [
        &s.lineage_id[..],
        &s.seq.to_le_bytes(),
        &[s.event_class],
        &s.t_bucket.to_le_bytes(),
        &s.publisher_pk[..],
    ] {
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
    for field in [
        &s.lineage_id[..],
        &s.seq.to_le_bytes(),
        &[s.event_class],
        &s.t_bucket.to_le_bytes(),
        &s.publisher_pk[..],
    ] {
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

/// Build the append spending `prev`'s covenant UTXO into `next`'s, signed by
/// `signer`, and run it through the script VM.
///
/// Version-1 (Toccata) inputs commit a compute budget that the signature
/// covers, so the budget is measured under `u16::MAX` and the input re-signed
/// at the covering budget — the same two-round dance the live deployment does.
fn run_append(prev: &State, next: &State, signer: &Keypair) -> Result<u64, String> {
    let cov = artifact();
    let prev_script = cov.with_state(&state_region(prev));
    let next_script = cov.with_state(&state_region(next));
    let cov_id = covenant_id();

    let mut output = TransactionOutput::new(
        LINEAGE_VALUE_SOMPI - CARRIER_FEE_SOMPI,
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
        LINEAGE_VALUE_SOMPI,
        p2sh_lock_script(&prev_script),
        0,
        false,
        Some(cov_id),
    );

    let sigscript_for = |tx: &Transaction| -> Vec<u8> {
        let sighash = p2sh_input_sighash(tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, signer);
        append_signature_script(&state_args(next), &sig, &[], &prev_script)
            .expect("assemble append signature script")
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

/// Genesis state (`seq = 0`) for `publisher`, and the valid append that follows.
fn genesis_and_append(publisher: &Keypair) -> (State, State) {
    let lineage_id = [0x11u8; 32];
    let pk = x_only(publisher);
    let t0 = 1_700_000_000;
    (
        State {
            lineage_id,
            seq: 0,
            event_class: GENESIS_CLASS,
            t_bucket: t0,
            publisher_pk: pk,
        },
        State {
            lineage_id,
            seq: 1,
            event_class: APPEND_CLASS,
            t_bucket: t0,
            publisher_pk: pk,
        },
    )
}

/// The committed artifact is internally consistent, and splicing a state
/// rewrites only the state window.
///
/// This proves three things and no more: (1) the `script` field of
/// `sealed-lineage.compiled.json` and `sealed-lineage.script.hex` are the same
/// bytes, so the two committed representations cannot drift apart; (2)
/// `with_state` rewrites exactly `[state_start, state_start + state_len)` at the
/// artifact's real widths, leaving the program body byte-identical; and (3)
/// [`state_region`] reproduces the artifact's own genesis-template state region
/// byte-for-byte, so this file's encoder agrees with what silverc emitted. It
/// says nothing about what was deployed on-chain.
#[test]
fn sl_committed_artifact_matches_script_hex() {
    let committed = hex::decode(
        std::fs::read_to_string(format!(
            "{}/covenant/sealed-lineage.script.hex",
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

    let (_, append) = genesis_and_append(&keypair(0x11));
    let spliced = cov.with_state(&state_region(&append));
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
    // The artifact was compiled with the zero-state genesis template
    // (t_bucket = 1_700_000_000); re-encoding it must reproduce the artifact
    // exactly, which pins this file's field order, widths and push encoding.
    let template = State {
        lineage_id: [0u8; 32],
        seq: 0,
        event_class: GENESIS_CLASS,
        t_bucket: 1_700_000_000,
        publisher_pk: [0u8; 32],
    };
    assert_eq!(
        cov.with_state(&state_region(&template)),
        committed,
        "state_region must reproduce the artifact's own genesis-template region"
    );
}

/// ACCEPT baseline: valid append, genesis → append.
#[test]
fn sl_engine_accepts_valid_append_from_genesis() {
    let publisher = keypair(0x21);
    let (prev, next) = genesis_and_append(&publisher);
    run_append(&prev, &next, &publisher).expect("valid genesis→append must be ACCEPTED");
}

/// ACCEPT: the chain continues past the first step — append → append.
#[test]
fn sl_engine_accepts_valid_append_to_append() {
    let publisher = keypair(0x27);
    let (_, append) = genesis_and_append(&publisher);
    let mut next = append.clone();
    next.seq = append.seq + 1;
    run_append(&append, &next, &publisher).expect("valid append→append must be ACCEPTED");
}

/// L-1: `newState.seq == prevState.seq + 1`.
#[test]
fn sl_engine_rejects_seq_not_incremented() {
    let publisher = keypair(0x22);
    let (prev, next) = genesis_and_append(&publisher);
    let mut bad = next.clone();
    bad.seq = prev.seq;
    assert_rejected_for(
        "L-1 (seq not incremented)",
        run_append(&prev, &bad, &publisher),
        run_append(&prev, &next, &publisher),
    );
}

/// L-1: a skipped sequence number is rejected just as a repeated one is.
#[test]
fn sl_engine_rejects_seq_skip() {
    let publisher = keypair(0x28);
    let (prev, next) = genesis_and_append(&publisher);
    let mut bad = next.clone();
    bad.seq = prev.seq + 2;
    assert_rejected_for(
        "L-1 (seq skip)",
        run_append(&prev, &bad, &publisher),
        run_append(&prev, &next, &publisher),
    );
}

/// L-2: `newState.lineage_id == prevState.lineage_id`.
#[test]
fn sl_engine_rejects_lineage_id_change() {
    let publisher = keypair(0x23);
    let (prev, next) = genesis_and_append(&publisher);
    let mut bad = next.clone();
    bad.lineage_id = [0x88u8; 32];
    assert_rejected_for(
        "L-2 (lineage_id change)",
        run_append(&prev, &bad, &publisher),
        run_append(&prev, &next, &publisher),
    );
}

/// L-3: CLOSE is a legal append step — this is the transition that seals a
/// lineage, and the positive control for the terminality tests below.
#[test]
fn sl_engine_accepts_close_transition() {
    let publisher = keypair(0x29);
    let (prev, next) = genesis_and_append(&publisher);
    let mut close = next;
    close.event_class = CLOSE_CLASS;
    run_append(&prev, &close, &publisher).expect("append→close must be ACCEPTED");
}

/// L-3 (terminality — the permanence property of the whole pattern): once a
/// state is CLOSE, no successor can spend it.
#[test]
fn sl_engine_rejects_append_after_close() {
    let publisher = keypair(0x2a);
    let (_, append) = genesis_and_append(&publisher);
    let mut closed = append.clone();
    closed.event_class = CLOSE_CLASS;

    let mut next_from_closed = closed.clone();
    next_from_closed.seq = closed.seq + 1;
    next_from_closed.event_class = APPEND_CLASS;

    // Control: the identical successor spending a non-CLOSE predecessor.
    let mut next_from_open = append.clone();
    next_from_open.seq = append.seq + 1;

    assert_rejected_for(
        "L-3 (append after close)",
        run_append(&closed, &next_from_closed, &publisher),
        run_append(&append, &next_from_open, &publisher),
    );
}

/// L-3: a successor may not re-declare itself GENESIS.
#[test]
fn sl_engine_rejects_genesis_in_output_event_class() {
    let publisher = keypair(0x2b);
    let (prev, next) = genesis_and_append(&publisher);
    let mut bad = next.clone();
    bad.event_class = GENESIS_CLASS;
    assert_rejected_for(
        "L-3 (genesis event_class in output)",
        run_append(&prev, &bad, &publisher),
        run_append(&prev, &next, &publisher),
    );
}

/// L-4 boundary: exactly 90 days is inside the envelope.
#[test]
fn sl_engine_accepts_t_bucket_exactly_90_days() {
    let publisher = keypair(0x2c);
    let (prev, mut next) = genesis_and_append(&publisher);
    next.t_bucket = prev.t_bucket + T_BUCKET_MAX_STEP;
    run_append(&prev, &next, &publisher).expect("t_bucket at the 90-day bound must be ACCEPTED");
}

/// L-4: `newState.t_bucket <= prevState.t_bucket + 90 days`.
#[test]
fn sl_engine_rejects_t_bucket_exceeds_90_days() {
    let publisher = keypair(0x24);
    let (prev, next) = genesis_and_append(&publisher);
    let mut bad = next.clone();
    bad.t_bucket = prev.t_bucket + T_BUCKET_MAX_STEP + 1;
    assert_rejected_for(
        "L-4 (t_bucket beyond 90 days)",
        run_append(&prev, &bad, &publisher),
        run_append(&prev, &next, &publisher),
    );
}

/// L-4: `newState.t_bucket >= prevState.t_bucket` — time may not run backwards.
#[test]
fn sl_engine_rejects_t_bucket_decreasing() {
    let publisher = keypair(0x2d);
    let (prev, next) = genesis_and_append(&publisher);
    let mut bad = next.clone();
    bad.t_bucket = prev.t_bucket - 1;
    assert_rejected_for(
        "L-4 (t_bucket decreasing)",
        run_append(&prev, &bad, &publisher),
        run_append(&prev, &next, &publisher),
    );
}

/// Ownership: only the publisher named in the *input* state can append.
#[test]
fn sl_engine_rejects_wrong_signature() {
    let publisher = keypair(0x25);
    let impostor = keypair(0x26);
    let (prev, next) = genesis_and_append(&publisher);
    assert_rejected_for(
        "ownership (wrong signer)",
        run_append(&prev, &next, &impostor),
        run_append(&prev, &next, &publisher),
    );
}

/// The one link in this file that reaches the **deployed** script.
///
/// The live `KCP-SL-003` deployment's engine preflight is recorded in the evidence
/// register as consuming exactly `107149` script units. Script units are a
/// deterministic function of the executed opcode trace and the operand widths,
/// so re-running the *committed* artifact here and consuming the same count
/// corroborates that the deployed program body is this program body.
///
/// **This is corroboration, not a binding.** The count is insensitive to the
/// state *values* (the fields are fixed-width), the recorded number is itself
/// out-of-repo, and the on-chain `covenant_id` is derived from the funding
/// outpoint rather than the script — so it cannot be recomputed here. See
/// `docs/EVIDENCE.md`.
const LIVE_PREFLIGHT_SCRIPT_UNITS: u64 = 107149;

#[test]
fn sl_engine_cost_matches_recorded_live_preflight() {
    {
        let publisher = keypair(0x51);
        let (prev, next) = genesis_and_append(&publisher);
        let used = run_append(&prev, &next, &publisher).expect("valid append must be ACCEPTED");
        assert_eq!(
            used, LIVE_PREFLIGHT_SCRIPT_UNITS,
            "committed artifact no longer costs what the live KCP-SL-003 preflight cost"
        );
    }
}
