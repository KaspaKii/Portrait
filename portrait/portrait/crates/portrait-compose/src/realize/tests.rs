//! M3 realization tests.
//!
//! The three required directions, plus the honest-liveness distinction:
//!
//! - a realized Score has every role bound to ONE instance id (binding ACCEPTS)
//!   and every waiter has an escape (escape ACCEPTS) → NOT-STRANDED-BEYOND-T holds;
//! - a Score with a waiter lacking an escape → escape check REJECTS, naming the
//!   strandable role;
//! - a role covenant with a different / missing instance id → binding check
//!   REJECTS (no cross-instance splice);
//! - happy-path completion is NEVER claimed (honest-liveness distinction).

use super::*;
use crate::lift::{lift, RoleSkeleton, SkelEntry};
use crate::{asset_escrow_example, Global, Resource, Score, Sort};
use portrait_syntax::{App, CovenantMode, Edge, Entry, Role as SynRole};

// ----- fixture builders (mirror the lift tests) ------------------------------

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

/// The canonical real DigitalReit (lifecycle-carried), reused from the lift tests.
fn digital_reit_app() -> App {
    App {
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
    }
}

// ===== ACCEPT: a realized Score binds all roles + escapes every waiter ========

/// A realized Score: every role carries the SAME instance id (binding ACCEPTS),
/// every waiter carries an escape (escape ACCEPTS), so NOT-STRANDED-BEYOND-T holds.
#[test]
fn realized_score_binds_all_roles_and_escapes_every_waiter() {
    let app = digital_reit_app();
    let score = lift(&app).expect("DigitalReit must lift");
    let locals = score.check().expect("must check");
    let roles = realize(&score, &locals);

    // (1) binding: all roles share the one derived id.
    let id = realize_binding(&score, &roles).expect("binding must accept");
    assert_eq!(id, derive_instance_id(&score));
    for r in &roles {
        assert_eq!(r.instance, id, "every role bound to the SAME instance id");
    }

    // (2) escape: every waiter has an escape.
    has_escape_for_every_waiter(&roles).expect("every waiter must have an escape");

    // (3) liveness: NOT-STRANDED-BEYOND-T holds, completion NOT claimed.
    let report = liveness_report(&roles);
    assert!(report.not_stranded_beyond_t, "property must hold");
    assert!(report.strandable.is_empty());
    assert!(!report.waiters.is_empty(), "splitter waits on token");
    assert!(
        !report.happy_path_completion_guaranteed(),
        "completion is NEVER claimed"
    );
}

/// The same on the 3-party escrow worked example — more waiters, still all bound,
/// all escaped, property holds.
#[test]
fn escrow_realization_binds_and_escapes_all_waiters() {
    let score = asset_escrow_example();
    let locals = score.check().expect("escrow must check");
    let roles = realize(&score, &locals);

    realize_binding(&score, &roles).expect("escrow binding must accept");
    has_escape_for_every_waiter(&roles).expect("escrow waiters must all have escapes");

    let report = liveness_report(&roles);
    assert!(report.not_stranded_beyond_t);
    // Seller and Buyer both wait (deliver / settle / verdict); Arbiter only decides.
    assert!(report.waiters.contains(&"Seller".to_string()));
    assert!(!report.happy_path_completion_guaranteed());
}

// ===== REJECT: a waiter lacking an escape names the strandable role ===========

/// A realized role that is AT RISK (it escrowed a resource before a wait) but
/// carries NO escape covering it is strandable. The escape check must REJECT,
/// naming the strandable role; the liveness report must flag it and still refuse
/// to claim completion.
///
/// In DigitalReit the at-risk role is `token`: it SENDS `distribute{step}` to
/// splitter (escrowing `step`) and then AWAITS `payout{step}` back from splitter,
/// so token's own `step` is at risk if splitter goes silent. (splitter receives
/// before it sends, so splitter is NOT strandable — red-team fix.)
#[test]
fn waiter_without_escape_is_rejected_naming_the_strandable_role() {
    let app = digital_reit_app();
    let score = lift(&app).expect("must lift");
    let locals = score.check().expect("must check");
    let mut roles = realize(&score, &locals);

    // Strip the escapes off the AT-RISK role (token escrows `step` then awaits
    // payout), simulating an emission that forgot the timelock-escape branch.
    let token = roles
        .iter_mut()
        .find(|r| r.skeleton.role == "token")
        .expect("token present");
    assert!(
        token.is_waiter(),
        "token must be at risk (escrowed then awaits)"
    );
    token.escapes.clear();

    match has_escape_for_every_waiter(&roles) {
        Err(RealizeError::MissingEscape { role, waiting_on }) => {
            assert_eq!(role, "token", "the strandable role must be named");
            assert_eq!(waiting_on, "splitter");
        }
        other => panic!("expected MissingEscape(token); got {other:?}"),
    }

    // The liveness report agrees: property does NOT hold, role is flagged.
    let report = liveness_report(&roles);
    assert!(!report.not_stranded_beyond_t);
    assert!(report.strandable.contains(&"token".to_string()));
    assert!(!report.happy_path_completion_guaranteed());
}

