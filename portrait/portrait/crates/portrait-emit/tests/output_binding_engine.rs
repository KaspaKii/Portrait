//! B2 output-binding: PROVE the emitted `pays(...)` binding is real against the
//! pinned Toccata engine (rusty-kaspa tag `v2.0.0` = commit `90dbf07`).
//!
//! `portrait-emit` lowers a `pays(index, payee, amount)` clause to two
//! output-introspection `require`s in the emitted `.sil` (see
//! `emit_lowers_pays_guard_to_output_introspection_requires` in the unit tests):
//!
//! ```silverscript
//! require(tx.outputs[k].value == <committed amount>);
//! require(tx.outputs[k].scriptPubKey == byte[](new ScriptPubKeyP2PK(<committed payee>)));
//! ```
//!
//! silverc lowers those to the opcode form reconstructed below:
//!   * `tx.outputs[k].value`        → `OpTxOutputAmount` — the engine pushes the
//!     output value as a MINIMALLY-ENCODED SCRIPT NUMBER (via `push_number`), so
//!     the committed amount is compared as an `int` (byte-identical encoding).
//!   * `tx.outputs[k].scriptPubKey` → `OpTxOutputSpk` — the engine pushes the FULL
//!     serialized spk: `version.to_be_bytes()` (2 bytes) `|| script`. The payee's
//!     P2PK spk is reconstructed with the exact `ScriptPubKeyP2PK` opcode sequence
//!     silverc emits (`00 00 <OpData32> <pubkey> <OpCheckSig>`) and compared.
//!
//! This is the DECISIVE test that lifts B2 above opcode-presence: it drives the
//! real `TxScriptEngine` and shows a genuine ACCEPT / REJECT pair. The binding is
//! isolated as a P2SH redeem script (no covenant-id / signature machinery — those
//! belong to other slices), so a pass/fail here is attributable to the output
//! binding alone.
//!
//! Byte-encoding decision (recorded): amount = script number (compared as `int`);
//! payee = full spk `version_be || script`, P2PK reconstructed in-script. No
//! `OpNum2Bin` padding is needed — `OpTxOutputAmount` already yields the minimal
//! script-number form that a committed `int` push matches exactly.

use kaspa_consensus_core::hashing::sighash::SigHashReusedValuesUnsync;
use kaspa_consensus_core::subnets::SubnetworkId;
use kaspa_consensus_core::tx::{
    PopulatedTransaction, ScriptPublicKey, Transaction, TransactionId, TransactionInput,
    TransactionOutpoint, TransactionOutput, UtxoEntry, VerifiableTransaction,
};
use kaspa_txscript::caches::Cache;
use kaspa_txscript::covenants::CovenantsContext;
use kaspa_txscript::opcodes::codes::{
    OpBlake2b, OpCat, OpCheckSig, OpData32, OpEqual, OpEqualVerify, OpSwap, OpTrue,
    OpTxOutputAmount, OpTxOutputSpk,
};
use kaspa_txscript::script_builder::ScriptBuilder;
use kaspa_txscript::{pay_to_script_hash_script, EngineCtx, EngineFlags, TxScriptEngine};
use kaspa_txscript_errors::TxScriptError;

/// A committed amount large enough to force `add_i64`'s data-push path (values
/// 0..=16 take an OpN fast path), so the pushed bytes are byte-identical to the
/// engine's `push_number` encoding of the output value.
const COMMITTED_AMOUNT: i64 = 12_345;
const OUTPUT_INDEX: i64 = 0;

