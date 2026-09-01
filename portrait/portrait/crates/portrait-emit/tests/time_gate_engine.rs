//! B1 time-gate: PROVE the emitted `after(...)` gate is real against the pinned
//! Toccata engine (rusty-kaspa tag `v2.0.0` = commit `90dbf07`).
//!
//! `portrait-emit` lowers an `after(deadline)` clause to a single `require` in the
//! emitted `.sil` (see `emit_lowers_after_guard_to_tx_time_cltv_require` in the
//! unit tests):
//!
//! ```silverscript
//! require(tx.time >= <committed deadline>);
//! ```
//!
//! The special TxVar `tx.time` routes through silverc's `compile_time_op_statement`
//! to `OpCheckLockTimeVerify` (0xb0). Its engine semantics (opcodes/mod.rs,
//! lines 1014-1064 of the pinned checkout) enforce, in ONE opcode:
//!   * a domain match — the committed deadline and the tx lock_time must both be
//!     DAA scores (`< LOCK_TIME_THRESHOLD`) or both Unix timestamps (`>=` it);
//!   * `stack_lock_time <= tx.lock_time` — the tx COMMITS a `lock_time >=` the
//!     committed deadline (it reads only the spender-set `lock_time` FIELD);
//!   * `input.sequence != MAX_TX_IN_SEQUENCE_NUM` — the input is NON-FINAL, which
//!     defeats the final-sequence bypass that would otherwise disable the gate.
//!
//! TWO HALVES — WHAT THIS FILE DOES AND DOES NOT PROVE (do not overclaim):
//! The "cannot be spent before the deadline" guarantee is enforced by TWO SEPARATE
//! consensus rules; this file exercises only the FIRST (the txscript half):
//!   1. CLTV (this file) proves the tx must COMMIT a `lock_time >= deadline` on a
//!      NON-FINAL input. `OpCheckLockTimeVerify` has NO access to the block DAA
//!      score (opcodes/mod.rs:1039,1057 compare only against the tx's own
//!      lock_time field), so it does NOT by itself prove the deadline has elapsed.
//!   2. The actual no-early-INCLUSION rule is the SEPARATE consensus finalization
//!      check `check_tx_is_finalized`
//!      (consensus/src/processes/transaction_validator/tx_validation_in_header_context.rs:72-93):
//!      a non-final tx with `lock_time = L` is admissible into the blockDAG only
//!      once `block_daa_score > L`. That is the load-bearing "time has passed"
//!      half. It is NOT unit-tested here: it lives OUTSIDE txscript and is
//!      `pub(crate)` (reachable only through the full VirtualProcessor pipeline,
//!      not from a dev-dependency), so it is out of scope for an isolated
//!      txscript-opcode test. Logged honestly, not silently omitted.
//!
//! Together: CLTV forces the tx to carry `lock_time >= deadline` on a non-final
//! input, and rule 2 then bars that tx from a block until the DAA score passes it.
//!
//! The redeem script under test isolates the gate as a P2SH redeem
//! (`add_i64(deadline); OpCheckLockTimeVerify; OpTrue`) so a pass/fail here is
//! attributable to the CLTV gate alone (no covenant-id / signature machinery —
//! those belong to other slices). CLTV is a VERIFY opcode: it pops the pushed
//! deadline and either errors or succeeds without pushing, so `OpTrue` follows to
//! leave a truthy value for the P2SH spend.
//!
//! Domain decision (recorded): DAA score. `deadline = 500` is far below
//! `LOCK_TIME_THRESHOLD = 500_000_000_000`, so both the committed deadline and the
//! test transactions' `lock_time` are interpreted as DAA scores (block-height-like)
//! — the domain the gate is authored for.
//!
//! HONEST SCOPE (mirrors `output_binding_engine.rs`): the three tests below isolate
//! the CLTV OPCODE (accept + two rejects, including the final-sequence bypass) and
//! the composed `.sil` is proven to COMPILE under the pinned silverc (exit 0). NOT
//! asserted here: (a) the consensus-finalization half (rule 2 above), unit-untested
//! (out of txscript scope); (b) a composed end-to-end on-engine SPEND — assembling
//! it needs silverscript-lang's covenant sig-script/ABI, which pins a floating
//! engine branch incompatible with the mandated `v2.0.0` pin. See
//! library/ENFORCEMENT.md.