// ===== REJECT: a different / missing instance id is a splice =================

/// A role covenant carrying a DIFFERENT instance id (e.g. realized against a
/// different Score) cannot be spliced into this protocol: the binding check
/// REJECTS with a named mismatch.
#[test]
fn role_with_different_instance_id_is_rejected_no_splice() {
    let app = digital_reit_app();
    let score_a = lift(&app).expect("A must lift");
    let locals_a = score_a.check().expect("A must check");
    let mut roles = realize(&score_a, &locals_a);

    // A structurally DIFFERENT protocol (instance B) derives a different id.
    let score_b = Score::new(
        &["X", "Y"],
        vec![Resource::new("step", Sort::Continuation)],
        Global::interact("X", "go", "Y", &["step"], Global::End),
    );
    let id_b = derive_instance_id(&score_b);
    assert_ne!(
        id_b,
        derive_instance_id(&score_a),
        "two different Scores derive different ids"
    );

    // Splice: stamp one of A's roles with B's id.
    roles[0].instance = id_b;
    let spliced_role = roles[0].skeleton.role.clone();

    match realize_binding(&score_a, &roles) {
        Err(RealizeError::InstanceIdMismatch {
            role,
            expected,
            found,
        }) => {
            assert_eq!(role, spliced_role);
            assert_eq!(expected, derive_instance_id(&score_a));
            assert_eq!(found, id_b);
        }
        other => panic!("expected InstanceIdMismatch; got {other:?}"),
    }
}

/// A role carrying a missing/zero id that does not match the derived id is also a
/// splice rejection (covers the "missing instance id" half of the requirement).
#[test]
fn role_with_missing_instance_id_is_rejected() {
    let app = digital_reit_app();
    let score = lift(&app).expect("must lift");
    let locals = score.check().expect("must check");
    let mut roles = realize(&score, &locals);

    // A "missing" id (the zero sentinel) almost certainly differs from the derived
    // id; assert that and then confirm the binding rejects it.
    let derived = derive_instance_id(&score);
    assert_ne!(
        derived,
        InstanceId(0),
        "derived id is not the zero sentinel"
    );
    roles[0].instance = InstanceId(0);

    match realize_binding(&score, &roles) {
        Err(RealizeError::InstanceIdMismatch { found, .. }) => {
            assert_eq!(found, InstanceId(0));
        }
        other => panic!("expected InstanceIdMismatch for a missing id; got {other:?}"),
    }
}

// ===== instance id determinism + structure-sensitivity =======================

/// The instance id is deterministic (same Score → same id) and structure
/// sensitive (a changed protocol → a changed id). Both halves are load-bearing
/// for the no-splice property.
#[test]
fn instance_id_is_deterministic_and_structure_sensitive() {
    let app = digital_reit_app();
    let score = lift(&app).expect("must lift");
    assert_eq!(
        derive_instance_id(&score),
        derive_instance_id(&score),
        "same Score → same id (deterministic)"
    );

    // A different message label → a different id.
    let other = Score::new(
        &["token", "splitter"],
        vec![Resource::new("step", Sort::Continuation)],
        Global::interact("token", "DIFFERENT", "splitter", &["step"], Global::End),
    );
    assert_ne!(
        derive_instance_id(&score),
        derive_instance_id(&other),
        "different structure → different id"
    );
}

// ===== a role that never waits needs no escape (no over-rejection) ===========

