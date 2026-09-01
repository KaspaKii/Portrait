//! In-repo engine proof for the KTT (KCC20-profile) covenant.
//!
//! Runs the **committed** artifact (`covenant/ktt.compiled.json`) through the
//! real pinned consensus **script VM** — `rusty-kaspa` tag `v2.0.0`, commit
//! `90dbf07` — with `covenants_enabled: true` and a real
//! `CovenantsContext::from_tx`, and checks that KTT-1, KTT-2 and KTT-3 are
//! enforced *by the script VM*, not by this crate's Rust types.
//!
//! Per-state scripts are produced by splicing the state region of the committed
//! artifact ([`kcp_common::covenant`]); the program body is never touched, and
//! `ktt_committed_artifact_matches_script_hex` checks that the artifact's
//! `script` field and the committed `.script.hex` agree byte-for-byte and that
//! splicing state leaves everything outside the state window unchanged. There
//! is no `silverscript-lang` dependency (it would float the engine pin) and no
//! fixture holding key material: each test derives a deterministic keypair from
//! a fixed, never-funded seed and splices the matching `ownerIdentifier` into
//! the state.
//!
//! **Covenant arity.** The committed artifact permits **up to 2** covenant
//! inputs and **up to 2** covenant outputs (`maxCovIns = maxCovOuts = 2`, read
//! directly off the program body: `OpCovInputCount OpDup Op2
//! OpLessThanOrEqual`). Fan-out and merge are therefore *permitted* by this
//! covenant, bounded at 2 — supply conservation, not arity, is what stops a
//! holder minting. The tests below cover the 1→1 and 1→2 shapes; the
//! 2-covenant-**input** shape is not exercised (see `KNOWN-ISSUES.md`).
//!
//! **What is NOT covered.**
//! - *Transaction-level validation.* Only `TxScriptEngine::from_transaction_input`
//!   for input 0 runs here: no transaction mass, no KIP-9 storage mass, no
//!   standardness. The covenant places no floor on the output value, so a caller
//!   must still respect `MIN_CHANGE_SOMPI` and the storage-mass bound itself.
//! - *The live half of `[KCP-KTT-003]`.* Submitting a transaction is not
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

/// `identifierType` 0x00 — the owner identifier is an x-only Schnorr pubkey.
const IDENTIFIER_PUBKEY: u8 = 0x00;

const TOKEN_UTXO_VALUE_SOMPI: u64 = 10_000_000;

/// Placeholder covenant id shared by the input entry and the output bindings —
/// the continuation shape `CovenantsContext::from_tx` reconstructs.
fn covenant_id() -> Hash {
    Hash::from_bytes(*b"KCP-KTT-ENGINE-TEST-COVENANT-I01")
}

/// One KTT covenant state — the exact KCC20 four-field layout.
#[derive(Clone)]
struct State {
    owner_identifier: [u8; 32],
    identifier_type: u8,
    amount: i64,
    is_minter: bool,
}

fn artifact() -> CompiledCovenant {
    let path = format!("{}/covenant/ktt.compiled.json", env!("CARGO_MANIFEST_DIR"));
    CompiledCovenant::load(std::path::Path::new(&path)).expect("load committed covenant artifact")
}

/// The state region as silverc lays it out: fixed-width, explicit pushes.
fn state_region(s: &State) -> Vec<u8> {
    let mut out = Vec::new();
    for field in [
        &s.owner_identifier[..],
        &[s.identifier_type],
        &s.amount.to_le_bytes(),
        &[u8::from(s.is_minter)],
    ] {
        push_state_field(&mut out, field).expect("state field width");
    }
    out
}

/// The `newStates` argument for `transfer`, canonical pushes.
///
/// A `State[]` argument is passed as one push **per field**, holding the
/// concatenated values of that field across all N states — owners `32·N`,
/// identifier types `1·N`, amounts `8·N`, minter flags `1·N`. For N = 1 this
/// degenerates to the four single-value pushes.
fn state_args(states: &[State]) -> Vec<u8> {
    let mut owners = Vec::with_capacity(32 * states.len());
    let mut types = Vec::with_capacity(states.len());
    let mut amounts = Vec::with_capacity(8 * states.len());
    let mut minters = Vec::with_capacity(states.len());
    for s in states {
        owners.extend_from_slice(&s.owner_identifier);
        types.push(s.identifier_type);
        amounts.extend_from_slice(&s.amount.to_le_bytes());
        minters.push(u8::from(s.is_minter));
    }

    let mut builder = ScriptBuilder::with_flags(EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    });
    for field in [&owners, &types, &amounts, &minters] {
        builder.add_data(field).expect("push state argument");
    }
    builder.drain().to_vec()
}

/// The `witnesses` argument: one entry per covenant input, unused for a
/// pubkey-identified owner.
fn witness_args() -> Vec<u8> {
    let mut builder = ScriptBuilder::with_flags(EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    });
    builder.add_data(&[0x00]).expect("push witness argument");
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

