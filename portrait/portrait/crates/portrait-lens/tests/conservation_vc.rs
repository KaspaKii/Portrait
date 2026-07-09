//! End-to-end VC generation + (z3-gated) discharge tests for the total
//! value-conservation VC.
//!
//! The SMT-LIB *generation* tests run on any machine (no solver needed). The
//! *discharge* tests are gated on a reachable `z3` binary: when z3 is absent they
//! `eprintln!`-skip and return early (never a silent pass), so the default
//! `cargo test` stays GREEN on a machine without z3. With z3 present they assert
//! the real verdicts.

use portrait_lens::{discharge, prove_program, value_bearing_fields, Outcome, VcKind, VcReport};
use portrait_syntax::{parse, Program};

const TIMEOUT_MS: u64 = 10_000;

/// Serializes every test that runs (or simulates) the solver, because z3
/// discovery reads the process-global `PORTRAIT_Z3` env var; the z3-absent test
/// mutates it. Without this, parallel discharge tests could observe the bogus
/// path. Tests acquire this before calling `discharge`.
static SOLVER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ── Fixtures ────────────────────────────────────────────────────────────────

/// The real correct InternalSplit covenant (3 legs, deltas net to zero).
fn internalsplit_correct_src() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../library/finance/internal-split/InternalSplit.portrait"
    ))
    .expect("InternalSplit.portrait should exist")
}

/// The fat-fingered InternalSplit from spec §6.2: leg c gains `x` not `y`, so the
/// deltas {-(x+y), +x, +x} sum to +x-y ≠ 0 — value created out of nothing.
///
/// The `conservation_split` invariant is dropped from this variant: sema's
/// structural D4 check would (correctly) reject the broken source at parse time,
/// but the point of THIS test is that Lens's *arithmetic* VC also catches it —
/// so we let sema pass on the weakened invariant set and assert Lens REFUTES.
fn internalsplit_broken_src() -> String {
    internalsplit_correct_src()
        .replace("pool_c_balance + y", "pool_c_balance + x")
        .replace("  invariant conservation_split;\n", "")
        .replace("  invariant authorized;\n", "")
}

/// A 2-leg internal flow that conserves value via a *multiplicative* term on one
/// leg and an *additive-identity* term on the other: `-x*2` out of leg a, `+x+x`
/// into leg b. Net zero, but D4's structural atom-cancellation cannot see that
/// `x*2 == x+x` — Lens proves it by real arithmetic. Declares only
/// `no_undeclared_state` so sema's structural conservation checks do not fire.
const DOUBLING_CONSERVING: &str = r#"pragma portrait ^0.1.0;
app Doubling {
  role pool {
    param int    pool_a_balance;
    param int    pool_b_balance;
    param pubkey owner;
    state {
      int    pool_a_balance;
      int    pool_b_balance;
      pubkey owner;
    }
    #[covenant(mode = transition)]
    entrypoint function rebalance(
      sig auth,
      int x
    ) : (int pool_a_balance, int pool_b_balance, pubkey owner) {
      requires checkSig(auth, owner);
      requires x >= 0;
      requires x * 2 <= pool_a_balance;
      return Doubling {
        pool_a_balance: pool_a_balance - x * 2,
        pool_b_balance: pool_b_balance + x + x,
        owner:          owner
      };
    }
  }
  lifecycle { live -> live via pool.rebalance; }
  invariant no_undeclared_state;
}
"#;

/// A value-CREATING 2-leg flow with a SELF-CONTRADICTORY guard: `requires x >= 5`
/// AND `requires x <= 3` can never both hold, so the transition relation `T` is
/// unsatisfiable (the entrypoint is unreachable / vacuous). The body still
/// CREATES value (`+x+1` into leg b vs `-x` out of leg a). Without a vacuity
/// guard, `(T ∧ ¬VC)` is trivially `unsat` and Lens would FALSELY report PROVED.
/// With the vacuity check it must report UNKNOWN (T alone is unsat). The IDENTICAL
/// body with a SATISFIABLE guard (`VALUE_CREATING`) must still REFUTE.
const VALUE_CREATING_VACUOUS_GUARD: &str = r#"pragma portrait ^0.1.0;
app LeakyVacuous {
  role pool {
    param int    pool_a_balance;
    param int    pool_b_balance;
    param pubkey owner;
    state {
      int    pool_a_balance;
      int    pool_b_balance;
      pubkey owner;
    }
    #[covenant(mode = transition)]
    entrypoint function rebalance(
      sig auth,
      int x
    ) : (int pool_a_balance, int pool_b_balance, pubkey owner) {
      requires checkSig(auth, owner);
      requires x >= 5;
      requires x <= 3;
      requires x <= pool_a_balance;
      return LeakyVacuous {
        pool_a_balance: pool_a_balance - x,
        pool_b_balance: pool_b_balance + x + 1,
        owner:          owner
      };
    }
  }
  lifecycle { live -> live via pool.rebalance; }
  invariant no_undeclared_state;
}
"#;

