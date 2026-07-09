//! Regression harness for the Portrait compiler pipeline (Phase E + §4.2).
//!
//! Three complementary layers, run against the *stable* examples
//! (`examples/counter.portrait` and `tier3-demo/ComplianceToken.portrait`):
//!
//! 1. GOLDEN — drive the real library pipeline
//!    (`portrait_syntax::parse → portrait_sema::check → portrait_ir::lower →
//!     portrait_project::project → portrait_emit::emit`) and assert the emitted
//!    `.sil` contains the load-bearing anchors (pragma, contract decl, the
//!    entrypoint name, the lowered `return(...)`). Substring assertions only —
//!    we do NOT over-fit whitespace, so cosmetic emitter tweaks won't churn.
//!
//! 2. DIFFERENTIAL (§4.2) — write each emitted `.sil` + a generated CTOR JSON to
//!    a temp dir and invoke the *real* `silverc` binary. Exit 0 is asserted.
//!    This makes "the emitter targets the real silverscript syntax" a
//!    continuously-checked invariant rather than a one-time manual claim.
//!    If `silverc` is genuinely absent the differential test is skipped *with a
//!    clear message* — it never silently passes.
//!
//! 3. REJECT — assert malformed programs fail at `parse` or `sema::check`.
//!
//! These call the library crates directly (the established pattern; see the
//! sibling `tests/build.rs`, which exercises the CLI binary instead).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use portrait_emit::emit_ctor;
use portrait_ir::CovenantModel;

/// The two stable example sources, embedded at compile time so the test is
/// hermetic w.r.t. the working directory.
const COUNTER_SRC: &str = include_str!("../../../../examples/counter.portrait");
const COMPLIANCE_TOKEN_SRC: &str =
    include_str!("../../../../examples/tier3-demo/ComplianceToken.portrait");
/// Allocation companion to ComplianceToken: a two-layer source whose vProg
/// entrypoint (`tally`) holds a REAL out-of-subset construct (a `for` loop). It
/// pins that the same construct is a covenant rejection (LoopAirdrop) but an
/// accepted vProg body — i.e. allocation is now layer-aware.
const HEAVY_AIRDROP_SRC: &str =
    include_str!("../../../../examples/tier3-demo/HeavyAirdrop.portrait");

/// Run the real library pipeline end-to-end, returning the emitted covenant
/// models paired with their `.sil` source. Panics with a clear message on any
/// pipeline error so a regression surfaces at the failing stage.
fn run_pipeline(label: &str, src: &str) -> Vec<(CovenantModel, String)> {
    let program =
        portrait_syntax::parse(src).unwrap_or_else(|e| panic!("[{label}] parse failed: {e}"));
    portrait_sema::check(&program).unwrap_or_else(|ds| {
        let msgs: Vec<_> = ds.into_iter().map(|d| d.message).collect();
        panic!(
            "[{label}] sema::check rejected a stable example: {}",
            msgs.join("; ")
        );
    });
    let cartoon = portrait_ir::lower(&program);
    let models = portrait_project::project(&cartoon);
    let sil_files =
        portrait_emit::emit(&models).unwrap_or_else(|e| panic!("[{label}] emit failed: {e}"));
    assert_eq!(
        models.len(),
        sil_files.len(),
        "[{label}] model/sil count mismatch"
    );
    models
        .into_iter()
        .zip(sil_files)
        .map(|(m, s)| (m, s.source))
        .collect()
}