/// The escape discipline must NOT over-reject: a role that only sends / decides
/// (no awaits) carries no escape and is still fine.
#[test]
fn non_waiting_role_needs_no_escape() {
    // A linear A→B→C chain: A only ever SENDS (to B); it never awaits, so it is
    // not a waiter and carries no escape. C only ever RECEIVES (a pure waiter).
    let score = Score::new(
        &["A", "B", "C"],
        vec![Resource::new("step", Sort::Continuation)],
        Global::interact(
            "A",
            "go",
            "B",
            &["step"],
            Global::interact("B", "fwd", "C", &["step"], Global::End),
        ),
    );
    let locals = score.check().expect("must check");
    let roles = realize(&score, &locals);

    let a = roles.iter().find(|r| r.skeleton.role == "A").unwrap();
    assert!(!a.is_waiter(), "A only sends; it does not wait");
    assert!(a.escapes.is_empty(), "a non-waiter carries no escape");
    // The discipline still accepts (C, the waiter, has its escape).
    has_escape_for_every_waiter(&roles).expect("non-waiter must not be flagged");
}

// ===== rendering: id + escapes + honest liveness footer ======================

/// The realization render carries the binding id, the escape branches, and the
/// honest liveness footer that distinguishes not-stranded-beyond-T from completion.
#[test]
fn render_realization_shows_binding_escapes_and_honest_footer() {
    let app = digital_reit_app();
    let score = lift(&app).expect("must lift");
    let locals = score.check().expect("must check");
    let roles = realize(&score, &locals);

    let rendered = render_realization(&score, &roles);
    // Binding id present and labelled KIP-20.
    assert!(rendered.contains("instance binding id"));
    assert!(rendered.contains("KIP-20"));
    assert!(rendered.contains("OpInputCovenantId(0)"));
    // Escape branch present and labelled relative-timelock.
    assert!(rendered.contains("after this.age >= T"));
    assert!(rendered.contains("escape"));
    // Honest liveness footer: property stated, completion NOT claimed.
    assert!(rendered.contains("NOT-STRANDED-BEYOND-T"));
    assert!(rendered.contains("happy-path completion guaranteed: false"));
    assert!(rendered.contains("NOT on-chain"));
}

// ===== END-TO-END opt-in harness: parse → lift → check → realize → render ====

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

/// The full M3 pipeline on a PARSED program: parse → lift → check → realize →
/// binding-check → escape-check → render. The test-only stand-in for a
/// `portrait compose --realize <file>` subcommand (kept here to avoid touching
/// portrait-cli). Run with:
/// `cargo test -p portrait-compose -- --nocapture compose_realize_report`.
#[test]
fn compose_realize_report() {
    let program = portrait_syntax::parse(REIT_SRC).expect("REIT_SRC must parse");
    let score = lift(&program.app).expect("parsed DigitalReit must lift");
    let locals = score.check().expect("must check");
    let roles = realize(&score, &locals);

    let binding = realize_binding(&score, &roles);
    let escape = has_escape_for_every_waiter(&roles);

    println!(
        "\n=== portrait compose --realize (M3) — {} (parsed) ===",
        program.app.name
    );
    match &binding {
        Ok(id) => println!("BINDING: ACCEPT — all roles bound to {id}"),
        Err(e) => println!("BINDING: REJECT — {e}"),
    }
    match &escape {
        Ok(()) => println!("ESCAPE:  ACCEPT — every waiter has a timelock escape"),
        Err(e) => println!("ESCAPE:  REJECT — {e}"),
    }
    println!("\n{}", render_realization(&score, &roles));

    binding.expect("parsed DigitalReit binding must accept");
    escape.expect("parsed DigitalReit escapes must accept");
}

// ===== a manually-built waiter with NO escape is the worst case ==============