use kaspa_consensus_core::hashing::sighash::SigHashReusedValuesUnsync;
use kaspa_consensus_core::subnets::SubnetworkId;
use kaspa_consensus_core::tx::{
    PopulatedTransaction, ScriptPublicKey, Transaction, TransactionId, TransactionInput,
    TransactionOutpoint, TransactionOutput, UtxoEntry, VerifiableTransaction,
};
use kaspa_txscript::caches::Cache;
use kaspa_txscript::covenants::CovenantsContext;
use kaspa_txscript::opcodes::codes::{OpAdd, OpCheckLockTimeVerify, OpTrue};
use kaspa_txscript::script_builder::ScriptBuilder;
use kaspa_txscript::{
    pay_to_script_hash_script, EngineCtx, EngineFlags, TxScriptEngine, MAX_TX_IN_SEQUENCE_NUM,
};
use kaspa_txscript_errors::TxScriptError;

/// The committed deadline, chosen in the DAA-score domain (well below
/// `LOCK_TIME_THRESHOLD = 500_000_000_000`) and above 16 so `add_i64` takes the
/// data-push path (values 0..=16 take an OpN fast path) — CLTV pads the raw push
/// to 8 bytes and reads it little-endian, so 500 round-trips to `stack_lock_time`.
const DEADLINE: i64 = 500;

/// The redeem script mirroring the emitted `after(deadline)` lowering: push the
/// committed deadline, `OpCheckLockTimeVerify`, then `OpTrue` so the P2SH spend
/// succeeds when the gate holds.
fn time_gate_redeem_script(deadline: i64) -> Vec<u8> {
    let mut b = ScriptBuilder::new();
    // require(tx.time >= deadline)  →  push deadline; OpCheckLockTimeVerify.
    b.add_i64(deadline).unwrap();
    b.add_op(OpCheckLockTimeVerify).unwrap();
    b.add_op(OpTrue).unwrap();
    b.drain()
}

/// Run `redeem` as a P2SH input on a transaction whose `lock_time` and input
/// `sequence` are set by the caller — the two fields the CLTV gate inspects.
fn run_time_gate(redeem: &[u8], lock_time: u64, sequence: u64) -> Result<(), TxScriptError> {
    let sig_script = ScriptBuilder::new().add_data(redeem).unwrap().drain();
    let input = TransactionInput::new(
        TransactionOutpoint::new(TransactionId::from_bytes([1; 32]), 0),
        sig_script,
        sequence,
        0,
    );
    let utxo_spk = pay_to_script_hash_script(redeem);
    let entries = vec![UtxoEntry::new(100_000, utxo_spk, 0, false, None)];
    // A trivial output so the transaction is well-formed; the CLTV gate does not
    // inspect outputs.
    let output0 = TransactionOutput::new(100_000, ScriptPublicKey::from_vec(0, vec![OpTrue]));
    let tx = Transaction::new(
        1,
        vec![input],
        vec![output0],
        lock_time,
        SubnetworkId::from_bytes([0u8; 20]),
        0,
        vec![],
    );

    let reused = SigHashReusedValuesUnsync::new();
    let sig_cache = Cache::new(10_000);
    let input0 = tx.inputs[0].clone();
    let populated = PopulatedTransaction::new(&tx, entries);
    let cov_ctx = CovenantsContext::from_tx(&populated).map_err(TxScriptError::from)?;
    let utxo = populated.utxo(0).expect("input utxo");
    let mut vm = TxScriptEngine::from_transaction_input(
        &populated,
        &input0,
        0,
        utxo,
        EngineCtx::new(&sig_cache)
            .with_reused(&reused)
            .with_covenants_ctx(&cov_ctx),
        EngineFlags {
            covenants_enabled: true,
            sigop_script_units: 0.into(),
        },
    );
    vm.execute()
}

#[test]
fn engine_accepts_spend_at_or_after_deadline_with_nonfinal_input() {
    // lock_time == deadline (the boundary: `stack_lock_time <= tx.lock_time`),
    // input NON-FINAL (sequence 0) → the gate is satisfied.
    let redeem = time_gate_redeem_script(DEADLINE);
    run_time_gate(&redeem, DEADLINE as u64, 0).expect(
        "engine must ACCEPT a spend at/after the committed deadline with a non-final input",
    );
}

#[test]
fn engine_rejects_early_spend_before_deadline() {
    // lock_time < deadline → the tx commits a lock_time below the committed
    // deadline; CLTV must reject with UnsatisfiedLockTime ("locktime requirement
    // not satisfied"). (The DAA-score "has the deadline elapsed" step is the
    // separate finalization rule — see the file header, half 2.)
    let redeem = time_gate_redeem_script(DEADLINE);
    let err = run_time_gate(&redeem, (DEADLINE as u64) - 1, 0)
        .expect_err("engine must REJECT a spend before the committed deadline");
    assert!(
        matches!(err, TxScriptError::UnsatisfiedLockTime(_)),
        "expected an UnsatisfiedLockTime rejection for an early spend, got {err:?}"
    );
}

