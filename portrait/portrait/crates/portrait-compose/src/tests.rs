//! Non-vacuity proof for the Composer M1 checker.
//!
//! The checker is only meaningful if it ACCEPTS the well-formed 3-party escrow
//! AND REJECTS several malformed protocols, each with the *right named error*.
//! These tests pin both directions.

use super::*;

// ===== ACCEPT: the well-formed 3-party escrow ===============================

#[test]
fn escrow_accepts_and_projects_three_roles() {
    let score = asset_escrow_example();
    let locals = score
        .check()
        .expect("the well-formed escrow must pass all three checks");

    // Exactly the three declared roles project.
    let roles: Vec<&String> = locals.keys().collect();
    assert_eq!(roles, vec!["Arbiter", "Buyer", "Seller"]);

    // Buyer's local type starts with a send of `fund` to Seller.
    match locals.get("Buyer").unwrap() {
        Local::Send {
            to,
            message,
            resources,
            ..
        } => {
            assert_eq!(to, "Seller");
            assert_eq!(message, "fund");
            assert!(resources.contains(&"payment".to_string()));
            assert!(resources.contains(&"step".to_string()));
        }
        other => panic!("Buyer should lead with a send; got {other:?}"),
    }

    // Seller's local type starts with a receive of `fund` from Buyer.
    match locals.get("Seller").unwrap() {
        Local::Recv { from, message, .. } => {
            assert_eq!(from, "Buyer");
            assert_eq!(message, "fund");
        }
        other => panic!("Seller should lead with a recv; got {other:?}"),
    }

    // Arbiter's local type is an internal choice (it decides).
    match locals.get("Arbiter").unwrap() {
        Local::Select { to, branches } => {
            assert_eq!(to, "Buyer");
            assert_eq!(branches.len(), 2);
            let labels: Vec<&String> = branches.iter().map(|(l, _)| l).collect();
            assert!(labels.contains(&&"release".to_string()));
            assert!(labels.contains(&&"refund".to_string()));
        }
        other => panic!("Arbiter should be an internal choice; got {other:?}"),
    }
}

#[test]
fn escrow_seller_is_notified_of_verdict_via_distinguishing_receive() {
    // §5.3: the dangerous case is Seller waiting forever on `settle`. Seller must
    // be notified of release vs refund. In our model Buyer relays the verdict to
    // Seller via distinct `verdict_release` / `verdict_refund` messages, so the
    // merge resolves to an external choice (Offer) — NOT a rejection.
    let score = asset_escrow_example();
    let locals = score.check().unwrap();
    match locals.get("Seller").unwrap() {
        // fund recv, deliver send, then the verdict offer.
        Local::Recv { cont, .. } => match &**cont {
            Local::Send { cont, .. } => match &**cont {
                Local::Offer { from, branches } => {
                    assert_eq!(from, "Buyer");
                    assert_eq!(branches.len(), 2, "Seller must distinguish both verdicts");
                }
                other => panic!("expected Offer after deliver; got {other:?}"),
            },
            other => panic!("expected deliver send; got {other:?}"),
        },
        other => panic!("expected fund recv; got {other:?}"),
    }
}

// ===== REJECT: malformed protocols, each with the right named error ==========

/// Non-projectable: a non-deciding role has divergent behaviour across branches
/// with NO distinguishing notification — the classic orphan-wait.
#[test]
fn rejects_non_projectable_unnotified_role() {
    use Sort::*;
    // Arbiter decides, informs Buyer. Seller is NOT informed, yet must SEND on
    // one branch and do nothing on the other → undefined merge for Seller.
    let global = Global::branching(
        "Arbiter",
        "Buyer",
        vec![
            (
                "release",
                Global::interact("Seller", "settle", "Buyer", &["payment"], Global::End),
            ),
            ("refund", Global::End),
        ],
    );
    let score = Score::new(
        &["Arbiter", "Buyer", "Seller"],
        vec![Resource::new("payment", Value)],
        global,
    );
    match score.check() {
        Err(ComposeError::NotProjectable { role, .. }) => assert_eq!(role, "Seller"),
        other => panic!("expected NotProjectable for Seller; got {other:?}"),
    }
}

/// Orphan message: a send with no matching receive.
#[test]
fn rejects_orphan_unmatched_message() {
    // Build local types directly so we have a send with no receiver. We do this
    // through a global where the receiver never projects the message: impossible
    // by construction from a single global, so we test the duality check on a
    // hand-built local-type map (the check operates on the projected set).
    let mut locals: BTreeMap<Role, Local> = BTreeMap::new();
    locals.insert(
        "A".to_string(),
        Local::Send {
            to: "B".to_string(),
            message: "ping".to_string(),
            resources: vec![],
            cont: Box::new(Local::End),
        },
    );
    // B never receives `ping`.
    locals.insert("B".to_string(), Local::End);

    match check_duality(&locals) {
        Err(ComposeError::OrphanMessage { message, unmatched }) => {
            assert_eq!(message, "ping");
            assert_eq!(unmatched, "send");
        }
        other => panic!("expected OrphanMessage(send); got {other:?}"),
    }
}