/// Directly construct a realized role that is genuinely AT RISK (it escrowed a
/// resource at a wait) but has an EMPTY escape list — the exact shape an emission
/// bug would produce — and confirm both the escape check and the liveness report
/// catch it. (Belt-and-braces; this never goes through `realize()` at all.)
///
/// HONEST MODEL (red-team fix): being a waiter is not enough to be strandable —
/// the role must have its OWN escrowed resources at risk. Here B has escrowed
/// `step` (recorded in `at_risk_waits`) and carries no escape, so it is
/// strandable. (A role that only receives, escrowing nothing, is NOT strandable;
/// see `waiter_that_escrowed_nothing_is_not_strandable`.)
#[test]
fn hand_built_waiter_without_escape_is_strandable() {
    let score = Score::new(
        &["A", "B"],
        vec![Resource::new("step", Sort::Continuation)],
        Global::interact("A", "go", "B", &["step"], Global::End),
    );
    let id = derive_instance_id(&score);
    let waiter = RealizedRole {
        skeleton: RoleSkeleton {
            role: "B".to_string(),
            entrypoints: vec![SkelEntry {
                message: "pre".to_string(),
                peer: "A".to_string(),
                resources: vec!["step".to_string()],
            }],
            awaits: vec![SkelEntry {
                message: "go".to_string(),
                peer: "A".to_string(),
                resources: vec![],
            }],
        },
        instance: id,
        // B escrowed `step` before awaiting A — genuinely at risk.
        at_risk_waits: vec![AtRiskWait {
            waiting_on: "A".to_string(),
            escrowed: vec!["step".to_string()],
        }],
        escapes: vec![], // BUG: an at-risk waiter with no escape.
    };
    let roles = vec![waiter];

    match has_escape_for_every_waiter(&roles) {
        Err(RealizeError::MissingEscape { role, waiting_on }) => {
            assert_eq!(role, "B");
            assert_eq!(waiting_on, "A");
        }
        other => panic!("expected MissingEscape(B); got {other:?}"),
    }
    let report = liveness_report(&roles);
    assert!(!report.not_stranded_beyond_t);
    assert_eq!(report.strandable, vec!["B".to_string()]);
}

// ===== HARDEN: the escape check must be NON-VACUOUS =========================
//
// The red-team showed the original check matched escapes by PEER ONLY and never
// inspected `reclaim`, so (a) an empty-reclaim escape, and (b) an escape that
// reclaimed the AWAITED (incoming) resource rather than the waiter's OWN
// escrowed resource, both passed while recovering nothing. These tests pin the
// corrected property: an escape only counts if it reclaims the waiter's own
// escrowed (previously-sent) resources.

/// REGRESSION GUARD (red-team finding 2): `realize()` must emit, for a waiter
/// that escrowed resources before its wait, an escape that reclaims THOSE OWN
/// escrowed resources — never the incoming/awaited resource held by the silent
/// counterparty. On the §5 escrow, Buyer escrows {payment, step} via `fund`
/// then awaits `deliver{asset}` from Seller; the escape must reclaim Buyer's own
/// {payment, step}, NOT the `asset` (which Seller holds and Buyer never got).
#[test]
fn realize_reclaims_own_escrowed_resource_not_the_incoming_one() {
    let score = asset_escrow_example();
    let locals = score.check().expect("escrow must check");
    let roles = realize(&score, &locals);

    let buyer = roles
        .iter()
        .find(|r| r.skeleton.role == "Buyer")
        .expect("Buyer present");

    // Buyer awaits `deliver{asset}` from Seller and is a waiter.
    assert!(buyer.is_waiter(), "Buyer waits on Seller's deliver");

    // Every escape Buyer carries must reclaim ONLY Buyer's own escrowed
    // resources (payment / step) and must be NON-EMPTY — never `asset`.
    let own_sent: std::collections::BTreeSet<String> = buyer
        .skeleton
        .entrypoints
        .iter()
        .flat_map(|e| e.resources.iter().cloned())
        .collect();
    assert!(
        !buyer.escapes.is_empty(),
        "Buyer (a waiter at risk) must have an escape"
    );
    for esc in &buyer.escapes {
        assert!(
            !esc.reclaim.is_empty(),
            "an escape that reclaims nothing is vacuous: {esc:?}"
        );
        for r in &esc.reclaim {
            assert!(
                own_sent.contains(r),
                "escape reclaims `{r}` which is not one of Buyer's own escrowed \
                 resources {own_sent:?} — backwards reclaim of incoming resource"
            );
            assert_ne!(r, "asset", "must not reclaim the incoming `asset`");
        }
    }

    // The corrected realize() output is accepted by both checks.
    realize_binding(&score, &roles).expect("binding accepts");
    has_escape_for_every_waiter(&roles).expect("escapes accept");
    let report = liveness_report(&roles);
    assert!(
        report.not_stranded_beyond_t,
        "property holds with real reclaims"
    );
    assert!(!report.happy_path_completion_guaranteed());
}

