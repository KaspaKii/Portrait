//! M2 lift + emission tests.
//!
//! Faithfulness is the load-bearing claim, so these pin BOTH directions:
//!
//! - a real multi-role program (`DigitalReit`, via its lifecycle) lifts to a
//!   Score that `check()` ACCEPTS and emits one skeleton per role;
//! - a synthetic multi-role flow lifts and accepts;
//! - an UNSAFE flow (a non-decider diverging across a `choose` with no
//!   notification) lifts to a Score that `check()` REJECTS with the right named
//!   error — the lift does NOT paper over it;
//! - un-liftable shapes (single role, self-interaction) return named
//!   [`LiftError`]s rather than a silently-wrong Score.

use super::*;
use crate::ComposeError;
use portrait_syntax::{App, CovenantMode, Edge, Entry, Flow, Role as SynRole, Step};

// ----- fixture builders ------------------------------------------------------

fn role(name: &str) -> SynRole {
    SynRole {
        name: name.to_string(),
        component: None,
        params: vec![],
        state: vec![],
        entrypoints: vec![],
    }
}

fn entry(name: &str) -> Entry {
    Entry {
        name: name.to_string(),
        mode: CovenantMode::Transition,
        args: vec![],
        returns: None,
        requires: vec![],
        body: vec![],
    }
}

fn role_with_entries(name: &str, entries: &[&str]) -> SynRole {
    SynRole {
        name: name.to_string(),
        component: None,
        params: vec![],
        state: vec![],
        entrypoints: entries.iter().map(|e| entry(e)).collect(),
    }
}

fn mv(role: &str, entry: &str) -> Step {
    Step::Move {
        role: role.to_string(),
        entry: entry.to_string(),
    }
}

fn flow(steps: Vec<Step>) -> Flow {
    Flow { steps }
}

// ===== ACCEPT: the real DigitalReit, via its lifecycle =======================

/// `DigitalReit` is the canonical real multi-role Portrait program: two roles
/// (`token`, `splitter`) and a cross-role lifecycle
/// (`token.distribute → splitter.payout`). It has NO `flow {}` block, so the lift
/// must fall back to the lifecycle — and the lifted Score must pass `check()`.
#[test]
fn digital_reit_lifecycle_lifts_checks_and_emits_two_skeletons() {
    let app = App {
        name: "DigitalReit".to_string(),
        roles: vec![role("token"), role("splitter")],
        lifecycle: vec![
            Edge {
                from: "issued".to_string(),
                to: "distributing".to_string(),
                via_role: "token".to_string(),
                via_entry: "distribute".to_string(),
                terminal: false,
            },
            Edge {
                from: "distributing".to_string(),
                to: "distributing".to_string(),
                via_role: "splitter".to_string(),
                via_entry: "payout".to_string(),
                terminal: false,
            },
        ],
        flow: None,
        invariants: vec![],
    };

    let score = lift(&app).expect("DigitalReit lifecycle must lift");
    let locals = score
        .check()
        .expect("the lifted DigitalReit Score must pass all checks");

    // token's view leads with a send of `distribute` to splitter.
    match locals.get("token").unwrap() {
        Local::Send { to, message, .. } => {
            assert_eq!(to, "splitter");
            assert_eq!(message, "distribute");
        }
        other => panic!("token should lead with a send; got {other:?}"),
    }
    // splitter's view leads with a receive of `distribute` from token.
    match locals.get("splitter").unwrap() {
        Local::Recv { from, message, .. } => {
            assert_eq!(from, "token");
            assert_eq!(message, "distribute");
        }
        other => panic!("splitter should lead with a recv; got {other:?}"),
    }

    // Exactly two role skeletons emitted, one per role.
    let skels = emit_role_skeletons(&locals);
    assert_eq!(skels.len(), 2);
    let names: Vec<&str> = skels.iter().map(|s| s.role.as_str()).collect();
    assert!(names.contains(&"splitter"));
    assert!(names.contains(&"token"));

    // token authorises `distribute`; splitter awaits it and authorises `payout`.
    let token = skels.iter().find(|s| s.role == "token").unwrap();
    assert!(token.entrypoints.iter().any(|e| e.message == "distribute"));
    let splitter = skels.iter().find(|s| s.role == "splitter").unwrap();
    assert!(splitter.awaits.iter().any(|a| a.message == "distribute"));
    assert!(splitter.entrypoints.iter().any(|e| e.message == "payout"));
}

// ===== ACCEPT: a synthetic multi-role flow ===================================