/// A value-CREATING 2-leg flow: `-x` out of leg a, `+x+1` into leg b. Net +1 —
/// the transition mints one unit out of nothing. Must be REFUTED.
const VALUE_CREATING: &str = r#"pragma portrait ^0.1.0;
app Leaky {
  role pool {
    param int    pool_a_balance;
    param int    pool_b_balance;
    param pubkey owner;
    state {
      int    pool_a_balance;
      int    pool_b_balance;
      pubkey owner;
    }
    #[covenant(mode = transition)]
    entrypoint function rebalance(
      sig auth,
      int x
    ) : (int pool_a_balance, int pool_b_balance, pubkey owner) {
      requires checkSig(auth, owner);
      requires x >= 0;
      requires x <= pool_a_balance;
      return Leaky {
        pool_a_balance: pool_a_balance - x,
        pool_b_balance: pool_b_balance + x + 1,
        owner:          owner
      };
    }
  }
  lifecycle { live -> live via pool.rebalance; }
  invariant no_undeclared_state;
}
"#;

/// A value-out spend whose body is a lone decrease `balance: balance - amount`
/// but whose guard does NOT bound `amount >= 0`. With `amount` free, z3 can pick a
/// NEGATIVE amount, making `balance' = balance - amount > balance` — the spend
/// MINTS value into the covenant. The M5 spend VC must REFUTE this (state total
/// increased). Sema's structural checks are kept off by declaring only
/// `no_undeclared_state`. (The build-level direction; end-to-end, a `spend` would
/// normally also carry `non_negative_amount`, pre-empting this — defence in depth.)
const SPEND_MINTS_VALUE: &str = r#"pragma portrait ^0.1.0;
app LeakySpend {
  role vault {
    param pubkey owner;
    param int    balance;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(sig auth, int amount) : (pubkey owner, int balance) {
      requires checkSig(auth, owner);
      return LeakySpend { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via vault.spend; }
  invariant no_undeclared_state;
}
"#;

/// A value-out spend with a SELF-CONTRADICTORY guard (`amount >= 5` AND
/// `amount <= 3`), so the transition relation `T` is unsatisfiable (unreachable).
/// `(T ∧ ¬VC)` is then vacuously `unsat`; the SAT(T) vacuity guard must report
/// UNKNOWN for the spend class too — never a false PROVED on an unreachable spend.
const SPEND_VACUOUS_GUARD: &str = r#"pragma portrait ^0.1.0;
app VacuousSpend {
  role vault {
    param pubkey owner;
    param int    balance;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(sig auth, int amount) : (pubkey owner, int balance) {
      requires checkSig(auth, owner);
      requires amount >= 5;
      requires amount <= 3;
      return VacuousSpend { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via vault.spend; }
  invariant no_undeclared_state;
}
"#;

// ── Helpers ───────────────────────────────────────────────────────────────

fn checked_program(src: &str) -> Program {
    let prog = parse(src).expect("fixture should parse");
    portrait_sema::check(&prog).expect("fixture should pass sema");
    prog
}

/// The single **value-conservation** VC for a fixture. `prove_program` now also
/// emits range / refinement / preservation VCs for the same entrypoint; these
/// conservation tests isolate the conservation class by `VcKind`.
fn single_vc(src: &str) -> VcReport {
    let prog = checked_program(src);
    let reports = prove_program(&prog).expect("prove_program should not refuse");
    let mut conservation: Vec<VcReport> = reports
        .into_iter()
        .filter(|r| r.vc_kind == VcKind::ValueConservation)
        .collect();
    assert_eq!(
        conservation.len(),
        1,
        "fixture should yield exactly one value-conservation VC"
    );
    conservation.pop().unwrap()
}

/// The single **spend** VC for a fixture.
fn single_spend_vc(src: &str) -> VcReport {
    let prog = checked_program(src);
    let reports = prove_program(&prog).expect("prove_program should not refuse");
    let mut spend: Vec<VcReport> = reports
        .into_iter()
        .filter(|r| r.vc_kind == VcKind::Spend)
        .collect();
    assert_eq!(spend.len(), 1, "fixture should yield exactly one spend VC");
    spend.pop().unwrap()
}

/// Returns `false` (and prints a skip notice) when z3 is unreachable, so a
/// solver-gated test can return early without a silent pass.
fn z3_or_skip(test: &str) -> bool {
    if portrait_lens::z3_available() {
        return true;
    }
    eprintln!("SKIP {test}: z3 not found on PATH or $PORTRAIT_Z3 (solver-gated test)");
    false
}

// ── Generation tests (no solver needed) ──────────────────────────────────────

#[test]
fn internalsplit_v_is_exactly_the_three_legs() {
    // The wide split predicate must pick up all three int-typed *balance legs and
    // NOT the owner pubkey. (Using the narrow predicate would empty V and
    // spuriously prove — the red-team trap.)
    let prog = checked_program(&internalsplit_correct_src());
    let role = &prog.app.roles[0];
    let v: Vec<&str> = value_bearing_fields(role)
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        v,
        vec!["pool_a_balance", "pool_b_balance", "pool_c_balance"]
    );
}

#[test]
fn smtlib_negates_the_vc_and_sums_over_v() {
    // The generated document must contain the negated conservation VC — a VC that
    // forgets to negate asks z3 the wrong question.
    let vc = single_vc(&internalsplit_correct_src());
    assert!(
        vc.smtlib.contains(
            "(assert (not (= (+ pool_a_balance_p pool_b_balance_p pool_c_balance_p) \
             (+ pool_a_balance pool_b_balance pool_c_balance))))"
        ),
        "missing negated conservation VC over V; got:\n{}",
        vc.smtlib
    );
    // checkSig stays uninterpreted (declared, NOT asserted true).
    assert!(vc
        .smtlib
        .contains("(declare-fun checkSig (Sig PubKey) Bool)"));
    assert!(!vc.smtlib.contains("(assert (= (checkSig"));
}

#[test]
fn multisig_treasury_spend_emits_a_spend_vc_not_a_conservation_vc() {
    // MultisigTreasury.spend is a value-OUT spend (V={balance}, lone decrease).
    // M5 (formerly Q3-deferred): Lens must NOT apply the internal-flow
    // conservation VC (which would falsely REFUTE a legitimate spend); instead it
    // emits a dedicated `VcKind::Spend` VC over the SAME SAT(T) vacuity-safe
    // discharge. The spend VC's full query must declare the fresh spent_out var,
    // constrain it non-negative, bind it to the drop, and negate as "value
    // created" (state total increased). The vacuity probe stays T alone.
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../library/governance/treasury/MultisigTreasury.portrait"
    ))
    .expect("MultisigTreasury.portrait should exist");
    let prog = checked_program(&src);
    let reports = prove_program(&prog).expect("prove_program should not refuse");
    assert!(
        !reports
            .iter()
            .any(|r| r.vc_kind == VcKind::ValueConservation),
        "a value-out spend must NOT emit the internal-flow conservation VC"
    );
    let spend: Vec<&VcReport> = reports
        .iter()
        .filter(|r| r.vc_kind == VcKind::Spend)
        .collect();
    assert_eq!(spend.len(), 1, "exactly one spend VC for the spend entry");
    let vc = spend[0];
    assert!(vc.smtlib.contains("(declare-const spent_out Int)"));
    assert!(
        vc.smtlib
            .contains("(assert (= spent_out (- balance balance_p)))"),
        "spent_out must be bound to the drop Σf − Σf'; got:\n{}",
        vc.smtlib
    );
    // Negated VC: spent_out < 0, i.e. the state total INCREASED (value created).
    // `(>= spent_out 0)` must NOT be asserted as a premise (it is the obligation).
    assert!(
        vc.smtlib.contains("(assert (< spent_out 0))"),
        "negated spend VC must assert spent_out < 0 (value-created); got:\n{}",
        vc.smtlib
    );
    assert!(
        !vc.smtlib.contains("(assert (>= spent_out 0))"),
        "spent_out >= 0 is the OBLIGATION, never a premise; got:\n{}",
        vc.smtlib
    );
    // The vacuity probe is T ALONE — no spent_out, no negated VC.
    assert!(!vc.transition_smtlib.contains("spent_out"));
}

#[test]
fn multisig_treasury_spend_is_proved_and_cross_checked() {
    // The honest M5 verdict: a conserving/decreasing spend CREATES NO value, so
    // the spend VC must discharge to PROVED — over a SATISFIABLE T (the guards are
    // consistent), and CONFIRMED by the independent cross-check run. z3-gated.
    if !z3_or_skip("multisig_treasury_spend_is_proved_and_cross_checked") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../library/governance/treasury/MultisigTreasury.portrait"
    ))
    .expect("MultisigTreasury.portrait should exist");
    let prog = checked_program(&src);
    let mut reports = prove_program(&prog).expect("prove_program should not refuse");
    let vc = reports
        .iter_mut()
        .find(|r| r.vc_kind == VcKind::Spend)
        .expect("a spend VC");
    discharge(vc, TIMEOUT_MS);
    assert!(
        matches!(vc.outcome, Some(Outcome::Proved { .. })),
        "a conserving spend creates no value ⇒ PROVED; got {:?}",
        vc.outcome
    );
}