/// The redeem script mirroring the emitted `pays(0, payee, amount)` lowering:
/// the two output-introspection requires, then `OpTrue` so the P2SH spend
/// succeeds when both hold.
fn binding_redeem_script(index: i64, payee_pubkey: &[u8; 32], amount: i64) -> Vec<u8> {
    let mut b = ScriptBuilder::new();
    // require(tx.outputs[index].value == amount)
    b.add_i64(index).unwrap();
    b.add_op(OpTxOutputAmount).unwrap();
    b.add_i64(amount).unwrap();
    b.add_op(OpEqualVerify).unwrap();
    // require(tx.outputs[index].scriptPubKey == byte[](new ScriptPubKeyP2PK(payee)))
    // LHS: full serialized spk of output[index].
    b.add_i64(index).unwrap();
    b.add_op(OpTxOutputSpk).unwrap();
    // RHS: reconstruct the P2PK spk exactly as silverc's ScriptPubKeyP2PK does —
    // push pubkey; push [00 00 OpData32]; swap; cat; push [OpCheckSig]; cat —
    // yielding `00 00 <OpData32> <pubkey> <OpCheckSig>` = version(0) || script.
    b.add_data(payee_pubkey).unwrap();
    b.add_data(&[0x00, 0x00, OpData32]).unwrap();
    b.add_op(OpSwap).unwrap();
    b.add_op(OpCat).unwrap();
    b.add_data(&[OpCheckSig]).unwrap();
    b.add_op(OpCat).unwrap();
    b.add_op(OpEqualVerify).unwrap();
    b.add_op(OpTrue).unwrap();
    b.drain()
}

/// The full serialized P2PK spk for `pubkey`, as `OpTxOutputSpk` would return it:
/// version 0 (`00 00`) || script (`OpData32 <pubkey> OpCheckSig`).
fn p2pk_script_public_key(pubkey: &[u8; 32]) -> ScriptPublicKey {
    let mut script = vec![OpData32];
    script.extend_from_slice(pubkey);
    script.push(OpCheckSig);
    ScriptPublicKey::from_vec(0, script)
}