#[test]
fn engine_rejects_final_sequence_bypass_at_deadline() {
    // THE soundness-decisive test. lock_time >= deadline (the lock-time bound is
    // satisfied) BUT the input is FINAL (sequence == MAX_TX_IN_SEQUENCE_NUM). A
    // bare lock-time compare would be BYPASSED here; the bundled non-final check in
    // OpCheckLockTimeVerify rejects it with UnsatisfiedLockTime ("input is
    // finalized"). This is why the emitter uses `tx.time` (CLTV), never a bare
    // `tx.locktime` compare.
    let redeem = time_gate_redeem_script(DEADLINE);
    let err = run_time_gate(&redeem, DEADLINE as u64, MAX_TX_IN_SEQUENCE_NUM)
        .expect_err("engine must REJECT the final-sequence bypass even at/after the deadline");
    assert!(
        matches!(err, TxScriptError::UnsatisfiedLockTime(_)),
        "expected an UnsatisfiedLockTime rejection for the final-sequence bypass, got {err:?}"
    );
}

// ── B1 (D1) window-sum CLTV: DIRECT engine evidence (RT-2) ──────────────────
//
// The emitted `after(a + b)` window lowers to `require(tx.time >= prev_states[0].a
// + prev_states[0].b)`, whose redeem shape is `push a; push b; OpAdd;
// OpCheckLockTimeVerify; OpTrue` — the threshold is computed ON STACK from the two
// committed atoms, then fed to the SAME CLTV opcode the single-field form uses.
// These two tests drive the real `TxScriptEngine` on the summed shape so the
// window-sum guarantee is DIRECT engine evidence, not a transitive argument.

/// The redeem script mirroring the emitted `after(a + b)` lowering: push both
/// committed atoms, `OpAdd` to form the threshold, then `OpCheckLockTimeVerify` +
/// `OpTrue`.
fn time_gate_sum_redeem_script(a: i64, b: i64) -> Vec<u8> {
    let mut sb = ScriptBuilder::new();
    sb.add_i64(a).unwrap();
    sb.add_i64(b).unwrap();
    sb.add_op(OpAdd).unwrap();
    sb.add_op(OpCheckLockTimeVerify).unwrap();
    sb.add_op(OpTrue).unwrap();
    sb.drain()
}

#[test]
fn engine_accepts_summed_deadline() {
    // anchor 300 + duration 200 = 500 threshold (DAA domain, both > 16 so add_i64
    // takes the data-push path). lock_time == a+b on a NON-FINAL input → ACCEPT;
    // lock_time == a+b-1 → REJECT (UnsatisfiedLockTime). Proves the on-stack sum is
    // the CLTV threshold, matching the single-field gate's semantics.
    let (a, b) = (300_i64, 200_i64);
    let redeem = time_gate_sum_redeem_script(a, b);
    run_time_gate(&redeem, (a + b) as u64, 0)
        .expect("engine must ACCEPT a spend at/after the summed committed deadline (non-final)");
    let err = run_time_gate(&redeem, (a + b - 1) as u64, 0)
        .expect_err("engine must REJECT a spend before the summed committed deadline");
    assert!(
        matches!(err, TxScriptError::UnsatisfiedLockTime(_)),
        "expected an UnsatisfiedLockTime rejection below the summed deadline, got {err:?}"
    );
}

#[test]
fn engine_summed_deadline_overflow_fails_closed() {
    // Fail-CLOSED: a summed threshold that overflows i64 (`i64::MAX + 1`) must ERROR
    // (`OpAdd` uses `checked_add` → NumberTooBig), NOT wrap to a small threshold that
    // would silently open the gate. sema rejects anchor+anchor before emit, but the
    // engine itself is the last line of defence, so prove the opcode fails closed.
    let redeem = time_gate_sum_redeem_script(i64::MAX, 1);
    let err = run_time_gate(&redeem, u64::MAX, 0)
        .expect_err("engine must FAIL CLOSED on a summed threshold that overflows i64");
    assert!(
        matches!(err, TxScriptError::NumberTooBig(_)),
        "expected a NumberTooBig fail-closed error on overflow, got {err:?}"
    );
}