/// Orphan message: a receive with no matching send.
#[test]
fn rejects_orphan_unmatched_receive() {
    let mut locals: BTreeMap<Role, Local> = BTreeMap::new();
    locals.insert(
        "B".to_string(),
        Local::Recv {
            from: "A".to_string(),
            message: "pong".to_string(),
            resources: vec![],
            cont: Box::new(Local::End),
        },
    );
    locals.insert("A".to_string(), Local::End);

    match check_duality(&locals) {
        Err(ComposeError::OrphanMessage { message, unmatched }) => {
            assert_eq!(message, "pong");
            assert_eq!(unmatched, "receive");
        }
        other => panic!("expected OrphanMessage(receive); got {other:?}"),
    }
}

/// Linearity: a double-spend of a linear resource — a role that is NOT the
/// current holder carries the live resource, breaking the handoff chain.
#[test]
fn rejects_linearity_double_consume() {
    use Sort::*;
    // `payment` produced at m1 (A→B, holder := B). At m2, C (who never held it)
    // tries to carry it to D → two parties claim the same linear token.
    let global = Global::interact(
        "A",
        "m1",
        "B",
        &["payment"],
        Global::interact("C", "m2", "D", &["payment"], Global::End),
    );
    let score = Score::new(
        &["A", "B", "C", "D"],
        vec![Resource::new("payment", Value)],
        global,
    );
    match score.check() {
        Err(ComposeError::ResourceConsumedTwice { resource }) => {
            assert_eq!(resource, "payment")
        }
        other => panic!("expected ResourceConsumedTwice(payment); got {other:?}"),
    }
}

/// Linearity (FIX 2): a `Value` delivered to two DISTINCT receivers on one path
/// is a double-spend, even though the same SENDER authorises both carries. The
/// holder for a Value stays under the sender, so the second carry looks like a
/// legal re-carry — but the coin is delivered to two different parties.
#[test]
fn rejects_value_delivered_to_two_distinct_receivers() {
    use Sort::*;
    // A→B{coin} then A→C{coin}: same sender A, but coin goes to B AND to C.
    let global = Global::interact(
        "A",
        "p1",
        "B",
        &["coin"],
        Global::interact("A", "p2", "C", &["coin"], Global::End),
    );
    let score = Score::new(&["A", "B", "C"], vec![Resource::new("coin", Value)], global);
    match score.check() {
        Err(ComposeError::ResourceConsumedTwice { resource }) => assert_eq!(resource, "coin"),
        other => panic!("expected ResourceConsumedTwice(coin) for a double-receive; got {other:?}"),
    }
}

/// Linearity (FIX 2): same double-receive hole for a `Capability` (double-grant).
#[test]
fn rejects_capability_granted_to_two_distinct_receivers() {
    use Sort::*;
    let global = Global::interact(
        "A",
        "g1",
        "B",
        &["cap"],
        Global::interact("A", "g2", "C", &["cap"], Global::End),
    );
    let score = Score::new(
        &["A", "B", "C"],
        vec![Resource::new("cap", Capability)],
        global,
    );
    match score.check() {
        Err(ComposeError::ResourceConsumedTwice { resource }) => assert_eq!(resource, "cap"),
        other => panic!("expected ResourceConsumedTwice(cap) for a double-grant; got {other:?}"),
    }
}

/// Linearity (FIX 2, positive): re-carrying a `Value` to the SAME receiver is a
/// legal re-carry, not a double-spend (the escrow `fund … settle` pattern).
#[test]
fn accepts_value_recarried_to_same_receiver() {
    use Sort::*;
    // A→B{coin} then A→B{coin}: same sender, same receiver — legal re-carry.
    let global = Global::interact(
        "A",
        "p1",
        "B",
        &["coin"],
        Global::interact("A", "p2", "B", &["coin"], Global::End),
    );
    let score = Score::new(&["A", "B"], vec![Resource::new("coin", Value)], global);
    score
        .check()
        .expect("re-carrying a Value to the SAME receiver must be accepted");
}

/// Linearity (FIX 3): a `Value` left live at a NON-terminating recursion cut is
/// stranded — produced, then abandoned when the loop folds back without reaching
/// `end`. `μX. A→B{coin} . B→A . X` strands `coin`.
#[test]
fn rejects_value_stranded_at_recursion_cut() {
    use Sort::*;
    let body = Global::interact(
        "A",
        "mk",
        "B",
        &["coin"],
        Global::interact("B", "ack", "A", &[], Global::var("X")),
    );
    let global = Global::rec("X", body);
    let score = Score::new(&["A", "B"], vec![Resource::new("coin", Value)], global);
    match score.check() {
        Err(ComposeError::ResourceStranded { resource }) => assert_eq!(resource, "coin"),
        other => panic!("expected ResourceStranded(coin) at the recursion cut; got {other:?}"),
    }
}

