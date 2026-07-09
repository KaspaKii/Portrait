//! M5 executor tests.
//!
//! - the well-formed escrow COMPLETES with the correct resource movements;
//! - an ill-formed / deadlocking protocol reports `Stuck` (NOT a false `Completed`);
//! - a recursive protocol reports `LoopBounded` (bounded, not a false complete);
//! - authored choices drive a specific branch.
//!
//! HONESTY: a `Completed` here is the MODEL completing under cooperative
//! scheduling — NOT on-chain liveness (see the executor footer).

use super::*;
use crate::{asset_escrow_example, Global, Resource, Score, Sort};

#[test]
fn well_formed_escrow_completes_with_correct_movements() {
    let score = asset_escrow_example();
    // Author the `release` arm so the trace reaches `settle` (payment -> Seller).
    let trace = execute_with_choices(&score, &["release"]);
    assert_eq!(
        trace.status,
        Status::Completed,
        "well-formed escrow must complete; trace: {:?}",
        trace
    );
    // First event is Buyer funding, carrying both payment (Value) and step
    // (Continuation).
    let first = &trace.events[0];
    assert_eq!(first.from, "Buyer");
    assert_eq!(first.message, "fund");
    assert!(first.resources.contains(&"payment".to_string()));
    assert!(first.resources.contains(&"step".to_string()));
    // The release arm must include Buyer settling payment to Seller.
    assert!(
        trace
            .events
            .iter()
            .any(|e| e.message == "settle" && e.from == "Buyer" && e.to == "Seller"),
        "release arm should settle payment Buyer->Seller; events: {:?}",
        trace.events
    );
}

#[test]
fn escrow_refund_arm_also_completes() {
    let score = asset_escrow_example();
    let trace = execute_with_choices(&score, &["refund"]);
    assert_eq!(trace.status, Status::Completed);
    // The refund arm has no `settle` (payment discharged locally to Buyer).
    assert!(!trace.events.iter().any(|e| e.message == "settle"));
}

#[test]
fn broken_handoff_is_stuck_not_false_completed() {
    // C carries the `step` Continuation it never received: A hands `step` to B,
    // then C (not the holder) tries to carry `step` — a broken handoff. The
    // executor must wedge into `Stuck`, NOT report a false `Completed`.
    let global = Global::interact(
        "A",
        "fund",
        "B",
        &["step"],
        Global::interact("C", "steal", "B", &["step"], Global::End),
    );
    let score = Score::new(
        &["A", "B", "C"],
        vec![Resource::new("step", Sort::Continuation)],
        global,
    );
    let trace = execute(&score);
    assert!(
        matches!(trace.status, Status::Stuck { .. }),
        "broken handoff must be Stuck, got {:?}",
        trace.status
    );
    // It fired the first (legal) step before wedging.
    assert_eq!(trace.events.len(), 1);
    assert_eq!(trace.events[0].message, "fund");
}

#[test]
fn double_delivery_of_escrowed_value_is_stuck() {
    // A escrows a Value `coin` to B, then re-delivers the SAME coin to a DIFFERENT
    // receiver C — the same linear token delivered twice. The executor wedges.
    let global = Global::interact(
        "A",
        "pay",
        "B",
        &["coin"],
        Global::interact("A", "pay2", "C", &["coin"], Global::End),
    );
    let score = Score::new(
        &["A", "B", "C"],
        vec![Resource::new("coin", Sort::Value)],
        global,
    );
    let trace = execute(&score);
    assert!(
        matches!(trace.status, Status::Stuck { .. }),
        "double-delivery must be Stuck, got {:?}",
        trace.status
    );
}

#[test]
fn recursive_protocol_reports_loop_bounded() {
    // μX. (A ->[tick] B {step} . B ->[tock] A {step} . X) — a never-ending
    // cooperative ping-pong loop where the `step` Continuation returns to the next
    // sender each iteration (so the handoff stays connected). The executor unfolds
    // to the bound and reports LoopBounded (not a false complete, not Stuck).
    let pingpong = Global::rec(
        "X",
        Global::interact(
            "A",
            "tick",
            "B",
            &["step"],
            Global::interact("B", "tock", "A", &["step"], Global::var("X")),
        ),
    );
    let score = Score::new(
        &["A", "B"],
        vec![Resource::new("step", Sort::Continuation)],
        pingpong,
    );
    let trace = execute(&score);
    assert_eq!(
        trace.status,
        Status::LoopBounded,
        "recursive protocol should be LoopBounded; trace: {:?}",
        trace
    );
    assert!(
        !trace.events.is_empty(),
        "it should have fired some iterations"
    );
}

#[test]
fn render_trace_has_footer_and_status() {
    let score = asset_escrow_example();
    let trace = execute_with_choices(&score, &["release"]);
    let out = render_trace(&trace);
    assert!(out.contains("M5 local executor trace"));
    assert!(out.contains("status: Completed"));
    assert!(out.contains("IN-MEMORY SIMULATION"));
    assert!(out.contains("does NOT imply on-chain liveness"));
}