/// NON-VACUITY (red-team findings 1 & 3): a waiter that escrowed a resource but
/// whose escape has an EMPTY reclaim recovers nothing, so the check must REJECT
/// it (naming the strandable role) — not silently accept on a peer match.
#[test]
fn empty_reclaim_escape_for_an_at_risk_waiter_is_rejected() {
    // A escrows `payment` to B, then awaits `ack` from B. A is at risk if B goes
    // silent after receiving the payment.
    let score = Score::new(
        &["A", "B"],
        vec![
            Resource::new("payment", Sort::Value),
            Resource::new("ack", Sort::Capability),
        ],
        Global::interact(
            "A",
            "pay",
            "B",
            &["payment"],
            Global::interact("B", "ack", "A", &["ack"], Global::End),
        ),
    );
    let locals = score.check().expect("must check");
    let mut roles = realize(&score, &locals);

    let a = roles
        .iter_mut()
        .find(|r| r.skeleton.role == "A")
        .expect("A present");
    assert!(a.is_waiter(), "A awaits B's ack");
    // Corrupt A's escape to reclaim NOTHING (the vacuous-escape emission bug).
    for esc in &mut a.escapes {
        esc.reclaim.clear();
    }

    match has_escape_for_every_waiter(&roles) {
        Err(RealizeError::MissingEscape { role, waiting_on }) => {
            assert_eq!(role, "A", "the at-risk role with an empty-reclaim escape");
            assert_eq!(waiting_on, "B");
        }
        other => panic!("expected MissingEscape(A) for an empty-reclaim escape; got {other:?}"),
    }
    let report = liveness_report(&roles);
    assert!(
        !report.not_stranded_beyond_t,
        "empty reclaim => not protected"
    );
    assert!(report.strandable.contains(&"A".to_string()));
}

/// NON-VACUITY (red-team finding 2): an escape that reclaims the AWAITED
/// (incoming) resource rather than the waiter's own escrowed resource recovers
/// nothing the waiter actually controls, so the check must REJECT it.
#[test]
fn escape_reclaiming_only_the_incoming_resource_is_rejected() {
    let score = Score::new(
        &["A", "B"],
        vec![
            Resource::new("payment", Sort::Value),
            Resource::new("goods", Sort::Capability),
        ],
        Global::interact(
            "A",
            "pay",
            "B",
            &["payment"],
            Global::interact("B", "ship", "A", &["goods"], Global::End),
        ),
    );
    let locals = score.check().expect("must check");
    let mut roles = realize(&score, &locals);

    let a = roles
        .iter_mut()
        .find(|r| r.skeleton.role == "A")
        .expect("A present");
    // Corrupt A's escape to reclaim the INCOMING `goods` (held by silent B),
    // not A's own escrowed `payment`.
    for esc in &mut a.escapes {
        esc.reclaim = vec!["goods".to_string()];
    }

    match has_escape_for_every_waiter(&roles) {
        Err(RealizeError::MissingEscape { role, waiting_on }) => {
            assert_eq!(role, "A");
            assert_eq!(waiting_on, "B");
        }
        other => panic!("expected MissingEscape(A) for an incoming-only reclaim; got {other:?}"),
    }
    assert!(!liveness_report(&roles).not_stranded_beyond_t);
}

/// NO OVER-REJECTION (the dual): a waiter that has escrowed NOTHING before its
/// wait has nothing at risk, so it needs no reclaim and must NOT be flagged. In
/// the §5 escrow, Seller's first action is awaiting `fund` from Buyer — Seller
/// has escrowed nothing at that point, so it is not strandable on that wait.
#[test]
fn waiter_that_escrowed_nothing_is_not_strandable() {
    let score = asset_escrow_example();
    let locals = score.check().expect("must check");
    let roles = realize(&score, &locals);

    // The whole realized escrow is accepted (no over-rejection on the
    // escrowed-nothing waits) and the property holds.
    has_escape_for_every_waiter(&roles).expect("no over-rejection");
    assert!(liveness_report(&roles).not_stranded_beyond_t);
}