/// Run `redeem` as a P2SH input, spending a tx whose `output[0]` is `output`.
fn run_binding(redeem: &[u8], output0: TransactionOutput) -> Result<(), TxScriptError> {
    let sig_script = ScriptBuilder::new().add_data(redeem).unwrap().drain();
    let input = TransactionInput::new(
        TransactionOutpoint::new(TransactionId::from_bytes([1; 32]), 0),
        sig_script,
        0,
        0,
    );
    let utxo_spk = pay_to_script_hash_script(redeem);
    let entries = vec![UtxoEntry::new(100_000, utxo_spk, 0, false, None)];
    let tx = Transaction::new(
        1,
        vec![input],
        vec![output0],
        0,
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
fn engine_accepts_output_that_matches_the_committed_amount_and_payee() {
    let payee = [0x02u8; 32];
    let redeem = binding_redeem_script(OUTPUT_INDEX, &payee, COMMITTED_AMOUNT);
    let output0 = TransactionOutput::new(COMMITTED_AMOUNT as u64, p2pk_script_public_key(&payee));
    run_binding(&redeem, output0).expect("engine must ACCEPT the matching output binding");
}

#[test]
fn engine_rejects_output_with_a_different_amount() {
    let payee = [0x02u8; 32];
    let redeem = binding_redeem_script(OUTPUT_INDEX, &payee, COMMITTED_AMOUNT);
    // Correct payee, WRONG amount → the value require must fail.
    let output0 = TransactionOutput::new(999, p2pk_script_public_key(&payee));
    let err =
        run_binding(&redeem, output0).expect_err("engine must REJECT a mismatched output amount");
    assert!(
        matches!(err, TxScriptError::VerifyError | TxScriptError::EvalFalse),
        "expected a verify/eval-false rejection, got {err:?}"
    );
}

#[test]
fn engine_rejects_output_paying_a_different_payee() {
    let payee = [0x02u8; 32];
    let attacker = [0x03u8; 32];
    let redeem = binding_redeem_script(OUTPUT_INDEX, &payee, COMMITTED_AMOUNT);
    // Correct amount, WRONG payee spk → the scriptPubKey require must fail.
    let output0 =
        TransactionOutput::new(COMMITTED_AMOUNT as u64, p2pk_script_public_key(&attacker));
    let err = run_binding(&redeem, output0)
        .expect_err("engine must REJECT a payment to a different payee");
    assert!(
        matches!(err, TxScriptError::VerifyError | TxScriptError::EvalFalse),
        "expected a verify/eval-false rejection, got {err:?}"
    );
}

// ── B3 TERMINAL SPEND: zero covenant-successor outputs ──────────────────────
//
// A terminal release (B3) RELEASES the coin and CONSUMES the UTXO — the spending
// transaction produces ZERO covenant-successor outputs; its only output is the
// committed payee's. The emitted `.sil` is a `binding = auth` verification
// function carrying the same two pays requires as the non-terminal path (only
// the state accessor differs, `prev_state` vs `prev_states[0]`), so the ISOLATED
// output-binding opcodes are byte-identical to the B2 redeem exercised above.
//
// These tests drive the pinned `TxScriptEngine` on a P2SH spend whose spending tx
// has EXACTLY ONE output (the payee) and NO covenant successor — the terminal
// shape — and assert ACCEPT for the committed payee / REJECT for a wrong payee.
//
// HONEST SCOPE (isolated-opcode, same as B2):
//   PROVABLE on the mandated v2.0.0 pin (asserted here) = the isolated pays-bound
//   TERMINAL spend producing zero successors accepts the committed payee and
//   rejects a wrong payee; plus the composed terminal `Escrow.sil` compiling under
//   silverc (`silverc_accepts_the_composed_escrow_sil_with_the_pays_binding`).
//   PENDING (same pin bucket as B2/B1, NOT fabricated) = the COMPOSED on-engine
//   terminal spend, including the runtime covenant semantics that `binding = auth`
//   + `to = 1` ADMITS a spend with 0 covenant-successor outputs — assembling that
//   needs silverscript-lang's covenant sig-script/ABI, which pins a floating
//   engine branch incompatible with the mandated `v2.0.0` pin. See
//   library/ENFORCEMENT.md.

#[test]
fn engine_accepts_terminal_release_paying_the_committed_payee_with_zero_successors() {
    // Terminal release: the spending tx has ONE output — the committed payee — and
    // NO covenant successor. The pays-bound redeem must ACCEPT it.
    let payee = [0x02u8; 32];
    let redeem = binding_redeem_script(OUTPUT_INDEX, &payee, COMMITTED_AMOUNT);
    let output0 = TransactionOutput::new(COMMITTED_AMOUNT as u64, p2pk_script_public_key(&payee));
    run_binding(&redeem, output0)
        .expect("engine must ACCEPT a terminal release paying the committed payee (0 successors)");
}

#[test]
fn engine_rejects_terminal_release_paying_a_different_payee() {
    // Terminal release paying a WRONG payee (still zero successors) must be
    // REJECTED — the coin cannot be redirected away from the committed payee.
    let payee = [0x02u8; 32];
    let attacker = [0x03u8; 32];
    let redeem = binding_redeem_script(OUTPUT_INDEX, &payee, COMMITTED_AMOUNT);
    let output0 =
        TransactionOutput::new(COMMITTED_AMOUNT as u64, p2pk_script_public_key(&attacker));
    let err = run_binding(&redeem, output0)
        .expect_err("engine must REJECT a terminal release to a different payee");
    assert!(
        matches!(err, TxScriptError::VerifyError | TxScriptError::EvalFalse),
        "expected a verify/eval-false rejection, got {err:?}"
    );
}

// ── Composed-covenant + silverc golden checks (M1a / M2) ────────────────────
//
// The engine tests above isolate the output-binding OPCODES (P2SH redeem) so a
// pass/fail is attributable to the binding alone. These two tests tie that back
// to the REAL emitted, COMPOSED terminal covenant. A composed end-to-end
// on-engine SPEND (valid signature + covenant runtime admitting 0 successors) is
// NOT asserted here: assembling it needs silverscript-lang's covenant
// sig-script/ABI, which pins a floating pre-release engine branch — incompatible
// with the mandated `v2.0.0` pin. See library/ENFORCEMENT.md for the honest scope.

/// Absolute path to a file under a repo `library/<pattern_dir>/` dir.
fn library_artifact(pattern_dir: &str, name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../library")
        .join(pattern_dir)
        .join(name)
}

/// Absolute path to a file under the repo `library/finance/escrow/` dir.
fn escrow_artifact(name: &str) -> std::path::PathBuf {
    library_artifact("finance/escrow", name)
}

/// Locate the pinned `silverc`: PATH first, then `$HOME/.cargo/bin/silverc`
/// (mirrors portrait-cli's `golden.rs`).
fn find_silverc() -> Option<std::path::PathBuf> {
    if std::process::Command::new("silverc")
        .arg("--version")
        .output()
        .is_ok()
    {
        return Some(std::path::PathBuf::from("silverc"));
    }
    if let Ok(home) = std::env::var("HOME") {
        let pinned = std::path::Path::new(&home).join(".cargo/bin/silverc");
        if pinned.exists() {
            return Some(pinned);
        }
    }
    None
}

#[test]
fn silverc_accepts_the_composed_escrow_sil_with_the_pays_binding() {
    // M1(a): the COMPOSED TERMINAL covenant (B3: `binding = auth`, NO successor,
    // the two pays requires) must compile under the pinned silverc (exit 0) —
    // proving the terminal release/refund spend composes with the rest of the
    // emitted covenant, not just in an isolated fragment.
    let Some(silverc) = find_silverc() else {
        eprintln!(
            "SKIP[Escrow]: silverc not found on PATH nor at $HOME/.cargo/bin — \
             composed-compile check skipped (NOT silently passed)."
        );
        return;
    };
    let sil = escrow_artifact("Escrow.sil");
    let ctor = escrow_artifact("Escrow_ctor.json");
    let output = std::process::Command::new(&silverc)
        .arg(&sil)
        .arg("--ctor")
        .arg(&ctor)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn silverc ({silverc:?}): {e}"));
    // Sanity: the emitted .sil must carry the TERMINAL binding — the two pays
    // requires read through the SINGULAR `prev_state` accessor, `binding = auth`,
    // and NO `binding = cov` / NO `return(` (the coin is released, not carried).
    let sil_text = std::fs::read_to_string(&sil).expect("read Escrow.sil");
    assert!(
        sil_text.contains("require(tx.outputs[0].value == prev_state.amount);")
            && sil_text.contains(
                "require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_state.seller)));"
            ),
        "the composed Escrow.sil under test must carry the terminal pays binding requires"
    );
    assert!(
        sil_text.contains("binding = auth")
            && !sil_text.contains("binding = cov")
            && !sil_text.contains("return("),
        "the composed Escrow.sil must be a terminal `binding = auth` spend with no successor return"
    );
    assert!(
        output.status.success(),
        "silverc rejected the composed Escrow.sil (exit {:?}).\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Parse the `"script": [ .. ]` u8 array out of a silverc compiled artifact.
fn read_compiled_script(path: &std::path::Path) -> Vec<u8> {
    let text = std::fs::read_to_string(path).expect("read compiled artifact");
    let start = text
        .find("\"script\"")
        .expect("artifact has a script field");
    let open = text[start..].find('[').expect("script array open") + start;
    let close = text[open..].find(']').expect("script array close") + open;
    text[open + 1..close]
        .split(',')
        .map(|t| t.trim().parse::<u8>().expect("script byte"))
        .collect()
}

#[test]
fn scriptpubkeyp2pk_lowering_matches_our_reconstruction_golden() {
    // M2: the RHS spk in the emitted binding is reconstructed as
    // `<pubkey> push[00 00 OpData32] OpSwap OpCat push[OpCheckSig] OpCat`. Assert
    // that EXACT constant subsequence (everything after the pubkey push, which is
    // computed from prev_states, not a literal) appears in silverc's REAL compiled
    // Escrow.json — i.e. our hand-reconstruction in `binding_redeem_script` is
    // byte-identical to silverc's `ScriptPubKeyP2PK` lowering, not merely assumed.
    // Bytes: OpData3(0x03) 00 00 OpData32(0x20) OpSwap(0x7c) OpCat(0x7e)
    //        OpData1(0x01) OpCheckSig(0xac) OpCat(0x7e).
    let expected: [u8; 9] = [
        0x03, 0x00, 0x00, OpData32, OpSwap, OpCat, 0x01, OpCheckSig, OpCat,
    ];
    let script = read_compiled_script(&escrow_artifact("Escrow.json"));
    let found = script.windows(expected.len()).any(|w| w == expected);
    assert!(
        found,
        "silverc's ScriptPubKeyP2PK lowering ({expected:02x?}) not found in the \
         compiled Escrow.json — the emitter/engine-test reconstruction has drifted \
         from silverc"
    );
}

// ── NON-TERMINAL pays: the Subscription charge payout (Portrait item 4) ─────
//
// `finance/Subscription`'s `charge` is the catalogue's FIRST NON-TERMINAL
// `pays(...)`: the same spend carries BOTH a covenant successor (`to = 1`) AND a
// separate bound payee output at index 1. That is well-formed at COMPILE time —
// silverc's `to` counts covenant SUCCESSOR outputs, not total tx outputs — and
// this test proves it (exit 0) on the composed, emitted covenant.
//
// HONEST SCOPE — see KNOWN-ISSUES.md KI-3: WHICH output index the covenant
// successor occupies at RUNTIME is UNVERIFIED on the mandated `v2.0.0` pin (the
// same composed-on-engine-spend bucket as KI-1). The output-binding OPCODES this
// lowers to are the ones already proven accept/reject above; what is NOT proven
// is the successor/payee output-index co-existence under the covenant runtime.

#[test]
fn silverc_accepts_the_composed_subscription_sil_with_the_non_terminal_pays_binding() {
    let Some(silverc) = find_silverc() else {
        eprintln!(
            "SKIP[Subscription]: silverc not found on PATH nor at $HOME/.cargo/bin — \
             composed-compile check skipped (NOT silently passed)."
        );
        return;
    };
    let sil = library_artifact("finance/subscription", "Subscription.sil");
    let ctor = library_artifact("finance/subscription", "Subscription_ctor.json");
    let sil_text = std::fs::read_to_string(&sil).expect("read Subscription.sil");
    // The payout binding is at output index 1, NOT 0: index 0 is left to the
    // covenant successor, so the bound payee output must not collide with it.
    assert!(
        sil_text
            .contains("require(tx.outputs[1].value == prev_states[0].amount_per_period);")
            && sil_text.contains(
                "require(tx.outputs[1].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_states[0].provider)));"
            ),
        "Subscription.sil must carry the output[1] payout binding to the committed provider"
    );
    // NON-TERMINAL: this spend still produces a covenant successor, so it must be
    // a `binding = cov` transition WITH a return — not the B3 terminal shape.
    assert!(
        sil_text.contains("binding = cov")
            && sil_text.contains("mode = transition")
            && sil_text.contains("return("),
        "Subscription.sil must remain a NON-TERMINAL covenant transition with a successor return"
    );
    let output = std::process::Command::new(&silverc)
        .arg(&sil)
        .arg("--ctor")
        .arg(&ctor)
        .arg("-c")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn silverc ({silverc:?}): {e}"));
    assert!(
        output.status.success(),
        "silverc rejected the composed Subscription.sil (exit {:?}).\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── P2SH payee: the type-directed spk dispatch (Portrait item 3) ────────────
//
// A `pays(k, payee, amount)` whose `payee` is declared `byte[32]` is a committed
// SCRIPT HASH, and the emitter lowers it with `ScriptPubKeyP2SH` instead of
// `ScriptPubKeyP2PK`. silverc builds that spk as
// `version(00 00) || OpBlake2b || OpData32 || <hash> || OpEqual` — exactly
// rusty-kaspa's `pay_to_script_hash` layout. The triple below mirrors the P2PK
// triple: accept the committed script hash, reject a different one, and pin our
// hand-reconstruction against silverc's REAL compiled bytes.
//
// WHAT THIS BUYS (do not overclaim): it does NOT unblock any currently-deferred
// catalogue pattern — the int-balance-payee patterns have no committed payee at
// all, and the spender-arg-amount group is blocked on the AMOUNT, not the payee.
// It removes a live FOOTGUN (M4): an Escrow/ArbiterEscrow instantiated for a
// P2SH/multisig seller previously had a permanently DEAD `release` path, with no
// way to express the working one; and it is the prerequisite for multisig-payee
// patterns.

/// A committed redeem-script hash standing in for a P2SH payee.
const COMMITTED_SCRIPT_HASH: [u8; 32] = [0x07; 32];

/// The redeem script mirroring the emitted `pays(0, <byte[32] payee>, amount)`
/// lowering: the amount require, then the P2SH spk require, then `OpTrue`.
fn p2sh_binding_redeem_script(index: i64, script_hash: &[u8; 32], amount: i64) -> Vec<u8> {
    let mut b = ScriptBuilder::new();
    // require(tx.outputs[index].value == amount)
    b.add_i64(index).unwrap();
    b.add_op(OpTxOutputAmount).unwrap();
    b.add_i64(amount).unwrap();
    b.add_op(OpEqualVerify).unwrap();
    // require(tx.outputs[index].scriptPubKey == byte[](new ScriptPubKeyP2SH(hash)))
    // LHS: full serialized spk of output[index].
    b.add_i64(index).unwrap();
    b.add_op(OpTxOutputSpk).unwrap();
    // RHS: reconstruct the P2SH spk exactly as silverc's ScriptPubKeyP2SH does —
    // push hash; push [00 00]; push [OpBlake2b]; cat; push [OpData32]; cat; swap;
    // cat; push [OpEqual]; cat — yielding
    // `00 00 <OpBlake2b> <OpData32> <hash> <OpEqual>` = version(0) || script.
    b.add_data(script_hash).unwrap();
    b.add_data(&[0x00, 0x00]).unwrap();
    b.add_data(&[OpBlake2b]).unwrap();
    b.add_op(OpCat).unwrap();
    b.add_data(&[OpData32]).unwrap();
    b.add_op(OpCat).unwrap();
    b.add_op(OpSwap).unwrap();
    b.add_op(OpCat).unwrap();
    b.add_data(&[OpEqual]).unwrap();
    b.add_op(OpCat).unwrap();
    b.add_op(OpEqualVerify).unwrap();
    b.add_op(OpTrue).unwrap();
    b.drain()
}

/// The full serialized P2SH spk for `script_hash`, as `OpTxOutputSpk` would
/// return it: version 0 (`00 00`) || `OpBlake2b OpData32 <hash> OpEqual` — the
/// same layout rusty-kaspa's `pay_to_script_hash` builds.
fn p2sh_script_public_key(script_hash: &[u8; 32]) -> ScriptPublicKey {
    let mut script = vec![OpBlake2b, OpData32];
    script.extend_from_slice(script_hash);
    script.push(OpEqual);
    ScriptPublicKey::from_vec(0, script)
}

#[test]
fn engine_accepts_output_paying_the_committed_script_hash() {
    let redeem = p2sh_binding_redeem_script(OUTPUT_INDEX, &COMMITTED_SCRIPT_HASH, COMMITTED_AMOUNT);
    let output0 = TransactionOutput::new(
        COMMITTED_AMOUNT as u64,
        p2sh_script_public_key(&COMMITTED_SCRIPT_HASH),
    );
    run_binding(&redeem, output0)
        .expect("engine must ACCEPT the output paying the committed P2SH payee");
}

#[test]
fn engine_rejects_output_paying_a_different_script_hash() {
    let attacker_hash = [0x08u8; 32];
    let redeem = p2sh_binding_redeem_script(OUTPUT_INDEX, &COMMITTED_SCRIPT_HASH, COMMITTED_AMOUNT);
    // Correct amount, WRONG script hash → the scriptPubKey require must fail.
    let output0 = TransactionOutput::new(
        COMMITTED_AMOUNT as u64,
        p2sh_script_public_key(&attacker_hash),
    );
    let err = run_binding(&redeem, output0)
        .expect_err("engine must REJECT a payment to a different script hash");
    assert!(
        matches!(err, TxScriptError::VerifyError | TxScriptError::EvalFalse),
        "expected a verify/eval-false rejection, got {err:?}"
    );
}

#[test]
fn scriptpubkeyp2sh_lowering_matches_our_reconstruction_golden() {
    // LOAD-BEARING: without this, the P2SH bytes in `p2sh_binding_redeem_script`
    // are only ever asserted against themselves. Here the EMITTER produces a .sil
    // with a `byte[32]` payee, the REAL silverc compiles it, and the constant
    // subsequence our reconstruction emits (everything after the hash push, which
    // is computed from prev_states rather than a literal) must appear verbatim in
    // silverc's compiled script.
    // Bytes: OpData2(0x02) 00 00 OpData1(0x01) OpBlake2b(0xaa) OpCat(0x7e)
    //        OpData1(0x01) OpData32(0x20) OpCat(0x7e) OpSwap(0x7c) OpCat(0x7e)
    //        OpData1(0x01) OpEqual(0x87) OpCat(0x7e).
    let expected: [u8; 14] = [
        0x02, 0x00, 0x00, 0x01, OpBlake2b, OpCat, 0x01, OpData32, OpCat, OpSwap, OpCat, 0x01,
        OpEqual, OpCat,
    ];

    // Sanity: that constant IS what our engine-test reconstruction emits, so the
    // golden below pins the reconstruction and not a hand-copied literal.
    let ours = p2sh_binding_redeem_script(OUTPUT_INDEX, &COMMITTED_SCRIPT_HASH, COMMITTED_AMOUNT);
    assert!(
        ours.windows(expected.len()).any(|w| w == expected),
        "the engine-test P2SH reconstruction does not contain the constant subsequence under test"
    );

    let Some(silverc) = find_silverc() else {
        eprintln!(
            "SKIP[P2SH golden]: silverc not found on PATH nor at $HOME/.cargo/bin — \
             ScriptPubKeyP2SH golden skipped (NOT silently passed)."
        );
        return;
    };
    let (sil, ctor_name, ctor_json, name) = emit_p2sh_payee_covenant();
    let dir = std::env::temp_dir().join(format!("portrait-p2sh-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let sil_path = dir.join(format!("{name}.sil"));
    let ctor_path = dir.join(ctor_name);
    let out_path = dir.join(format!("{name}.json"));
    std::fs::write(&sil_path, &sil).expect("write sil");
    std::fs::write(&ctor_path, ctor_json).expect("write ctor");
    assert!(
        sil.contains("ScriptPubKeyP2SH"),
        "the emitted covenant under golden must use the P2SH spk builtin:\n{sil}"
    );
    let output = std::process::Command::new(&silverc)
        .arg(&sil_path)
        .arg("--ctor")
        .arg(&ctor_path)
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn silverc ({silverc:?}): {e}"));
    assert!(
        output.status.success(),
        "silverc rejected the emitted P2SH-payee covenant (exit {:?}).\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let script = read_compiled_script(&out_path);
    assert!(
        script.windows(expected.len()).any(|w| w == expected),
        "silverc's ScriptPubKeyP2SH lowering ({expected:02x?}) not found in the compiled \
         artifact — the emitter/engine-test reconstruction has drifted from silverc"
    );
}

/// Emit a minimal covenant whose `pays(0, seller_script, amount)` payee is a
/// committed `byte[32]` script hash, returning `(sil, ctor_name, ctor_json, name)`.
fn emit_p2sh_payee_covenant() -> (String, String, String, String) {
    use portrait_ir::{CovenantModel, Guard, Mode, Transition};
    use portrait_syntax::{Stmt, Type};

    let model = CovenantModel {
        name: "P2shPayee".into(),
        params: vec![
            ("seller_script".into(), Type::Bytes32),
            ("amount".into(), Type::Coin),
        ],
        state: vec![
            ("seller_script".into(), Type::Bytes32),
            ("amount".into(), Type::Coin),
        ],
        transitions: vec![Transition {
            entry: "release".into(),
            from: "live".into(),
            to: Some("live".into()),
            mode: Mode::Transition,
            guards: vec![Guard::OutputPays {
                index: 0,
                to: "seller_script".into(),
                amount: "amount".into(),
            }],
            capability: None,
            args: vec![],
            body: vec![
                Stmt::Pays {
                    index: 0,
                    payee: "seller_script".into(),
                    amount: "amount".into(),
                },
                Stmt::Return(
                    portrait_syntax::parse_return_expr(
                        "P2shPayee { seller_script: seller_script, amount: amount }",
                    )
                    .expect("parse return"),
                ),
            ],
        }],
        has_vprog: false,
    };
    let files =
        portrait_emit::emit(std::slice::from_ref(&model)).expect("emit P2SH-payee covenant");
    let (ctor_name, ctor_json) = portrait_emit::emit_ctor(&model);
    (
        files[0].source.clone(),
        ctor_name,
        ctor_json,
        model.name.clone(),
    )
}