/// A three-role explicit `flow {}` lifts to a connected handoff chain that
/// `check()` accepts, and emits three distinct skeletons.
#[test]
fn three_role_flow_lifts_and_accepts() {
    let app = App {
        name: "Chain".to_string(),
        roles: vec![
            role_with_entries("A", &["a"]),
            role_with_entries("B", &["b"]),
            role_with_entries("C", &["c"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![mv("A", "a"), mv("B", "b"), mv("C", "c")])),
        invariants: vec![],
    };
    let score = lift(&app).expect("three-role flow must lift");
    let locals = score.check().expect("the lifted chain must check");
    assert_eq!(locals.len(), 3);
    let skels = emit_role_skeletons(&locals);
    assert_eq!(skels.len(), 3);
}

// ===== REJECT: an UNSAFE flow lifts to a Score that check() rejects ==========

/// A `choose` in which a non-decider role (`C`) acts on one branch but not the
/// other, with NO notification, is a classic orphan-wait. The lift must NOT
/// paper over it: it lifts faithfully to a `Branching`, and `check()` then
/// rejects with `NotProjectable` naming `C`.
#[test]
fn unsafe_choose_lifts_then_check_rejects_not_projectable() {
    // Branch `x`: A.x then C.z (C acts).  Branch `y`: A.y (C does nothing).
    // A decides (leads both branches); C is neither decider nor informed and
    // diverges with no distinguishing notification.
    let app = App {
        name: "Unsafe".to_string(),
        roles: vec![
            role_with_entries("A", &["x", "y"]),
            role_with_entries("B", &["b"]),
            role_with_entries("C", &["z"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![
            Step::Choose(vec![
                flow(vec![mv("A", "x"), mv("C", "z")]),
                flow(vec![mv("A", "y")]),
            ]),
            // a trailing move so the choice has a successor (informed role).
            mv("B", "b"),
        ])),
        invariants: vec![],
    };
    let score = lift(&app).expect("the unsafe program must still LIFT (faithfully)");
    match score.check() {
        Err(ComposeError::NotProjectable { role, .. }) => {
            assert_eq!(role, "C", "the orphan-wait role must be named");
        }
        other => panic!("expected NotProjectable(C) on the lifted unsafe Score; got {other:?}"),
    }
}

// ===== REJECT (lift-level): un-liftable shapes are NAMED LiftErrors ==========

/// A single-role program has no inter-role interaction; the lift refuses rather
/// than fabricating a counterparty.
#[test]
fn single_role_program_is_a_named_lift_error() {
    let app = App {
        name: "Solo".to_string(),
        roles: vec![role_with_entries("only", &["go"])],
        lifecycle: vec![Edge {
            from: "live".to_string(),
            to: "live".to_string(),
            via_role: "only".to_string(),
            via_entry: "go".to_string(),
            terminal: false,
        }],
        flow: None,
        invariants: vec![],
    };
    match lift(&app) {
        Err(LiftError::SingleRole { role }) => assert_eq!(role, "only"),
        other => panic!("expected SingleRole; got {other:?}"),
    }
}

/// A program with two roles but a flow that names an undeclared acting role is a
/// named `UnknownRole` error, not a silently-wrong Score.
#[test]
fn unknown_acting_role_is_a_named_lift_error() {
    let app = App {
        name: "Bad".to_string(),
        roles: vec![
            role_with_entries("A", &["a"]),
            role_with_entries("B", &["b"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![mv("A", "a"), mv("Z", "zap")])),
        invariants: vec![],
    };
    match lift(&app) {
        Err(LiftError::UnknownRole { role, entry }) => {
            assert_eq!(role, "Z");
            assert_eq!(entry, "zap");
        }
        other => panic!("expected UnknownRole(Z); got {other:?}"),
    }
}

/// No flow and no lifecycle → `NoFlow`.
#[test]
fn no_flow_no_lifecycle_is_a_named_lift_error() {
    let app = App {
        name: "Empty".to_string(),
        roles: vec![role("A"), role("B")],
        lifecycle: vec![],
        flow: None,
        invariants: vec![],
    };
    match lift(&app) {
        Err(LiftError::NoFlow) => {}
        other => panic!("expected NoFlow; got {other:?}"),
    }
}

/// A `repeat` body with nested control flow is refused with a named error rather
/// than silently mis-lifted.
#[test]
fn nested_control_in_repeat_is_a_named_lift_error() {
    let app = App {
        name: "BadLoop".to_string(),
        roles: vec![
            role_with_entries("A", &["x", "y"]),
            role_with_entries("B", &["b"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![Step::Repeat(
            2,
            Box::new(flow(vec![Step::Choose(vec![
                flow(vec![mv("A", "x")]),
                flow(vec![mv("A", "y")]),
            ])])),
        )])),
        invariants: vec![],
    };
    match lift(&app) {
        Err(LiftError::NestedControlInRepeat) => {}
        other => panic!("expected NestedControlInRepeat; got {other:?}"),
    }
}

// ===== a bounded `repeat` lifts to a guarded recursion that accepts ==========

#[test]
fn repeat_lifts_to_guarded_recursion() {
    // repeat(3) { A.tick; B.tock }  →  μX. A→B[tick] . B→A[tock] . X
    let app = App {
        name: "Loop".to_string(),
        roles: vec![
            role_with_entries("A", &["tick"]),
            role_with_entries("B", &["tock"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![Step::Repeat(
            3,
            Box::new(flow(vec![mv("A", "tick"), mv("B", "tock")])),
        )])),
        invariants: vec![],
    };
    let score = lift(&app).expect("repeat must lift");
    let locals = score.check().expect("guarded recursion must check");
    // A's projection should be a Rec.
    match locals.get("A").unwrap() {
        Local::Rec(_, _) => {}
        other => panic!("expected Rec for A; got {other:?}"),
    }
}

// ===== FAITHFULNESS: nothing sequenced after a control construct is dropped ==

/// CRITICAL regression guard (red-team finding). A flow step sequenced AFTER a
/// `repeat` must NOT be silently dropped from the lifted Score. Here the
/// post-repeat tail is an UNSAFE orphan-wait `choose` (non-decider `C` diverges
/// on one branch with no notification — genuinely `NotProjectable(C)`). The OLD
/// `append_global` returned the `Rec` unchanged and DROPPED the tail, so the
/// lifted Score described a DIFFERENT, truncated program and `check()` ACCEPTED
/// it — the worst-possible lift unfaithfulness (`docs/COMPOSER-M0-DESIGN.md
/// §4.4`). The repeat is modelled as an UNBOUNDED loop with no exit, so the tail
/// is genuinely unreachable in the model; the lift must therefore REFUSE with a
/// named error rather than either dropping it or fabricating an exit. The crucial
/// property: the unsafe program is NOT accepted.
#[test]
fn unsafe_choose_after_repeat_is_not_dropped_lift_refuses() {
    // repeat(1){ A.tick; B.tock };  choose{ A.x; C.z | A.y };  B.b
    let app = App {
        name: "TailAfterRepeat".to_string(),
        roles: vec![
            role_with_entries("A", &["tick", "x", "y"]),
            role_with_entries("B", &["tock", "b"]),
            role_with_entries("C", &["z"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![
            Step::Repeat(1, Box::new(flow(vec![mv("A", "tick"), mv("B", "tock")]))),
            Step::Choose(vec![
                flow(vec![mv("A", "x"), mv("C", "z")]),
                flow(vec![mv("A", "y")]),
            ]),
            mv("B", "b"),
        ])),
        invariants: vec![],
    };
    // The lift must NOT silently produce an accepting Score for this unsafe,
    // truncated program. The honest result is a named refusal at lift time.
    match lift(&app) {
        Err(LiftError::TrailingStepsAfterUnboundedRepeat) => {}
        Ok(score) => panic!(
            "unsafe post-repeat tail must NOT lift to an accepting Score; \
             check() said {:?}",
            score.check()
        ),
        other => panic!("expected TrailingStepsAfterUnboundedRepeat; got {other:?}"),
    }
}

/// Even a SAFE flow sequenced after a `repeat` cannot be faithfully represented:
/// the repeat lifts to an unbounded `μX. body . X` with no exit, so there is
/// nowhere to attach the tail. The lift must REFUSE with a named error rather
/// than SILENTLY DROP the tail (the old behaviour, which made roles that only act
/// in the tail vanish from the Score). Honesty over silent truncation.
#[test]
fn safe_tail_after_repeat_is_a_named_lift_error_not_dropped() {
    // repeat(2){ A.tick; B.tock };  B.handoff(to C);  C.done
    let app = App {
        name: "SafeTail".to_string(),
        roles: vec![
            role_with_entries("A", &["tick"]),
            role_with_entries("B", &["tock", "handoff"]),
            role_with_entries("C", &["done"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![
            Step::Repeat(2, Box::new(flow(vec![mv("A", "tick"), mv("B", "tock")]))),
            mv("B", "handoff"),
            mv("C", "done"),
        ])),
        invariants: vec![],
    };
    match lift(&app) {
        Err(LiftError::TrailingStepsAfterUnboundedRepeat) => {}
        other => {
            panic!("post-repeat tail must be a named refusal, never a silent drop; got {other:?}")
        }
    }
}

/// A `repeat` that is the LAST construct in the flow (no trailing steps) still
/// lifts cleanly — the fix must not over-refuse. This re-asserts the original
/// guarded-recursion accept path next to the new refusals.
#[test]
fn repeat_as_last_step_still_lifts_and_accepts() {
    let app = App {
        name: "LoopLast".to_string(),
        roles: vec![
            role_with_entries("A", &["tick"]),
            role_with_entries("B", &["tock"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![Step::Repeat(
            2,
            Box::new(flow(vec![mv("A", "tick"), mv("B", "tock")])),
        )])),
        invariants: vec![],
    };
    let score = lift(&app).expect("a trailing repeat must still lift");
    score.check().expect("guarded recursion must still check");
}

/// Secondary drop mechanism (red-team note): a shared post-`choose` tail appended
/// onto a branch whose body ends in a `repeat` (a `Rec` leaf) was also silently
/// dropped by the old `append_global`. With the fix, that append hits the `Rec`
/// boundary and the lift refuses with the named error rather than truncating.
#[test]
fn shared_choose_tail_onto_branch_ending_in_repeat_is_refused() {
    // choose{ B.x; repeat(2){A.tick;B.tock} | B.y };  A.done
    // Branch `x` leads with B (so B is the decider/branch-leader), then a repeat
    // → the branch's lifted body ends in a `Rec`; the shared tail `A.done` cannot
    // be appended past that boundary. (B.x hands the step to A.tick, so no
    // self-interaction fires first — this isolates the shared-tail-onto-Rec path.)
    let app = App {
        name: "ChoiceWithLoopBranch".to_string(),
        roles: vec![
            role_with_entries("A", &["tick", "done"]),
            role_with_entries("B", &["x", "y", "tock"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![
            Step::Choose(vec![
                flow(vec![
                    mv("B", "x"),
                    Step::Repeat(2, Box::new(flow(vec![mv("A", "tick"), mv("B", "tock")]))),
                ]),
                flow(vec![mv("B", "y")]),
            ]),
            mv("A", "done"),
        ])),
        invariants: vec![],
    };
    match lift(&app) {
        Err(LiftError::TrailingStepsAfterUnboundedRepeat) => {}
        other => panic!(
            "a shared choose-tail onto a branch ending in a repeat must be refused, not dropped; got {other:?}"
        ),
    }
}

// ===== render helpers are non-empty and labelled =============================

// ===== END-TO-END opt-in harness: parse → lift → check → emit → footer =======

/// A self-contained two-covenant program (the DigitalReit shape) whose cross-role
/// `lifecycle` IS the parsed multi-role flow. Kept inline so the harness is
/// hermetic (no external file dependency).
const REIT_SRC: &str = r#"
pragma portrait ^0.1.0;
app DigitalReit {
  role token {
    state { int period; }
    #[covenant(mode = transition)]
    entrypoint function distribute() : (int period) { return period + 1; }
  }
  role splitter {
    state { int paid; }
    #[covenant(mode = transition)]
    entrypoint function payout() : (int paid) { return paid + 1; }
  }
  lifecycle {
    issued       -> distributing via token.distribute;
    distributing -> distributing via splitter.payout;
  }
  invariant value_conserved;
}
"#;

/// The full M2 pipeline on a PARSED program: parse → lift → check → emit. This is
/// the test-only stand-in for a `portrait compose <file>` subcommand (kept here to
/// avoid touching portrait-cli). Run with:
/// `cargo test -p portrait-compose -- --nocapture compose_lift_report`.
#[test]
fn compose_lift_report() {
    let program = portrait_syntax::parse(REIT_SRC).expect("REIT_SRC must parse");

    // It is genuinely multi-role with a cross-role lifecycle and no flow block —
    // exactly the dormant-carrier case M2 targets.
    assert_eq!(program.app.roles.len(), 2);
    assert!(
        program.app.flow.is_none(),
        "real programs carry the flow in lifecycle"
    );
    assert_eq!(program.app.lifecycle.len(), 2);

    let score = lift(&program.app).expect("parsed DigitalReit must lift");
    let verdict = score.check();

    println!(
        "\n=== portrait compose (M2) — {} (parsed) ===",
        program.app.name
    );
    match &verdict {
        Ok(locals) => {
            println!("\n-- per-role projection (local types) --");
            for (role, t) in locals {
                println!("  {role:<10} ⊢ {}", crate::render_local(t));
            }
            println!("\n{}", render_skeletons(&emit_role_skeletons(locals)));
            println!("VERDICT: ACCEPT — projectable, dual (no orphans), linear.");
        }
        Err(e) => println!("VERDICT: REJECT — {e}"),
    }
    println!("\n{}", crate::HONEST_BOUNDARY_FOOTER);

    let locals = verdict.expect("the parsed multi-role program must check");
    assert_eq!(locals.len(), 2);
    assert_eq!(emit_role_skeletons(&locals).len(), 2);
}

// ===== END-TO-END (M4): an AUTHORED flow{} with `choose` → parse → lift → check

/// A self-contained two-role program whose multi-role interaction is authored
/// directly in a `flow {}` block using the M4 `choose` surface syntax. The
/// decider `a` leads both branches and `b` is the informed successor on each, so
/// the choice is projectable: this exercises the full M4 path —
/// surface syntax → parser → `App.flow` → existing M2 lift → `check()` ACCEPT.
const AUTHORED_CHOOSE_SRC: &str = r#"
pragma portrait ^0.1.0;
app AuthoredChoice {
  role a {
    #[covenant(mode = transition)]
    entrypoint function approve() : (int n) { return n; }
    #[covenant(mode = transition)]
    entrypoint function reject() : (int n) { return n; }
  }
  role b {
    #[covenant(mode = transition)]
    entrypoint function settle() : (int n) { return n; }
  }
  flow {
    choose {
      branch { a.approve; b.settle }
      branch { a.reject; b.settle }
    }
  }
}
"#;

#[test]
fn authored_choose_flow_parses_lifts_and_checks() {
    let program = portrait_syntax::parse(AUTHORED_CHOOSE_SRC).expect("authored source must parse");

    // The choose reached the AST as a real Step::Choose with two branches.
    let flow = program.app.flow.as_ref().expect("flow block must parse");
    assert_eq!(flow.steps.len(), 1);
    match &flow.steps[0] {
        Step::Choose(branches) => assert_eq!(branches.len(), 2, "two authored branches"),
        other => panic!("expected an authored Step::Choose; got {other:?}"),
    }

    // It then flows through the existing M2 lift + check unchanged, and ACCEPTs.
    let score = lift(&program.app).expect("authored choose must lift");
    let locals = score
        .check()
        .expect("the projectable authored choose must check");
    assert_eq!(locals.len(), 2);
    assert_eq!(emit_role_skeletons(&locals).len(), 2);
}

/// An authored `repeat` flow likewise parses and lifts to a guarded recursion
/// that checks — pinning the M4 `repeat <N> { .. }` surface syntax end to end.
const AUTHORED_REPEAT_SRC: &str = r#"
pragma portrait ^0.1.0;
app AuthoredLoop {
  role a {
    #[covenant(mode = transition)]
    entrypoint function tick() : (int n) { return n; }
  }
  role b {
    #[covenant(mode = transition)]
    entrypoint function tock() : (int n) { return n; }
  }
  flow {
    repeat 3 { a.tick; b.tock }
  }
}
"#;

#[test]
fn authored_repeat_flow_parses_lifts_and_checks() {
    let program = portrait_syntax::parse(AUTHORED_REPEAT_SRC).expect("authored source must parse");
    let flow = program.app.flow.as_ref().expect("flow block must parse");
    match &flow.steps[0] {
        Step::Repeat(3, body) => assert_eq!(body.steps.len(), 2),
        other => panic!("expected an authored Step::Repeat(3, ..); got {other:?}"),
    }
    let score = lift(&program.app).expect("authored repeat must lift");
    score.check().expect("guarded recursion must check");
}

#[test]
fn render_skeletons_is_labelled_as_skeleton() {
    let app = App {
        name: "Chain".to_string(),
        roles: vec![
            role_with_entries("A", &["a"]),
            role_with_entries("B", &["b"]),
        ],
        lifecycle: vec![],
        flow: Some(flow(vec![mv("A", "a"), mv("B", "b")])),
        invariants: vec![],
    };
    let locals = lift(&app).unwrap().check().unwrap();
    let rendered = render_skeletons(&emit_role_skeletons(&locals));
    assert!(rendered.contains("SKELETON"));
    assert!(rendered.contains("NOT deployable"));
}