/// A fresh, unique temp working directory for differential compilation.
fn temp_workdir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    dir.push(format!("portrait-golden-{tag}-{stamp}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Locate the `silverc` binary: prefer PATH, fall back to the pinned cargo-bin
/// location recorded in the session FACTS. Returns `None` if neither resolves.
fn find_silverc() -> Option<PathBuf> {
    // 1. PATH (covers CI images that install silverc system-wide).
    if let Ok(output) = Command::new("silverc").arg("--version").output() {
        if output.status.success() || !output.stdout.is_empty() {
            return Some(PathBuf::from("silverc"));
        }
    }
    // Some CLIs exit non-zero on --version; also accept a plain `which`.
    if let Ok(output) = Command::new("which").arg("silverc").output() {
        if output.status.success() {
            let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
    }
    // 2. Pinned cargo-bin fallback (FACTS): $HOME/.cargo/bin/silverc, resolved
    //    at runtime (HOME is not guaranteed to be set at compile time).
    if let Ok(home) = std::env::var("HOME") {
        let pinned = PathBuf::from(home).join(".cargo/bin/silverc");
        if pinned.exists() {
            return Some(pinned);
        }
    }
    None
}

/// DIFFERENTIAL core: write `.sil` + CTOR JSON to a temp dir and run
/// `silverc --ctor <ctor> -c <sil>`, asserting exit 0. Returns `true` if
/// silverc actually ran (so callers can distinguish "passed" from "skipped").
fn differential_compile(label: &str, model: &CovenantModel, sil: &str) -> bool {
    let Some(silverc) = find_silverc() else {
        eprintln!(
            "SKIP[{label}]: silverc not found on PATH nor at $HOME/.cargo/bin/silverc \
             — differential check skipped (NOT silently passed)."
        );
        return false;
    };

    let dir = temp_workdir(label);
    let sil_path = dir.join(format!("{}.sil", model.name));
    fs::write(&sil_path, sil).expect("write sil");

    let (ctor_name, ctor_json) = emit_ctor(model);
    let ctor_path = dir.join(ctor_name);
    fs::write(&ctor_path, ctor_json).expect("write ctor json");

    let output = Command::new(&silverc)
        .arg("--ctor")
        .arg(&ctor_path)
        .arg("-c")
        .arg(&sil_path)
        .output()
        .unwrap_or_else(|e| panic!("[{label}] failed to spawn silverc ({silverc:?}): {e}"));

    assert!(
        output.status.success(),
        "[{label}] DIFFERENTIAL FAIL: silverc rejected the emitted .sil (exit {:?}).\n\
         --- sil ---\n{sil}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    true
}

// ---------------------------------------------------------------------------
// 1. GOLDEN snapshots
// ---------------------------------------------------------------------------

#[test]
fn golden_counter_emits_expected_anchors() {
    let emitted = run_pipeline("counter", COUNTER_SRC);
    assert_eq!(emitted.len(), 1, "counter is a single-role app");
    let (model, sil) = &emitted[0];

    assert_eq!(model.name, "Counter", "covenant model name");
    assert!(
        sil.contains("pragma silverscript ^0.1.0;"),
        "missing silverscript pragma:\n{sil}"
    );
    assert!(
        sil.contains("contract Counter"),
        "missing contract decl:\n{sil}"
    );
    // Entrypoint name survives lowering.
    assert!(
        sil.contains("function bump("),
        "missing `bump` entrypoint:\n{sil}"
    );
    // Transition return-type declaration + lowered return expr.
    assert!(
        sil.contains(": (State)"),
        "missing transition return-type:\n{sil}"
    );
    assert!(
        sil.contains("return({ value: prev_states[0].value + delta })"),
        "missing lowered return body:\n{sil}"
    );
}

#[test]
fn golden_compliance_token_emits_expected_anchors() {
    let emitted = run_pipeline("compliance_token", COMPLIANCE_TOKEN_SRC);
    assert_eq!(emitted.len(), 1, "ComplianceToken is a single-role app");
    let (model, sil) = &emitted[0];

    assert_eq!(model.name, "ComplianceToken", "covenant model name");
    assert!(
        model.has_vprog,
        "ComplianceToken has a VProg (verify_compliance) transition"
    );
    assert!(
        sil.contains("pragma silverscript ^0.1.0;"),
        "missing silverscript pragma:\n{sil}"
    );
    assert!(
        sil.contains("contract ComplianceToken"),
        "missing contract decl:\n{sil}"
    );
    // The covenant entrypoint is emitted; the VProg entrypoint is NOT (handled by Atelier).
    assert!(
        sil.contains("function transfer("),
        "missing `transfer` entrypoint:\n{sil}"
    );
    assert!(
        !sil.contains("function verify_compliance"),
        "VProg entrypoint must NOT be emitted into the .sil (Atelier owns it):\n{sil}"
    );
    // VProg binding injects the proof covenant-id arg + OpInputCovenantId guard.
    assert!(
        sil.contains("byte[32] proof_cov_id"),
        "missing VProg proof_cov_id arg:\n{sil}"
    );
    assert!(
        sil.contains("OpInputCovenantId(0)"),
        "missing OpInputCovenantId cross-layer binding guard:\n{sil}"
    );
    assert!(
        sil.contains("return({ balance: prev_states[0].balance - amount })"),
        "missing lowered return body:\n{sil}"
    );
}

#[test]
fn golden_heavy_airdrop_is_layer_aware() {
    // The two-layer allocation proof. The covenant entrypoint `settle` emits a
    // `.sil`; the vProg entrypoint `tally` — which contains a `for` loop, the
    // exact construct LoopAirdrop is REJECTED for — is accepted (as a vProg hole)
    // and is NOT emitted into the covenant `.sil` (Atelier owns it).
    let emitted = run_pipeline("heavy_airdrop", HEAVY_AIRDROP_SRC);
    assert_eq!(emitted.len(), 1, "HeavyAirdrop is a single-role app");
    let (model, sil) = &emitted[0];
    assert_eq!(model.name, "HeavyAirdrop", "covenant model name");
    assert!(
        model.has_vprog,
        "HeavyAirdrop has a VProg (tally) transition"
    );
    assert!(
        sil.contains("function settle("),
        "covenant `settle` must be emitted:\n{sil}"
    );
    assert!(
        !sil.contains("function tally"),
        "vProg `tally` (with its loop) must NOT be emitted into the .sil:\n{sil}"
    );

    // The allocation advisor confirms `tally` is correctly on the vProgs layer
    // because it uses `for`, and does NOT flag the clean covenant `settle`.
    let program = portrait_syntax::parse(HEAVY_AIRDROP_SRC).expect("HeavyAirdrop parses");
    let advisories = portrait_sema::advise(&program);
    assert!(
        advisories
            .iter()
            .any(|a| a.entry == "tally" && a.layer == "VProg" && a.message.contains("`for`")),
        "advisor must note `tally` uses `for` on the vProgs layer, got: {advisories:?}"
    );
    assert!(
        !advisories.iter().any(|a| a.entry == "settle"),
        "advisor must not flag the clean covenant `settle`, got: {advisories:?}"
    );
}

// ---------------------------------------------------------------------------
// 1b. BYTE-IDENTITY (LOW-3, Phase B/C red-team) — the emitted `.sil` must be
//     byte-for-byte identical to the committed baseline. The substring anchors
//     above guard the load-bearing tokens but tolerate cosmetic drift; this
//     locks the *exact* bytes the prior red-team could only verify by manual
//     diff. If the emitter is intentionally changed, regenerate the committed
//     baseline (it lives next to the source) and this test re-pins it.
// ---------------------------------------------------------------------------

/// Committed `.sil` baselines, embedded at compile time.
const COUNTER_BASELINE_SIL: &str = include_str!("../../../../examples/Counter.sil");
const COMPLIANCE_TOKEN_BASELINE_SIL: &str =
    include_str!("../../../../examples/tier3-demo/ComplianceToken.sil");

#[test]
fn byte_identity_counter_matches_baseline() {
    let emitted = run_pipeline("counter", COUNTER_SRC);
    assert_eq!(emitted.len(), 1, "counter is a single-role app");
    let (_, sil) = &emitted[0];
    assert_eq!(
        sil, COUNTER_BASELINE_SIL,
        "emitted Counter.sil is no longer byte-identical to the committed baseline \
         (examples/Counter.sil). If this change is intentional, regenerate the baseline."
    );
}

#[test]
fn byte_identity_compliance_token_matches_baseline() {
    let emitted = run_pipeline("compliance_token", COMPLIANCE_TOKEN_SRC);
    assert_eq!(emitted.len(), 1, "ComplianceToken is a single-role app");
    let (_, sil) = &emitted[0];
    assert_eq!(
        sil, COMPLIANCE_TOKEN_BASELINE_SIL,
        "emitted ComplianceToken.sil is no longer byte-identical to the committed baseline \
         (examples/tier3-demo/ComplianceToken.sil). If intentional, regenerate the baseline."
    );
}

// ---------------------------------------------------------------------------
// 2. DIFFERENTIAL (§4.2) — emitted .sil must compile under the real silverc
// ---------------------------------------------------------------------------

#[test]
fn differential_counter_compiles_under_silverc() {
    let emitted = run_pipeline("counter", COUNTER_SRC);
    let (model, sil) = &emitted[0];
    let _ran = differential_compile("counter", model, sil);
    // If silverc is absent the call prints a SKIP message and returns false;
    // a regression (silverc present but rejects) trips the assert inside.
}

#[test]
fn differential_compliance_token_compiles_under_silverc() {
    let emitted = run_pipeline("compliance_token", COMPLIANCE_TOKEN_SRC);
    let (model, sil) = &emitted[0];
    let _ran = differential_compile("compliance_token", model, sil);
}

// ---------------------------------------------------------------------------
// 3. REJECT vectors — malformed programs must fail at parse or sema
// ---------------------------------------------------------------------------

/// Reject at PARSE: a source missing the mandatory `pragma` header is not a
/// well-formed Portrait program.
#[test]
fn reject_missing_pragma_fails_at_parse() {
    let src = "app Broken { role r { state { int x; } } }";
    let result = portrait_syntax::parse(src);
    assert!(
        result.is_err(),
        "expected parse error for source with no pragma, got: {result:?}"
    );
}

/// Reject at PARSE: structurally truncated source (unterminated `app` block).
#[test]
fn reject_truncated_app_fails_at_parse() {
    let src = "pragma portrait ^0.1.0;\napp Truncated {\n  role r {\n";
    let result = portrait_syntax::parse(src);
    assert!(
        result.is_err(),
        "expected parse error for truncated app block, got: {result:?}"
    );
}

/// Reject at SEMA: a program that parses cleanly but whose lifecycle edge
/// references an entrypoint that does not exist. `parse` accepts it; the
/// structural checker (`portrait_sema::check`) must reject it.
#[test]
fn reject_unknown_lifecycle_entry_fails_at_sema() {
    let src = "\
pragma portrait ^0.1.0;

app Bad {
  role r {
    param int start;
    state { int value; }

    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }

  lifecycle { live -> live via r.does_not_exist; }
  invariant no_undeclared_state;
}
";
    // It must parse (the malformation is structural, not syntactic)...
    let program = portrait_syntax::parse(src)
        .expect("source is syntactically valid; the entry reference is the defect");
    // ...and sema must reject the dangling entrypoint reference.
    let result = portrait_sema::check(&program);
    assert!(
        result.is_err(),
        "expected sema rejection for lifecycle edge referencing unknown entrypoint, \
         but check() passed"
    );
}

// ---------------------------------------------------------------------------
// 4. REJECTION SET (vision risk H-1) — out-of-subset constructs must be
//    rejected fail-loud, NAMING the offending construct and routing it to the
//    vProgs layer, NOT silently degraded to Stmt::Raw / miscompiled. These wire
//    the documented rejection set (docs/SOLIDITY-SUBSET-V0.md §3) into the
//    regression harness so the boundary has test coverage, not just prose.
// ---------------------------------------------------------------------------

const ALLOWANCE_TOKEN_SRC: &str =
    include_str!("../../../../examples/engraver-demo/rejected/AllowanceToken.portrait");
const LOOP_AIRDROP_SRC: &str =
    include_str!("../../../../examples/engraver-demo/rejected/LoopAirdrop.portrait");
const CROSS_CALL_VAULT_SRC: &str =
    include_str!("../../../../examples/engraver-demo/rejected/CrossCallVault.portrait");

/// Assert `parse` rejects `src` fail-loud with a message that NAMES `construct`
/// and routes it to the vProgs layer (the precise shape vision risk H-1 demands).
fn assert_rejected_naming(label: &str, src: &str, construct: &str) {
    let err = portrait_syntax::parse(src).map(|_| ()).expect_err(&format!(
        "[{label}] out-of-subset source must be rejected, not accepted"
    ));
    assert!(
        err.contains(construct),
        "[{label}] rejection must NAME the offending construct `{construct}`, got: {err}"
    );
    assert!(
        err.contains("unsupported construct") && err.contains("vProgs layer"),
        "[{label}] rejection must fail loud and route to the vProgs layer, got: {err}"
    );
}

#[test]
fn reject_allowance_mapping_names_construct() {
    // map<K,V> shared mutable mapping (SUBSET-V0 §3 item 1) is hit first.
    assert_rejected_naming("AllowanceToken", ALLOWANCE_TOKEN_SRC, "map<K, V>");
}

#[test]
fn reject_unbounded_loop_names_construct() {
    assert_rejected_naming("LoopAirdrop", LOOP_AIRDROP_SRC, "`for`");
}

#[test]
fn reject_cross_contract_call_names_construct() {
    assert_rejected_naming("CrossCallVault", CROSS_CALL_VAULT_SRC, "`call`");
}

// 4b. EMBEDDED-VECTOR rejection (adversarial-verify follow-up) — a blacklisted
//     construct embedded inside a `require`/`return` previously bypassed the
//     fail-loud rejection set and degraded to Stmt::Raw (a FALSE ACCEPT). These
//     fixtures pin that the require/return paths now consult REJECTION_SET before
//     degrading, emitting the SAME diagnostic as the standalone path.

const REQUIRE_CROSS_CALL_VAULT_SRC: &str =
    include_str!("../../../../examples/engraver-demo/rejected/RequireCrossCallVault.portrait");
const RETURN_CROSS_CALL_VAULT_SRC: &str =
    include_str!("../../../../examples/engraver-demo/rejected/ReturnCrossCallVault.portrait");

#[test]
fn reject_cross_contract_call_embedded_in_require_names_construct() {
    assert_rejected_naming(
        "RequireCrossCallVault",
        REQUIRE_CROSS_CALL_VAULT_SRC,
        "`call`",
    );
}

#[test]
fn reject_cross_contract_call_embedded_in_return_names_construct() {
    assert_rejected_naming(
        "ReturnCrossCallVault",
        RETURN_CROSS_CALL_VAULT_SRC,
        "`call`",
    );
}

// ---------------------------------------------------------------------------
// 5. ACCEPT fixtures — Green-tier contracts that must project AND compile under
//    silverc, growing the validated set (was 3: SimpleToken, PausableToken,
//    VestingWallet → now 5 with SimpleEscrow + OwnableCounter).
// ---------------------------------------------------------------------------

const SIMPLE_ESCROW_SRC: &str =
    include_str!("../../../../examples/engraver-demo/SimpleEscrow.portrait");
const OWNABLE_COUNTER_SRC: &str =
    include_str!("../../../../examples/engraver-demo/OwnableCounter.portrait");

#[test]
fn accept_simple_escrow_projects_and_compiles() {
    let emitted = run_pipeline("simple_escrow", SIMPLE_ESCROW_SRC);
    assert_eq!(emitted.len(), 1, "SimpleEscrow is a single-role app");
    let (model, sil) = &emitted[0];
    assert_eq!(model.name, "SimpleEscrow");
    assert!(
        sil.contains("function release("),
        "missing release entrypoint:\n{sil}"
    );
    let _ran = differential_compile("simple_escrow", model, sil);
}

#[test]
fn accept_ownable_counter_projects_and_compiles() {
    let emitted = run_pipeline("ownable_counter", OWNABLE_COUNTER_SRC);
    assert_eq!(emitted.len(), 1, "OwnableCounter is a single-role app");
    let (model, sil) = &emitted[0];
    assert_eq!(model.name, "OwnableCounter");
    assert!(
        sil.contains("function increment("),
        "missing increment entrypoint:\n{sil}"
    );
    let _ran = differential_compile("ownable_counter", model, sil);
}

// ---------------------------------------------------------------------------
// 6. RWA / INDUSTRIAL COVENANTS (Month-2 pattern additions) — a bounded,
//    table-driven differential/property increment over the FOUR new finance
//    covenants. Two complementary layers per pattern:
//
//      ACCEPT — the real `.portrait` source parses, passes `sema::check`, drives
//               the full pipeline, emits the load-bearing `.sil` anchors, and
//               (where silverc is present) compiles under the real `silverc`.
//      REJECT — a single surgical string-mutation of the valid source that
//               violates EXACTLY ONE declared invariant must be rejected by
//               `sema::check`, with the diagnostic NAMING that invariant.
//
//    Hermetic: the valid sources are embedded via include_str! and every negative
//    is a one-line string replacement of an embedded copy — no on-disk fixtures
//    beyond the four `.portrait` files themselves. Deterministic and additive.
// ---------------------------------------------------------------------------

const KYC_GATED_TRANSFER_SRC: &str =
    include_str!("../../../../library/finance/kyc-transfer/KycGatedTransfer.portrait");
const LIQUIDATABLE_LOAN_SRC: &str =
    include_str!("../../../../library/finance/liquidatable-loan/LiquidatableLoan.portrait");
const TRANCHE_WATERFALL_SRC: &str =
    include_str!("../../../../library/finance/tranche-waterfall/TrancheWaterfall.portrait");
const PAYROLL_STREAM_SRC: &str =
    include_str!("../../../../library/finance/payroll-stream/PayrollStream.portrait");

/// ACCEPT helper: parse + check Ok, run the full pipeline, assert the emitted
/// `.sil` carries the load-bearing anchors (pragma, contract decl, the named
/// entrypoints), then differential-compile under silverc (skip-with-message if
/// silverc is absent — never a silent pass).
fn accept_covenant(label: &str, src: &str, name: &str, entrypoints: &[&str]) {
    // parse + check must both succeed on the real source.
    let program =
        portrait_syntax::parse(src).unwrap_or_else(|e| panic!("[{label}] parse failed: {e}"));
    portrait_sema::check(&program).unwrap_or_else(|ds| {
        let msgs: Vec<_> = ds.into_iter().map(|d| d.message).collect();
        panic!(
            "[{label}] sema::check rejected a valid covenant: {}",
            msgs.join("; ")
        );
    });
    // full pipeline + anchor assertions.
    let emitted = run_pipeline(label, src);
    assert_eq!(emitted.len(), 1, "[{label}] is a single-role app");
    let (model, sil) = &emitted[0];
    assert_eq!(model.name, name, "[{label}] covenant model name");
    assert!(
        sil.contains("pragma silverscript ^0.1.0;"),
        "[{label}] missing silverscript pragma:\n{sil}"
    );
    assert!(
        sil.contains(&format!("contract {name}")),
        "[{label}] missing contract decl:\n{sil}"
    );
    for ep in entrypoints {
        assert!(
            sil.contains(&format!("function {ep}(")),
            "[{label}] missing `{ep}` entrypoint:\n{sil}"
        );
    }
    let _ran = differential_compile(label, model, sil);
}

/// REJECT helper: apply a single surgical string replacement to a copy of the
/// valid source (asserting the `from` substring is present and unique so the
/// mutation is the only change), then assert `sema::check` rejects it with a
/// diagnostic that NAMES the violated invariant. The mutated source must still
/// PARSE — the defect is semantic, not syntactic.
fn reject_mutation(label: &str, src: &str, from: &str, to: &str, invariant: &str) {
    let occurrences = src.matches(from).count();
    assert_eq!(
        occurrences, 1,
        "[{label}] mutation anchor `{from}` must appear exactly once in the source \
         (found {occurrences}); the test would otherwise not be a single surgical edit"
    );
    let mutated = src.replacen(from, to, 1);
    assert_ne!(mutated, src, "[{label}] mutation was a no-op");
    let program = portrait_syntax::parse(&mutated).unwrap_or_else(|e| {
        panic!("[{label}] mutated source must still PARSE (semantic defect): {e}")
    });
    let result = portrait_sema::check(&program);
    let diags = result.expect_err(&format!(
        "[{label}] mutated source must be REJECTED by sema::check, but it passed"
    ));
    let joined = diags
        .into_iter()
        .map(|d| d.message)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        joined.contains(invariant),
        "[{label}] rejection diagnostic must NAME the violated invariant `{invariant}`, got: {joined}"
    );
}

// ── ACCEPT (4 cases) ─────────────────────────────────────────────────────

#[test]
fn accept_kyc_gated_transfer() {
    accept_covenant(
        "kyc_gated_transfer",
        KYC_GATED_TRANSFER_SRC,
        "KycGatedTransfer",
        &["transfer"],
    );
}

#[test]
fn accept_liquidatable_loan() {
    accept_covenant(
        "liquidatable_loan",
        LIQUIDATABLE_LOAN_SRC,
        "LiquidatableLoan",
        &["repay", "liquidate"],
    );
}

#[test]
fn accept_tranche_waterfall() {
    accept_covenant(
        "tranche_waterfall",
        TRANCHE_WATERFALL_SRC,
        "TrancheWaterfall",
        &["distribute"],
    );
}

#[test]
fn accept_payroll_stream() {
    accept_covenant(
        "payroll_stream",
        PAYROLL_STREAM_SRC,
        "PayrollStream",
        &["release"],
    );
    // Pin the new compound shape: the SAME entrypoint carries BOTH the temporal
    // gate and the per-release cap on one value-conserved balance.
    let emitted = run_pipeline("payroll_stream", PAYROLL_STREAM_SRC);
    let (_, sil) = &emitted[0];
    assert!(
        sil.contains("now_bucket >= prev_states[0].last_paid + prev_states[0].period"),
        "PayrollStream must lower the temporal gate:\n{sil}"
    );
    assert!(
        sil.contains("amount <= prev_states[0].limit"),
        "PayrollStream must lower the per-release spending cap:\n{sil}"
    );
}

// ── REJECT / invariant-mutation table ────────────────────────────────────

// KycGatedTransfer: (a) non_negative_amount, (b) conservation_split, (c) authorized.
// Also pin that the load-bearing KYC gate is present in the ACCEPTed source.
#[test]
fn kyc_gate_require_is_present() {
    assert!(
        KYC_GATED_TRANSFER_SRC.contains("requires allowed == 1;"),
        "the KYC gate `requires allowed == 1;` is the load-bearing difference from a \
         plain transfer and must be present in the covenant source"
    );
}

#[test]
fn reject_kyc_drops_non_negative_amount() {
    reject_mutation(
        "kyc:non_negative_amount",
        KYC_GATED_TRANSFER_SRC,
        "      requires amount >= 0;              // non-negative transfer (non_negative_amount)\n",
        "",
        "non_negative_amount",
    );
}

#[test]
fn reject_kyc_breaks_conservation_split() {
    reject_mutation(
        "kyc:conservation_split",
        KYC_GATED_TRANSFER_SRC,
        "to_balance:   to_balance + amount      // destination leg increases by the SAME amount",
        "to_balance:   to_balance + amount + 1  // BROKEN: deltas no longer cancel",
        "conservation_split",
    );
}

#[test]
fn reject_kyc_drops_checksig() {
    reject_mutation(
        "kyc:authorized",
        KYC_GATED_TRANSFER_SRC,
        "      requires checkSig(auth, holder);   // only the committed holder may transfer\n",
        "",
        "authorized",
    );
}

// LiquidatableLoan: (a) liquidate authorized, (b) repay non_negative_amount.
// The liquidation ratio guard has no dedicated invariant (it is a plain require),
// so its presence is pinned by the ACCEPT golden plus this substring check.
#[test]
fn liquidation_ratio_guard_is_present() {
    assert!(
        LIQUIDATABLE_LOAN_SRC.contains("requires collateral < debt * min_ratio;"),
        "the liquidation guard `requires collateral < debt * min_ratio;` (structural inverse \
         of the borrow ratio guard) is load-bearing and must be present"
    );
}

#[test]
fn reject_loan_liquidate_drops_checksig() {
    reject_mutation(
        "loan:authorized",
        LIQUIDATABLE_LOAN_SRC,
        "      requires checkSig(auth, liquidator);          // only the committed liquidator may liquidate\n",
        "",
        "authorized",
    );
}

#[test]
fn reject_loan_repay_drops_non_negative_amount() {
    reject_mutation(
        "loan:non_negative_amount",
        LIQUIDATABLE_LOAN_SRC,
        "      requires amount >= 0;                // non-negative repayment (non_negative_amount)\n",
        "",
        "non_negative_amount",
    );
}

// TrancheWaterfall: (a) conservation_split, (b) authorized.
#[test]
fn reject_tranche_breaks_conservation_split() {
    reject_mutation(
        "tranche:conservation_split",
        TRANCHE_WATERFALL_SRC,
        "senior_balance: senior_balance + s,            // senior gains s",
        "senior_balance: senior_balance + s + 1,        // BROKEN: added atoms != subtracted atoms",
        "conservation_split",
    );
}

#[test]
fn reject_tranche_drops_checksig() {
    reject_mutation(
        "tranche:authorized",
        TRANCHE_WATERFALL_SRC,
        "      requires checkSig(auth, trustee);          // only the committed trustee may distribute\n",
        "",
        "authorized",
    );
}

// PayrollStream: (a) spending_cap, (b) temporal_guard, (c) value_conserved.
#[test]
fn reject_payroll_drops_spending_cap() {
    reject_mutation(
        "payroll:spending_cap",
        PAYROLL_STREAM_SRC,
        "      requires amount <= limit;                        // per-release wage cap (spending_cap)\n",
        "",
        "spending_cap",
    );
}

#[test]
fn reject_payroll_weakens_temporal_guard() {
    // The transition still READS `now_bucket` in a guard (so the temporal_guard
    // check fires), but the gate no longer compares against a COMMITTED time —
    // `amount` is a caller-supplied arg, not committed state — so the committed-
    // window form `asserts_temporal_gate` requires is no longer satisfied. A
    // caller could pass any `now_bucket` and bypass the cadence. This is the
    // honest negative: simply DELETING the require would remove the only
    // `now_bucket` guard and (correctly) stop triggering the check, so the
    // mutation that exercises the invariant weakens the gate rather than removing it.
    reject_mutation(
        "payroll:temporal_guard",
        PAYROLL_STREAM_SRC,
        "requires now_bucket >= last_paid + period;       // rate limit: one release per period (temporal_guard)",
        "requires now_bucket >= amount;                   // BROKEN: gate not against committed time",
        "temporal_guard",
    );
}

#[test]
fn reject_payroll_breaks_value_conserved() {
    reject_mutation(
        "payroll:value_conserved",
        PAYROLL_STREAM_SRC,
        "balance:   balance - amount                    // single additive subtraction (value_conserved)",
        "balance:   balance - amount - amount           // BROKEN: non-single-additive",
        "value_conserved",
    );
}

// ---------------------------------------------------------------------------
//  FLAGSHIP — CsciInstrument (the Covenant-Settled Compliance Instrument).
//
//  This is the panel flagship: one Portrait source -> a silverscript covenant
//  whose compiled `settle` enforces the CSCI state machine on-chain (proven
//  LIVE on TN10; see kaspa-compliance-patterns examples/portrait-settlement/
//  PROVENANCE.json). Its siblings above (kyc/loan/tranche/payroll) carry ACCEPT
//  + REJECT goldens; the flagship was previously exercised only INDIRECTLY by
//  the portrait-sema c2_*/c3_* unit tests. These goldens pin the flagship
//  end-to-end through the SAME real pipeline + real silverc the others use, and
//  in particular assert the KIP-20 covenant-id binding line is present in the
//  emitted `.sil` — the load-bearing difference that ties the silverscript
//  layer to the ZK-settled journal.
//
//  CsciInstrument has TWO entrypoints in the source: `settle` (the #[covenant]
//  transition the Engraver compiles to .sil) and `csci_rules` (no #[covenant]
//  attribute -> a NonCovenant/vProg body that Atelier lowers to a guest main,
//  and whose mere presence flips `has_vprog`, which is what causes the
//  `require(proof_cov_id == OpInputCovenantId(0))` binding to be emitted).
//  So the emitted single covenant is named CsciInstrument with one entrypoint
//  (`settle`) in the .sil.
// ---------------------------------------------------------------------------

const CSCI_INSTRUMENT_SRC: &str = include_str!("../../../../library/state/CsciInstrument.portrait");

/// ACCEPT (end-to-end): the flagship source parses, passes sema, drives the
/// full pipeline to exactly ONE covenant (`CsciInstrument` with `settle`), and
/// the emitted `.sil` compiles under the real `silverc`. Then pin the two
/// load-bearing anchors that make this the CSCI flagship:
///   (1) the KIP-20 covenant-id binding `require(proof_cov_id == OpInputCovenantId(0))`
///       — the silverscript-layer cross-binding to the settled UTXO's identity;
///   (2) the committed-owner auth `checkSig(auth, prev_states[0].owner)` and the
///       seq-advances-by-one continuation, the on-chain state-machine rules.
#[test]
fn accept_csci_instrument_flagship() {
    accept_covenant(
        "csci_instrument",
        CSCI_INSTRUMENT_SRC,
        "CsciInstrument",
        &["settle"],
    );
    let emitted = run_pipeline("csci_instrument", CSCI_INSTRUMENT_SRC);
    assert_eq!(
        emitted.len(),
        1,
        "CsciInstrument lowers to exactly ONE covenant (the `csci_rules` body is a \
         NonCovenant/vProg, not a second covenant)"
    );
    let (_, sil) = &emitted[0];
    // (1) the KIP-20 covenant-id binding — emitted because `has_vprog`.
    assert!(
        sil.contains("require(proof_cov_id == OpInputCovenantId(0));"),
        "CsciInstrument MUST emit the KIP-20 covenant-id binding \
         `require(proof_cov_id == OpInputCovenantId(0));` (the load-bearing tie \
         between the silverscript layer and the ZK-settled journal):\n{sil}"
    );
    // (2) committed-owner authorization (never a caller-supplied pubkey).
    assert!(
        sil.contains("require(checkSig(auth, prev_states[0].owner));"),
        "CsciInstrument MUST authorize against the COMMITTED owner \
         `checkSig(auth, prev_states[0].owner)`:\n{sil}"
    );
    // (3) sequence advances by exactly one in the continuation.
    assert!(
        sil.contains("seq: prev_states[0].seq + 1"),
        "CsciInstrument continuation MUST advance seq by exactly one:\n{sil}"
    );
}

/// Pin the KIP-20 binding at the SOURCE level too: the binding is emitted only
/// because the source carries a vProg companion entrypoint (`csci_rules` with no
/// #[covenant] attribute) alongside the covenant `settle`. If that companion is
/// ever removed the binding silently disappears — this guards against that.
#[test]
fn csci_kip20_binding_is_present() {
    assert!(
        CSCI_INSTRUMENT_SRC.contains("entrypoint function csci_rules("),
        "the `csci_rules` vProg companion is what flips `has_vprog` and causes the \
         covenant-id binding require() to be emitted; it is load-bearing"
    );
    let emitted = run_pipeline("csci_kip20", CSCI_INSTRUMENT_SRC);
    let (_, sil) = &emitted[0];
    assert!(
        sil.contains("require(proof_cov_id == OpInputCovenantId(0));"),
        "the KIP-20 covenant-id binding must be present in the emitted flagship .sil:\n{sil}"
    );
}

// ── FLAGSHIP REJECT vectors (compile-time sema, mirroring the on-chain
//    negative controls captured live in PROVENANCE.json) ───────────────────
//
//  Each surgical mutation of the valid flagship source is rejected by
//  sema::check, NAMING the violated invariant — the compile-time tier of the
//  same property the live TN10 negative controls reject at consensus:
//
//    sema reject (here)              on-chain negative control (PROVENANCE.json)
//    ─────────────────────────────  ──────────────────────────────────────────
//    monotonic_seq violated         seq-violation settle REJECT (f11c8875...,
//                                    "script ran, but verification failed", 404)
//    no-auth state mutation         (committed-owner auth; the live cross-binding
//                                    and ZK-integrity rejects guard the adjacent
//                                    identity/proof properties: 60e9effc...,
//                                    e44d4f7b.../1f49c3dc..., each REST 404)

/// REJECT: break seq monotonicity in the on-chain `settle` covenant (advance by
/// zero instead of one). The anchor is the unique `settle`-block prefix (it
/// carries `requires checkSig`, which the `csci_rules` vProg body does not), so
/// the mutation touches ONLY the covenant entrypoint. sema must reject naming
/// `monotonic_seq` — the compile-time mirror of the live seq-violation reject
/// (f11c8875..., "script ran, but verification failed").
#[test]
fn reject_csci_non_monotonic_seq() {
    reject_mutation(
        "csci:monotonic_seq",
        CSCI_INSTRUMENT_SRC,
        "      requires checkSig(auth, owner);          // committed-owner authorization\n      return CsciInstrument {\n        owner:      owner,                      // owner key carried unchanged\n        amount:     amount,                     // value conserved (carry f:f)\n        seq:        seq + 1,                    // CSCI sequence advances by one",
        "      requires checkSig(auth, owner);          // committed-owner authorization\n      return CsciInstrument {\n        owner:      owner,                      // owner key carried unchanged\n        amount:     amount,                     // value conserved (carry f:f)\n        seq:        seq,                        // BROKEN: seq does not advance",
        "monotonic_seq",
    );
}

/// REJECT: drop the committed-owner authorization from the `settle` covenant.
/// Under the declared `value_conserved` invariant a state-mutating transition
/// with NO authorization is rejected by sema (the C2 no-checkSig check). This is
/// the compile-time guard for the on-chain owner-auth property that the live
/// settle ACCEPT (5731a203...) and seq-violation REJECT exercise.
#[test]
fn reject_csci_drops_owner_auth() {
    reject_mutation(
        "csci:owner_auth",
        CSCI_INSTRUMENT_SRC,
        "      requires checkSig(auth, owner);          // committed-owner authorization\n",
        "",
        "state-mutating transition has NO",
    );
}