#[test]
fn mint_entrypoint_emits_no_conservation_vc() {
    // An entrypoint named mint* is an authorised supply change (parity with
    // sema's is_mint_or_burn) — no conservation VC, so it is never spuriously
    // REFUTED.
    let src = r#"pragma portrait ^0.1.0;
app Minter {
  role token {
    param int    supply;
    param pubkey owner;
    state { int supply; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function mint(sig auth, int amount) : (int supply, pubkey owner) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      return Minter { supply: supply + amount, owner: owner };
    }
  }
  lifecycle { live -> live via token.mint; }
  invariant no_undeclared_state;
}
"#;
    let prog = checked_program(src);
    let reports = prove_program(&prog).expect("prove_program should not refuse");
    assert!(
        !reports
            .iter()
            .any(|r| r.vc_kind == VcKind::ValueConservation),
        "mint must be exempt from conservation"
    );
}

// ── Discharge tests (z3-gated) ────────────────────────────────────────────────

#[test]
fn correct_internalsplit_is_proved() {
    if !z3_or_skip("correct_internalsplit_is_proved") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_vc(&internalsplit_correct_src());
    discharge(&mut vc, TIMEOUT_MS);
    assert!(
        matches!(vc.outcome, Some(Outcome::Proved { .. })),
        "correct InternalSplit must be PROVED; got {:?}; smtlib:\n{}",
        vc.outcome,
        vc.smtlib
    );
}