/// Linearity (positive): a legal `Continuation` handoff chain — A hands the step
/// to B, then B (now the holder) hands it to C — is accepted.
#[test]
fn accepts_legal_handoff_chain() {
    use Sort::*;
    // step is a Continuation: holder transfers to the receiver each carry.
    // A→B (holder B) then B→C (B is the holder, legal handoff).
    let global = Global::interact(
        "A",
        "m1",
        "B",
        &["step"],
        Global::interact("B", "m2", "C", &["step"], Global::End),
    );
    let score = Score::new(
        &["A", "B", "C"],
        vec![Resource::new("step", Continuation)],
        global,
    );
    score
        .check()
        .expect("a connected Continuation handoff chain must be accepted");
}

/// Linearity: a resource declared in the ledger but never carried by any
/// interaction is produced zero times — it can never be consumed (stranded).
#[test]
fn rejects_stranded_unused_resource() {
    use Sort::*;
    let global = Global::interact("A", "m", "B", &["used"], Global::End);
    let score = Score::new(
        &["A", "B"],
        vec![
            Resource::new("used", Value),
            Resource::new("ghost_asset", Value), // declared, never carried
        ],
        global,
    );
    match score.check() {
        Err(ComposeError::ResourceStranded { resource }) => assert_eq!(resource, "ghost_asset"),
        other => panic!("expected ResourceStranded(ghost_asset); got {other:?}"),
    }
}

/// Self-interaction: `p → p` is forbidden (p ≠ q, §2.2).
#[test]
fn rejects_self_interaction() {
    let global = Global::interact("A", "loop", "A", &[], Global::End);
    let score = Score::new(&["A"], vec![], global);
    match score.check() {
        Err(ComposeError::SelfInteraction { role, .. }) => assert_eq!(role, "A"),
        other => panic!("expected SelfInteraction; got {other:?}"),
    }
}

/// Unguarded recursion: `μX. X` does not project to a finite covenant lifecycle.
#[test]
fn rejects_unguarded_recursion() {
    let global = Global::rec("X", Global::var("X"));
    let score = Score::new(&["A"], vec![], global);
    match score.check() {
        Err(ComposeError::UnguardedRecursion { var }) => assert_eq!(var, "X"),
        other => panic!("expected UnguardedRecursion; got {other:?}"),
    }
}

/// Unbound recursion variable.
#[test]
fn rejects_unbound_variable() {
    let global = Global::interact("A", "m", "B", &[], Global::var("Y"));
    let score = Score::new(&["A", "B"], vec![], global);
    match score.check() {
        Err(ComposeError::UnboundVariable { var }) => assert_eq!(var, "Y"),
        other => panic!("expected UnboundVariable; got {other:?}"),
    }
}

/// Parallel composition that shares a role is rejected (disjointness, §2.2).
#[test]
fn rejects_par_role_overlap() {
    let left = Global::interact("A", "x", "B", &[], Global::End);
    let right = Global::interact("A", "y", "C", &[], Global::End); // A appears in both
    let score = Score::new(&["A", "B", "C"], vec![], Global::par(left, right));
    match score.check() {
        Err(ComposeError::ParRoleOverlap { role }) => assert_eq!(role, "A"),
        other => panic!("expected ParRoleOverlap; got {other:?}"),
    }
}

/// Parallel composition that shares a resource is rejected (§2.2).
#[test]
fn rejects_par_resource_overlap() {
    use Sort::*;
    let left = Global::interact("A", "x", "B", &["coin"], Global::End);
    let right = Global::interact("C", "y", "D", &["coin"], Global::End); // coin in both
    let score = Score::new(
        &["A", "B", "C", "D"],
        vec![Resource::new("coin", Value)],
        Global::par(left, right),
    );
    match score.check() {
        Err(ComposeError::ParResourceOverlap { resource }) => assert_eq!(resource, "coin"),
        other => panic!("expected ParResourceOverlap; got {other:?}"),
    }
}

/// Undeclared resource carried by an interaction.
#[test]
fn rejects_undeclared_resource() {
    let global = Global::interact("A", "m", "B", &["ghost"], Global::End);
    let score = Score::new(&["A", "B"], vec![], global);
    match score.check() {
        Err(ComposeError::UndeclaredResource { resource }) => assert_eq!(resource, "ghost"),
        other => panic!("expected UndeclaredResource; got {other:?}"),
    }
}