/// Build the transfer spending `prev`'s single covenant UTXO into one covenant
/// output per entry of `nexts`, signed by `signer`, and run it through the
/// script VM.
///
/// Version-1 (Toccata) inputs commit a compute budget that the signature
/// covers, so the budget is measured under `u16::MAX` and the input re-signed
/// at the covering budget — the same two-round dance the live deployment does.
fn run_transfer_to(prev: &State, nexts: &[State], signer: &Keypair) -> Result<u64, String> {
    let cov = artifact();
    let prev_script = cov.with_state(&state_region(prev));
    let cov_id = covenant_id();

    let spendable = TOKEN_UTXO_VALUE_SOMPI - CARRIER_FEE_SOMPI;
    let per_output = spendable / nexts.len() as u64;
    let outputs: Vec<TransactionOutput> = nexts
        .iter()
        .enumerate()
        .map(|(i, next)| {
            // The last output absorbs the integer-division remainder.
            let value = if i + 1 == nexts.len() {
                spendable - per_output * (nexts.len() as u64 - 1)
            } else {
                per_output
            };
            let mut out = TransactionOutput::new(
                value,
                p2sh_lock_script(&cov.with_state(&state_region(next))),
            );
            out.covenant = Some(CovenantBinding {
                authorizing_input: 0,
                covenant_id: cov_id,
            });
            out
        })
        .collect();

    let outpoint = TransactionOutpoint::new(TransactionId::from_bytes([0xab; 32]), 0);
    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(outpoint, vec![], 0, 0)],
        outputs,
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let entry = UtxoEntry::new(
        TOKEN_UTXO_VALUE_SOMPI,
        p2sh_lock_script(&prev_script),
        0,
        false,
        Some(cov_id),
    );

    let sigscript_for = |tx: &Transaction| -> Vec<u8> {
        let sighash = p2sh_input_sighash(tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, signer);
        append_signature_script(&state_args(nexts), &sig, &witness_args(), &prev_script)
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

/// The 1-covenant-input → 1-covenant-output shape.
fn run_transfer(prev: &State, next: &State, signer: &Keypair) -> Result<u64, String> {
    run_transfer_to(prev, std::slice::from_ref(next), signer)
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

fn holding(owner: &Keypair, amount: i64) -> State {
    State {
        owner_identifier: x_only(owner),
        identifier_type: IDENTIFIER_PUBKEY,
        amount,
        is_minter: false,
    }
}

/// A non-minter holding of 1_000 tokens held by `from`, and the conserving
/// handoff of the whole balance to `to`.
fn holding_and_handoff(from: &Keypair, to: &Keypair) -> (State, State) {
    (holding(from, 1_000), holding(to, 1_000))
}

/// The committed artifact is internally consistent, and splicing a state
/// rewrites only the state window.
///
/// This proves three things and no more: (1) the `script` field of
/// `ktt.compiled.json` and `ktt.script.hex` are the same bytes, so the two
/// committed representations cannot drift apart; (2) `with_state` rewrites
/// exactly `[state_start, state_start + state_len)` at the artifact's real
/// widths, leaving the program body byte-identical; and (3) [`state_region`]
/// reproduces the artifact's own genesis-template state region byte-for-byte,
/// so this file's encoder agrees with what silverc emitted. It says nothing
/// about what was deployed on-chain.
#[test]
fn ktt_committed_artifact_matches_script_hex() {
    let committed = hex::decode(
        std::fs::read_to_string(format!(
            "{}/covenant/ktt.script.hex",
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

    let (held, _) = holding_and_handoff(&keypair(0x11), &keypair(0x12));
    let spliced = cov.with_state(&state_region(&held));
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

    // The artifact was compiled with an all-zero owner, identifierType 0x00,
    // genesisAmount 1000 and isMinter false; re-encoding that template must
    // reproduce the artifact exactly, which pins this file's field order,
    // widths and push encoding — and independently confirms `genesisAmount`.
    let template = State {
        owner_identifier: [0u8; 32],
        identifier_type: IDENTIFIER_PUBKEY,
        amount: 1_000,
        is_minter: false,
    };
    assert_eq!(
        cov.with_state(&state_region(&template)),
        committed,
        "state_region must reproduce the artifact's own genesis-template region"
    );
}

/// ACCEPT baseline: supply-conserving 1→1 handoff, signed by the owner.
#[test]
fn ktt_engine_accepts_valid_handoff() {
    let alice = keypair(0x21);
    let bob = keypair(0x22);
    let (prev, next) = holding_and_handoff(&alice, &bob);
    run_transfer(&prev, &next, &alice).expect("valid handoff must be ACCEPTED");
}

/// KTT-1: a non-minter transfer must conserve supply (1→1).
#[test]
fn ktt_engine_rejects_amount_inflation() {
    let alice = keypair(0x23);
    let bob = keypair(0x24);
    let (prev, next) = holding_and_handoff(&alice, &bob);
    let mut bad = next.clone();
    bad.amount = prev.amount + 1;
    assert_rejected_for(
        "KTT-1 (amount inflation)",
        run_transfer(&prev, &bad, &alice),
        run_transfer(&prev, &next, &alice),
    );
}

/// KTT-2: a non-minter input cannot produce a minter output.
#[test]
fn ktt_engine_rejects_minter_escalation() {
    let alice = keypair(0x25);
    let bob = keypair(0x26);
    let (prev, next) = holding_and_handoff(&alice, &bob);
    let mut bad = next.clone();
    bad.is_minter = true;
    assert_rejected_for(
        "KTT-2 (minter escalation)",
        run_transfer(&prev, &bad, &alice),
        run_transfer(&prev, &next, &alice),
    );
}

/// KTT-3: the owner named in the input state must authorise the transfer.
#[test]
fn ktt_engine_rejects_wrong_signature() {
    let alice = keypair(0x27);
    let bob = keypair(0x28);
    let impostor = keypair(0x29);
    let (prev, next) = holding_and_handoff(&alice, &bob);
    assert_rejected_for(
        "KTT-3 (wrong signer)",
        run_transfer(&prev, &next, &impostor),
        run_transfer(&prev, &next, &alice),
    );
}

/// The control that makes the KTT-2 negative meaningful: a *minter* input may
/// legitimately carry `isMinter` forward and change supply, so the escalation
/// rejection above is the rule firing and not a mis-encoded `true`.
#[test]
fn ktt_engine_accepts_minter_changing_supply() {
    let alice = keypair(0x31);
    let bob = keypair(0x32);
    let (mut prev, mut next) = holding_and_handoff(&alice, &bob);
    prev.is_minter = true;
    next.is_minter = true;
    next.amount = 5_000;
    run_transfer(&prev, &next, &alice).expect("a minter may mint: must be ACCEPTED");
}

// --- 1 covenant input → 2 covenant outputs -------------------------------
//
// KTT-1 is a SUM over the covenant outputs. On the 1→1 shape it degenerates to
// `a == b`, which cannot distinguish "the sum is checked" from "the single
// amount is copied". The committed artifact permits up to 2 covenant outputs,
// so the split shape is the one that actually exercises the rule.

/// ACCEPT: a conserving split — 1000 → 400 + 600 across two covenant outputs.
#[test]
fn ktt_engine_accepts_conserving_split() {
    let alice = keypair(0x41);
    let bob = keypair(0x42);
    let carol = keypair(0x43);
    let prev = holding(&alice, 1_000);
    let nexts = [holding(&bob, 400), holding(&carol, 600)];
    run_transfer_to(&prev, &nexts, &alice).expect("conserving 1→2 split must be ACCEPTED");
}

/// KTT-1 on the shape it exists for: the outputs must SUM to the input.
/// 400 + 601 > 1000 mints a token out of nothing and must be rejected.
#[test]
fn ktt_engine_rejects_inflating_split() {
    let alice = keypair(0x44);
    let bob = keypair(0x45);
    let carol = keypair(0x46);
    let prev = holding(&alice, 1_000);
    let good = [holding(&bob, 400), holding(&carol, 600)];
    let bad = [holding(&bob, 400), holding(&carol, 601)];
    assert_rejected_for(
        "KTT-1 (inflating 1→2 split)",
        run_transfer_to(&prev, &bad, &alice),
        run_transfer_to(&prev, &good, &alice),
    );
}

/// KTT-2 across a split: `isMinter` on *any* covenant output of a non-minter
/// input is an escalation, including the second one.
#[test]
fn ktt_engine_rejects_minter_escalation_on_split() {
    let alice = keypair(0x47);
    let bob = keypair(0x48);
    let carol = keypair(0x49);
    let prev = holding(&alice, 1_000);
    let good = [holding(&bob, 400), holding(&carol, 600)];
    let mut bad = good.clone();
    bad[1].is_minter = true;
    assert_rejected_for(
        "KTT-2 (minter escalation on the second split output)",
        run_transfer_to(&prev, &bad, &alice),
        run_transfer_to(&prev, &good, &alice),
    );
}

/// The one link in this file that reaches the **deployed** script.
///
/// The live `KCP-KTT-003` deployment's engine preflight is recorded in the evidence
/// register as consuming exactly `111410` script units. Script units are a
/// deterministic function of the executed opcode trace and the operand widths,
/// so re-running the *committed* artifact here and consuming the same count
/// corroborates that the deployed program body is this program body.
///
/// **This is corroboration, not a binding.** The count is insensitive to the
/// state *values* (the fields are fixed-width), the recorded number is itself
/// out-of-repo, and the on-chain `covenant_id` is derived from the funding
/// outpoint rather than the script — so it cannot be recomputed here. See
/// `docs/EVIDENCE.md`.
const LIVE_PREFLIGHT_SCRIPT_UNITS: u64 = 111410;

#[test]
fn ktt_engine_cost_matches_recorded_live_preflight() {
    {
        let alice = keypair(0x51);
        let bob = keypair(0x52);
        let (prev, next) = holding_and_handoff(&alice, &bob);
        let used = run_transfer(&prev, &next, &alice).expect("valid handoff must be ACCEPTED");
        assert_eq!(
            used, LIVE_PREFLIGHT_SCRIPT_UNITS,
            "committed artifact no longer costs what the live KCP-KTT-003 preflight cost"
        );
    }
}