#[test]
fn broken_internalsplit_is_refuted_with_a_counter_model() {
    if !z3_or_skip("broken_internalsplit_is_refuted_with_a_counter_model") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_vc(&internalsplit_broken_src());
    discharge(&mut vc, TIMEOUT_MS);
    match &vc.outcome {
        Some(Outcome::Refuted { model, .. }) => {
            assert!(
                !model.trim().is_empty(),
                "REFUTED must carry a non-empty counter-model"
            );
        }
        other => panic!("broken InternalSplit must be REFUTED, got {other:?}"),
    }
}

#[test]
fn doubling_conserving_is_proved_the_case_structural_d4_cannot_do() {
    if !z3_or_skip("doubling_conserving_is_proved_the_case_structural_d4_cannot_do") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_vc(DOUBLING_CONSERVING);
    discharge(&mut vc, TIMEOUT_MS);
    assert!(
        matches!(vc.outcome, Some(Outcome::Proved { .. })),
        "x*2 == x+x conservation must be PROVED (real arithmetic); got {:?}; smtlib:\n{}",
        vc.outcome,
        vc.smtlib
    );
}

#[test]
fn value_creating_transition_is_refuted() {
    if !z3_or_skip("value_creating_transition_is_refuted") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_vc(VALUE_CREATING);
    discharge(&mut vc, TIMEOUT_MS);
    assert!(
        matches!(vc.outcome, Some(Outcome::Refuted { .. })),
        "a value-creating transition must be REFUTED, got {:?}",
        vc.outcome
    );
}

