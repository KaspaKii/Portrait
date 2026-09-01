//! RED-TEAM adversarial probes against the B1 emitted time gate.
//! Mirrors time_gate_engine.rs harness; tries to SATISFY the CLTV gate while
//! spending EARLY, plus domain-confusion / multi-input / sequence-variant probes.

use kaspa_consensus_core::hashing::sighash::SigHashReusedValuesUnsync;
use kaspa_consensus_core::subnets::SubnetworkId;
use kaspa_consensus_core::tx::{
    PopulatedTransaction, ScriptPublicKey, Transaction, TransactionId, TransactionInput,
    TransactionOutpoint, TransactionOutput, UtxoEntry, VerifiableTransaction,
};
use kaspa_txscript::caches::Cache;
use kaspa_txscript::covenants::CovenantsContext;
use kaspa_txscript::opcodes::codes::{OpCheckLockTimeVerify, OpTrue};
use kaspa_txscript::script_builder::ScriptBuilder;
use kaspa_txscript::{
    pay_to_script_hash_script, EngineCtx, EngineFlags, TxScriptEngine, MAX_TX_IN_SEQUENCE_NUM,
};
use kaspa_txscript_errors::TxScriptError;

const DEADLINE: i64 = 500;
const THRESHOLD: u64 = 500_000_000_000;

fn time_gate_redeem_script(deadline: i64) -> Vec<u8> {
    let mut b = ScriptBuilder::new();
    b.add_i64(deadline).unwrap();
    b.add_op(OpCheckLockTimeVerify).unwrap();
    b.add_op(OpTrue).unwrap();
    b.drain()
}