/// Duplicate branch labels are rejected.
#[test]
fn rejects_duplicate_branch_labels() {
    let global = Global::branching("A", "B", vec![("same", Global::End), ("same", Global::End)]);
    let score = Score::new(&["A", "B"], vec![], global);
    match score.check() {
        Err(ComposeError::BadBranchLabels { decider }) => assert_eq!(decider, "A"),
        other => panic!("expected BadBranchLabels; got {other:?}"),
    }
}

// ===== sanity: a minimal well-formed 2-party protocol accepts ===============

#[test]
fn minimal_two_party_accepts() {
    use Sort::*;
    let global = Global::interact(
        "A",
        "pay",
        "B",
        &["coin"],
        Global::interact("B", "ack", "A", &[], Global::End),
    );
    let score = Score::new(&["A", "B"], vec![Resource::new("coin", Value)], global);
    let locals = score.check().expect("minimal protocol should accept");
    assert_eq!(locals.len(), 2);
}

// ===== a well-formed recursive (bounded) protocol accepts ===================

#[test]
fn bounded_recursion_accepts() {
    // μX. A→B[tick] . X  — guarded; projects to finite local types.
    let global = Global::rec(
        "X",
        Global::interact("A", "tick", "B", &[], Global::var("X")),
    );
    let score = Score::new(&["A", "B"], vec![], global);
    let locals = score.check().expect("guarded recursion should accept");
    match locals.get("A").unwrap() {
        Local::Rec(x, _) => assert_eq!(x, "X"),
        other => panic!("expected Rec for A; got {other:?}"),
    }
}

// ===== render + footer are present (harness aid) ============================

#[test]
fn render_and_footer_are_nonempty() {
    let score = asset_escrow_example();
    let locals = score.check().unwrap();
    let s = render_local(locals.get("Buyer").unwrap());
    assert!(s.contains("fund"));
    assert!(HONEST_BOUNDARY_FOOTER.contains("SAFETY"));
    assert!(HONEST_BOUNDARY_FOOTER.to_lowercase().contains("liveness"));
}

/// Opt-in harness: `cargo test -p portrait-compose -- --nocapture compose_report`
/// prints the projection of the 3-party escrow, the ACCEPT verdict, and the
/// honest-boundary footer. This is the test-only stand-in for a future
/// `portrait compose` CLI subcommand (kept here to avoid touching portrait-cli).
#[test]
fn compose_report() {
    let score = asset_escrow_example();
    let verdict = score.check();
    println!("\n=== portrait compose — AssetEscrow (3-party) ===");
    match &verdict {
        Ok(locals) => {
            for (role, t) in locals {
                println!("  {role:<8} ⊢ {}", render_local(t));
            }
            println!("\nVERDICT: ACCEPT — projectable, dual (no orphans), linear.");
        }
        Err(e) => println!("VERDICT: REJECT — {e}"),
    }
    println!("\n{HONEST_BOUNDARY_FOOTER}");
    assert!(verdict.is_ok());
}

/// Opt-in M5 harness:
/// `cargo test -p portrait-compose -- --nocapture m5_report`.
///
/// Emits the per-role REAL covenants (and proves, on the real parser + sema, that
/// every one parses AND sema-checks), then runs the local executor on the escrow
/// along an authored `release` flow, printing the trace and the honest footers.
///
/// HONESTY: emission is real (the assertions below call `portrait_syntax::parse`
/// and `portrait_sema::check` — not a self-claim); the executor is an in-memory
/// SIMULATION of the Score model, NOT a chain runtime, and a `Completed` run does
/// NOT imply on-chain liveness.
#[test]
fn m5_report() {
    use crate::emit_real::{emit_real_covenants, render_real_covenants};
    use crate::execute::{execute_with_choices, render_trace};

    let score = asset_escrow_example();
    let locals = score.check().expect("escrow checks");

    // (1) REAL emission — prove every emitted covenant round-trips.
    let covenants = emit_real_covenants(&locals);
    for c in &covenants {
        let parsed = portrait_syntax::parse(&c.source)
            .unwrap_or_else(|e| panic!("role `{}` covenant failed to parse: {e}", c.role));
        portrait_sema::check(&parsed).unwrap_or_else(|d| {
            let msgs: Vec<String> = d.into_iter().map(|x| x.message).collect();
            panic!(
                "role `{}` covenant failed sema: {}",
                c.role,
                msgs.join("; ")
            );
        });
    }
    println!("\n{}", render_real_covenants(&covenants));
    println!(
        "[verified] all {} role covenants PARSE and SEMA-CHECK",
        covenants.len()
    );

    // (2)+(3) run the executor on the authored `release` flow and print the trace.
    let trace = execute_with_choices(&score, &["release"]);
    println!("\n{}", render_trace(&trace));

    assert_eq!(trace.status, crate::execute::Status::Completed);
}