#[test]
fn vacuous_transition_is_unknown_never_proved() {
    // SOUNDNESS (FIX 1): a self-contradictory guard makes the transition relation
    // `T` unsatisfiable, so `(T ∧ ¬VC)` is vacuously `unsat`. A naive mapping of
    // unsat→Proved would FALSELY prove conservation for an entrypoint that also
    // CREATES value. The vacuity check (T-alone must be SAT before unsat→Proved)
    // must catch this and report UNKNOWN, never PROVED.
    if !z3_or_skip("vacuous_transition_is_unknown_never_proved") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_vc(VALUE_CREATING_VACUOUS_GUARD);
    discharge(&mut vc, TIMEOUT_MS);
    match &vc.outcome {
        Some(Outcome::Unknown { reason }) => {
            assert!(
                reason.to_lowercase().contains("vacuous"),
                "vacuous-transition UNKNOWN should name the cause; got: {reason}"
            );
        }
        other => panic!(
            "a vacuous (self-contradictory) transition must be UNKNOWN, never PROVED; got {other:?}; smtlib:\n{}",
            vc.smtlib
        ),
    }
}

#[test]
fn value_creating_with_satisfiable_guard_still_refutes() {
    // The companion to the vacuity test: the IDENTICAL value-creating body, but
    // with a SATISFIABLE guard, must still be REFUTED — the vacuity check must not
    // suppress genuine refutations.
    if !z3_or_skip("value_creating_with_satisfiable_guard_still_refutes") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_vc(VALUE_CREATING);
    discharge(&mut vc, TIMEOUT_MS);
    assert!(
        matches!(vc.outcome, Some(Outcome::Refuted { .. })),
        "a value-creating transition with a satisfiable guard must REFUTE, got {:?}",
        vc.outcome
    );
}

#[test]
fn spend_that_mints_value_is_refuted() {
    // SOUNDNESS: the spend VC must not be a rubber stamp. A spend whose body can
    // CREATE value (free `amount` can go negative ⇒ `balance' > balance`) must be
    // REFUTED with a counter-model — over a SATISFIABLE T.
    if !z3_or_skip("spend_that_mints_value_is_refuted") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_spend_vc(SPEND_MINTS_VALUE);
    discharge(&mut vc, TIMEOUT_MS);
    match &vc.outcome {
        Some(Outcome::Refuted { model, .. }) => assert!(
            !model.trim().is_empty(),
            "a value-minting spend REFUTED must carry a counter-model"
        ),
        other => panic!(
            "a spend whose body creates value must be REFUTED; got {other:?}; smtlib:\n{}",
            vc.smtlib
        ),
    }
}

#[test]
fn spend_with_vacuous_guard_is_unknown_never_proved() {
    // The spend class routes through the SAME SAT(T) vacuity guard: an unreachable
    // (self-contradictory-guard) spend must report UNKNOWN, never a false PROVED.
    if !z3_or_skip("spend_with_vacuous_guard_is_unknown_never_proved") {
        return;
    }
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_spend_vc(SPEND_VACUOUS_GUARD);
    discharge(&mut vc, TIMEOUT_MS);
    match &vc.outcome {
        Some(Outcome::Unknown { reason }) => assert!(
            reason.to_lowercase().contains("vacuous"),
            "vacuous spend UNKNOWN should name the cause; got: {reason}"
        ),
        other => panic!(
            "a vacuous-guard spend must be UNKNOWN, never PROVED; got {other:?}; smtlib:\n{}",
            vc.smtlib
        ),
    }
}

#[test]
fn z3_absent_yields_unknown_never_proved() {
    // Soundness must not depend on the solver being present. Point PORTRAIT_Z3 at
    // a non-existent binary and assert discharge falls to UNKNOWN (the skip
    // message), never Proved/Refuted.
    let _g = SOLVER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vc = single_vc(&internalsplit_correct_src());
    // Save/clear any real override so this test is hermetic.
    let prev = std::env::var("PORTRAIT_Z3").ok();
    std::env::set_var("PORTRAIT_Z3", "/nonexistent/definitely-not-z3-xyz");
    discharge(&mut vc, TIMEOUT_MS);
    match prev {
        Some(p) => std::env::set_var("PORTRAIT_Z3", p),
        None => std::env::remove_var("PORTRAIT_Z3"),
    }
    match vc.outcome {
        Some(Outcome::Unknown { reason }) => {
            assert!(
                reason.contains("UNKNOWN, never PROVED"),
                "z3-absent reason should carry the skip message, got: {reason}"
            );
        }
        other => panic!("z3-absent must yield UNKNOWN, got {other:?}"),
    }
}
