//! M5 real-emission tests — the load-bearing honesty assertions.
//!
//! Every emitted per-role covenant MUST genuinely round-trip:
//!
//!   * `portrait_syntax::parse(&source)` is `Ok`, AND
//!   * `portrait_sema::check(&parsed)` is `Ok`.
//!
//! These are asserted on the REAL parse + check results, not a self-claim.

use super::*;
use crate::{asset_escrow_example, Global, Resource, Score, Sort};

/// Assert a single emitted covenant genuinely parses AND sema-checks.
fn assert_round_trips(c: &RealCovenant) {
    let parsed = portrait_syntax::parse(&c.source).unwrap_or_else(|e| {
        panic!(
            "emitted covenant for `{}` failed to PARSE: {e}\n--- source ---\n{}",
            c.role, c.source
        )
    });
    portrait_sema::check(&parsed).unwrap_or_else(|diags| {
        let msgs: Vec<String> = diags.into_iter().map(|d| d.message).collect();
        panic!(
            "emitted covenant for `{}` failed SEMA check: {}\n--- source ---\n{}",
            c.role,
            msgs.join("; "),
            c.source
        )
    });
}

#[test]
fn escrow_every_role_covenant_parses_and_checks() {
    let score = asset_escrow_example();
    let locals = score.check().expect("escrow checks");
    let covenants = emit_real_covenants(&locals);
    assert_eq!(covenants.len(), 3, "Buyer, Seller, Arbiter");
    for c in &covenants {
        assert_round_trips(c);
    }
}

#[test]
fn escrow_buyer_emits_its_authorised_entrypoints() {
    let score = asset_escrow_example();
    let locals = score.check().unwrap();
    let covenants = emit_real_covenants(&locals);
    let buyer = covenants.iter().find(|c| c.role == "Buyer").unwrap();
    // Buyer authorises `fund`, and (on the release arm) `settle`, plus the
    // verdict relays. At minimum `fund` must appear as a real entrypoint.
    assert!(
        buyer.source.contains("entrypoint function fund"),
        "Buyer covenant should declare `fund`:\n{}",
        buyer.source
    );
    assert!(buyer.gaps.is_empty(), "no gaps expected for escrow Buyer");
}

#[test]
fn pure_receiver_emits_valid_empty_role_covenant() {
    // A two-party flow where one role only ever receives (no authorised sends):
    // its emitted covenant has an empty role body + no lifecycle, and still
    // parses + checks (the honest minimal subset).
    let global = Global::interact("A", "ping", "B", &["step"], Global::End);
    let score = Score::new(
        &["A", "B"],
        vec![Resource::new("step", Sort::Continuation)],
        global,
    );
    let locals = score.check().unwrap();
    let covenants = emit_real_covenants(&locals);
    let b = covenants.iter().find(|c| c.role == "B").unwrap();
    assert!(
        !b.source.contains("entrypoint function"),
        "B is a pure receiver — no entrypoints:\n{}",
        b.source
    );
    assert_round_trips(b);
}

#[test]
fn library_shaped_program_round_trips() {
    // A DigitalReit-shaped lifecycle-carried program (token.distribute ->
    // splitter.payout), lifted and projected, emits covenants that all check.
    use crate::lift::lift;
    use portrait_syntax::{App, CovenantMode, Edge, Entry, Role as SynRole};

    fn role_with_entries(name: &str, entries: &[&str]) -> SynRole {
        SynRole {
            name: name.to_string(),
            component: None,
            params: vec![],
            state: vec![],
            entrypoints: entries
                .iter()
                .map(|e| Entry {
                    name: e.to_string(),
                    mode: CovenantMode::Transition,
                    args: vec![],
                    returns: None,
                    requires: vec![],
                    body: vec![],
                })
                .collect(),
        }
    }
    let app = App {
        name: "DigitalReit".to_string(),
        roles: vec![
            role_with_entries("token", &["distribute"]),
            role_with_entries("splitter", &["payout"]),
        ],
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
    let score = lift(&app).expect("lifts");
    let locals = score.check().expect("checks");
    let covenants = emit_real_covenants(&locals);
    assert_eq!(covenants.len(), 2);
    for c in &covenants {
        assert_round_trips(c);
    }
}

#[test]
fn reserved_label_is_recorded_as_gap_not_emitted_broken() {
    // A message label that is a reserved keyword (`return`) cannot be an
    // entrypoint name; it is recorded as a gap and the covenant still checks.
    let global = Global::interact("A", "return", "B", &["step"], Global::End);
    let score = Score::new(
        &["A", "B"],
        vec![Resource::new("step", Sort::Continuation)],
        global,
    );
    let locals = score.check().unwrap();
    let covenants = emit_real_covenants(&locals);
    let a = covenants.iter().find(|c| c.role == "A").unwrap();
    assert!(
        matches!(a.gaps.first(), Some(EmitGap::ReservedLabel { label, .. }) if label == "return"),
        "reserved label should be a recorded gap: {:?}",
        a.gaps
    );
    // The covenant for A omitted the broken entrypoint but still round-trips.
    assert_round_trips(a);
    assert!(!a.source.contains("entrypoint function return"));
}

#[test]
fn non_ident_role_name_is_recorded_as_gap_not_falsely_claimed_real() {
    // A role name that is not a valid identifier (e.g. `My Role`) cannot be
    // spliced into `app {role}` / `role {role}` and still parse. M5 must NOT
    // claim such a covenant round-trips: either the emitted source genuinely
    // parses+checks, or the failure is recorded as a gap. Here we require the
    // gap path — an EmitGap::NonIdentRole — and that no broken source is
    // claimed real. (The role-label hole flagged by red-team ATK1b.)
    let global = Global::interact("My Role", "ping", "B", &["step"], Global::End);
    let score = Score::new(
        &["My Role", "B"],
        vec![Resource::new("step", Sort::Continuation)],
        global,
    );
    let locals = score.check().unwrap();
    let covenants = emit_real_covenants(&locals);
    let bad = covenants.iter().find(|c| c.role == "My Role").unwrap();
    // The non-ident role name MUST be recorded as a gap, not silently emitted.
    assert!(
        bad.gaps
            .iter()
            .any(|g| matches!(g, EmitGap::NonIdentRole { role } if role == "My Role")),
        "non-ident role name must be a recorded gap: {:?}",
        bad.gaps
    );
    // And whatever source is emitted must NOT be a false 'real' claim: every
    // RealCovenant whose source is presented as real must genuinely round-trip.
    // For the honest subset we emit a parse+check-clean stub (no broken `app`).
    assert_round_trips(bad);
}

#[test]
fn render_block_is_nonempty_and_labelled() {
    let score = asset_escrow_example();
    let locals = score.check().unwrap();
    let covenants = emit_real_covenants(&locals);
    let block = render_real_covenants(&covenants);
    assert!(block.contains("M5 real per-role covenants"));
    assert!(block.contains("role `Buyer`"));
}