/// Run the covenant redeem at `cov_index` in a tx with `sequences` (one per input)
/// and a chosen `lock_time`. All non-covenant inputs are dummy P2SH(OpTrue).
fn run_multi(
    redeem: &[u8],
    lock_time: u64,
    sequences: &[u64],
    cov_index: usize,
) -> Result<(), TxScriptError> {
    let cov_sig = ScriptBuilder::new().add_data(redeem).unwrap().drain();
    let dummy_redeem = ScriptBuilder::new().add_op(OpTrue).unwrap().drain();
    let dummy_sig = ScriptBuilder::new()
        .add_data(&dummy_redeem)
        .unwrap()
        .drain();

    let mut inputs = Vec::new();
    let mut entries = Vec::new();
    for (i, seq) in sequences.iter().enumerate() {
        let (sig, spk) = if i == cov_index {
            (cov_sig.clone(), pay_to_script_hash_script(redeem))
        } else {
            (dummy_sig.clone(), pay_to_script_hash_script(&dummy_redeem))
        };
        inputs.push(TransactionInput::new(
            TransactionOutpoint::new(TransactionId::from_bytes([(i as u8) + 1; 32]), 0),
            sig,
            *seq,
            0,
        ));
        entries.push(UtxoEntry::new(100_000, spk, 0, false, None));
    }
    let output0 = TransactionOutput::new(100_000, ScriptPublicKey::from_vec(0, vec![OpTrue]));
    let tx = Transaction::new(
        1,
        inputs,
        vec![output0],
        lock_time,
        SubnetworkId::from_bytes([0u8; 20]),
        0,
        vec![],
    );
    let reused = SigHashReusedValuesUnsync::new();
    let sig_cache = Cache::new(10_000);
    let cov_input = tx.inputs[cov_index].clone();
    let populated = PopulatedTransaction::new(&tx, entries);
    let cov_ctx = CovenantsContext::from_tx(&populated).map_err(TxScriptError::from)?;
    let utxo = populated.utxo(cov_index).expect("input utxo");
    let mut vm = TxScriptEngine::from_transaction_input(
        &populated,
        &cov_input,
        cov_index,
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

fn run(redeem: &[u8], lock_time: u64, sequence: u64) -> Result<(), TxScriptError> {
    run_multi(redeem, lock_time, &[sequence], 0)
}

// ---- Probe 1: sequence variants ----

#[test]
fn probe_seq_max_minus_one_is_nonfinal_gate_holds() {
    // sequence just below MAX is NON-final → gate active; at deadline it ACCEPTS.
    let r = time_gate_redeem_script(DEADLINE);
    let v = run(&r, DEADLINE as u64, MAX_TX_IN_SEQUENCE_NUM - 1);
    println!("seq=MAX-1 lock=deadline => {v:?}");
    assert!(v.is_ok(), "MAX-1 is non-final, should accept at deadline");
}

#[test]
fn probe_seq_disable_flag_bit_set() {
    // sequence = 1<<63 (the CSV disable-flag bit). It is NOT == u64::MAX, so CLTV
    // treats the input as NON-final → gate stays active. Try an EARLY spend.
    let r = time_gate_redeem_script(DEADLINE);
    let early = run(&r, (DEADLINE as u64) - 1, 1u64 << 63);
    println!("seq=1<<63 lock=deadline-1 (EARLY) => {early:?}");
    assert!(
        early.is_err(),
        "EARLY spend must still reject with disable-flag sequence"
    );
}

// ---- Probe 2: domain confusion ----

#[test]
fn probe_domain_confusion_daa_deadline_unix_locktime() {
    // Committed deadline = 500 (DAA domain). Attacker sets tx.lock_time to a Unix-ms
    // value >= THRESHOLD to try to satisfy stack<=lock numerically while representing
    // a DIFFERENT (earlier real) time. Engine must reject on mismatched types.
    let r = time_gate_redeem_script(DEADLINE);
    let v = run(&r, THRESHOLD + 1, 0);
    println!("DAA deadline vs Unix locktime => {v:?}");
    assert!(
        matches!(v, Err(TxScriptError::UnsatisfiedLockTime(_))),
        "domain mismatch must reject, got {v:?}"
    );
}

#[test]
fn probe_domain_confusion_near_boundary() {
    // Deadline just below threshold (DAA), lock_time exactly at threshold (Unix).
    let near = (THRESHOLD - 1) as i64;
    let r = time_gate_redeem_script(near);
    let v = run(&r, THRESHOLD, 0);
    println!("deadline=THRESHOLD-1 (DAA) lock=THRESHOLD (Unix) => {v:?}");
    assert!(
        matches!(v, Err(TxScriptError::UnsatisfiedLockTime(_))),
        "boundary domain mismatch must reject, got {v:?}"
    );
}

// ---- Probe 3: multi-input, which input's sequence does CLTV check? ----

#[test]
fn probe_multi_input_covenant_nonfinal_other_final_early() {
    // 2 inputs: covenant (index 0) NON-final, other input FINAL. Try EARLY spend.
    // CLTV checks the COVENANT input's own sequence, so non-final → gate active →
    // early spend must reject.
    let r = time_gate_redeem_script(DEADLINE);
    let v = run_multi(&r, (DEADLINE as u64) - 1, &[0, MAX_TX_IN_SEQUENCE_NUM], 0);
    println!("multi: cov nonfinal, other final, EARLY => {v:?}");
    assert!(
        v.is_err(),
        "early spend must reject regardless of other input"
    );
}

#[test]
fn probe_multi_input_covenant_final_bypass_attempt() {
    // THE bypass attempt: make the COVENANT input FINAL (seq=MAX) and a DIFFERENT
    // input non-final, at/after deadline. If CLTV checked "any input non-final"
    // this would PASS and be a bypass. It must instead REJECT (finalized covenant
    // input).
    let r = time_gate_redeem_script(DEADLINE);
    let v = run_multi(&r, DEADLINE as u64, &[MAX_TX_IN_SEQUENCE_NUM, 0], 0);
    println!("multi: cov FINAL, other nonfinal, at deadline => {v:?}");
    assert!(
        matches!(v, Err(TxScriptError::UnsatisfiedLockTime(_))),
        "finalized covenant input must reject (CLTV checks THIS input), got {v:?}"
    );
}

// ---- Probe 4: the core early-spend claim at the txscript layer ----

#[test]
fn probe_core_txscript_accepts_locktime_field_at_deadline_regardless_of_real_time() {
    // CRITICAL SCOPE PROBE. The txscript engine only inspects the tx.lock_time
    // FIELD (spender-chosen) and this input's sequence. It has NO access to the
    // block DAA score. So a spender who simply SETS lock_time = deadline and a
    // non-final sequence is ACCEPTED by txscript — even though, in real consensus,
    // check_tx_is_finalized would still block the tx from a block until DAA score
    // exceeds lock_time. This proves the txscript half ALONE does not enforce
    // "real time has passed"; the consensus finalization rule (NOT exercised by
    // the B1 harness) is a required second half.
    let r = time_gate_redeem_script(DEADLINE);
    let v = run(&r, DEADLINE as u64, 0);
    println!("txscript verdict on lock_time==deadline, nonfinal => {v:?}");
    assert!(
        v.is_ok(),
        "txscript accepts a spender-set lock_time==deadline; real-time enforcement \
         is the SEPARATE consensus finalization rule the harness does not test"
    );
}

// ---- Probe 6: false-reject regression ----

#[test]
fn probe_legit_spend_strictly_after_deadline() {
    let r = time_gate_redeem_script(DEADLINE);
    let v = run(&r, (DEADLINE as u64) + 100, 0);
    println!("legit strictly-after => {v:?}");
    assert!(
        v.is_ok(),
        "a strictly-after spend must not be wrongly rejected"
    );
}

#[test]
fn probe_zero_locktime_finalized_domain() {
    // lock_time == 0 means "finalized" at consensus. In txscript, deadline=500 (DAA)
    // vs lock_time=0 (DAA, < THRESHOLD): domain matches, but stack(500) > lock(0) →
    // reject. So committing deadline>0 cannot be defeated by a zero lock_time.
    let r = time_gate_redeem_script(DEADLINE);
    let v = run(&r, 0, 0);
    println!("lock_time=0 vs deadline=500 => {v:?}");
    assert!(
        matches!(v, Err(TxScriptError::UnsatisfiedLockTime(_))),
        "zero lock_time must reject a positive DAA deadline, got {v:?}"
    );
}
