//! The three M3 VC classes beyond value conservation, each grounded in the
//! **real** Portrait surface AST and each routed through the *same* SAT(T)
//! vacuity-safe [`crate::discharge`] (only a satisfiable `T` plus an unsat
//! negated-VC yields `Proved`):
//!
//! - **Range / overflow** (§4(c)) — every value-bearing-field arithmetic result
//!   fits the on-chain `u64` sompi width `[0, 2^64)`. Turns the prior honest-scope
//!   bounded-int gap (`a*2` PROVED over ℤ but wraps on-chain) into a CHECKED
//!   obligation. Always generatable wherever a value field's return is arithmetic.
//! - **Refinement** (§4(b)) — a **declared** named refinement invariant
//!   (`non_negative_amount`, `spending_cap`) as a real implication `G ⟹ φ`.
//!   Generated ONLY when the app declares the invariant (the surface *can* state
//!   it, via the `invariant <name>;` tag with sema-fixed semantics).
//! - **Invariant preservation** (§4(d)) — a **declared** stateful invariant
//!   (`bounded_supply`: `supply <= total`; `monotonic_seq`: `seq' = seq + 1`)
//!   preserved across the transition, `I(s) ∧ G ⟹ I(s')`.
//!
//! ## What is NOT expressible (honest scope)
//!
//! The M0 spec §4(d) imagines a *user-written arbitrary arithmetic invariant*
//! `I` (e.g. a bespoke ceiling `balance <= cap`). The real surface AST has **no
//! syntax** for that: [`portrait_syntax::Invariant`] is `ValueConserved`,
//! `NoUndeclaredState`, or `Custom(String)` — a *named tag* whose arithmetic
//! meaning is fixed in `portrait-sema`, never an `Expr` the author supplies. So
//! the only groundable preservation/refinement obligations are the named
//! invariants with sema-defined arithmetic semantics. A VC for a predicate the
//! language cannot state is not generatable, and is NOT faked here. The
//! `temporal_guard` named invariant is likewise not generated: its meaning
//! (`now_bucket >= committed deadline`) is a structural capability/time gate, not
//! a closed arithmetic obligation over the conserved fields.

use crate::encode::is_value_bearing_split;
use crate::vc::{build_transition, primed, U64_EXCL_MAX};
use crate::{LensError, VcKind, VcReport};
use portrait_syntax::{Entry, Expr, Invariant, Role, Stmt};

/// The set of declared named invariants (the app-level `invariant <name>;` tags),
/// reduced to the `Custom`/builtin names relevant to the refinement +
/// preservation VC classes. `ValueConserved`/`NoUndeclaredState` are handled by
/// sema + the conservation VC and are not in this set's named form.
pub(crate) struct DeclaredInvariants {
    names: std::collections::BTreeSet<String>,
}

impl DeclaredInvariants {
    /// Collect the named (`Custom`) invariants declared at the app level.
    pub(crate) fn from_app(invariants: &[Invariant]) -> Self {
        let names = invariants
            .iter()
            .filter_map(|i| match i {
                Invariant::Custom(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        Self { names }
    }

    fn declares(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

/// Find the object-return next-state fields of a transition entrypoint, or `None`
/// if it has no object return (scalar / verification-only). Mirrors the
/// conservation VC's `return_fields` extraction so all classes see the same
/// next-state model.
fn object_return_fields(entry: &Entry) -> Option<&[(String, Expr)]> {
    entry.body.iter().find_map(|s| match s {
        Stmt::Return(portrait_syntax::ReturnExpr::Object { fields, .. }) => Some(fields.as_slice()),
        _ => None,
    })
}

/// `true` if a return value for `field` is a bare carry `f: f`. A carried field
/// performs no arithmetic, so it has no range obligation.
fn is_carry(field: &str, value: &Expr) -> bool {
    matches!(value, Expr::Var(name) if name == field)
}

// ── (c) Range / overflow ─────────────────────────────────────────────────────

/// Build the range/overflow VC for one transition entrypoint (spec §4(c)).
///
/// For the **post-state value of every value-bearing field whose return performs
/// arithmetic** (i.e. is not a bare carry), assert the transition relation `T`
/// and ask whether ANY such result can fall outside the on-chain `u64` sompi
/// window `[0, 2^64)`:
///
/// ```text
///   VC ≡ G ∧ s'=⟦return⟧ ⟹ ⋀_{f∈V, moved} 0 <= f' < 2^64
///   ¬VC: (or (< f' 0) (>= f' 2^64) ...)
/// ```
///
/// `unsat` ⇒ **PROVED** (no value-field result overflows/underflows `u64`).
/// `sat`   ⇒ **REFUTED** (a guard-satisfying transition overflows) + counter-model.
/// Routed through the SAT(T) vacuity guard via [`crate::discharge`].
///
/// Refused (a hard [`LensError`], skipped by `prove_program`, never a verdict)
/// when there is nothing to range-check: no object return, no value-bearing
/// field, or every value-bearing field is a bare carry.
pub(crate) fn build_range_vc(role: &Role, entry: &Entry) -> Result<VcReport, LensError> {
    if !matches!(entry.mode, portrait_syntax::CovenantMode::Transition) {
        return Err(LensError::Unsupported(
            "not a transition entrypoint".to_string(),
        ));
    }
    if entry.body.iter().any(|s| matches!(s, Stmt::Raw(_))) {
        return Err(LensError::RawStmtInCovenant);
    }
    let return_fields = object_return_fields(entry).ok_or_else(|| {
        LensError::Unsupported("no object-return next-state to range-check".to_string())
    })?;

    // The value-bearing fields whose return performs arithmetic (not a bare
    // carry). A carried field cannot overflow — it is the same committed value.
    let mut ranged: Vec<String> = Vec::new();
    for (field, value) in return_fields {
        let f = role.state.iter().find(|s| &s.name == field);
        let Some(f) = f else { continue };
        if !is_value_bearing_split(&f.name, &f.ty) {
            continue;
        }
        if is_carry(field, value) {
            continue;
        }
        ranged.push(primed(field));
    }
    if ranged.is_empty() {
        return Err(LensError::Unsupported(
            "no value-bearing field performs arithmetic; nothing to range-check".to_string(),
        ));
    }

    let t = build_transition(role, entry, return_fields)?;
    let transition_only = t.transition_only();
    let mut b = t.builder;

    // ¬VC: some value-field result is below 0 OR at/above 2^64 (the u64 window).
    let mut disjuncts: Vec<String> = Vec::new();
    for f in &ranged {
        disjuncts.push(format!("(< {f} 0)"));
        disjuncts.push(format!("(>= {f} {U64_EXCL_MAX})"));
    }
    let neg = if disjuncts.len() == 1 {
        disjuncts.into_iter().next().unwrap()
    } else {
        format!("(or {})", disjuncts.join(" "))
    };
    b.assert(&neg);

    Ok(VcReport {
        entrypoint: format!("{}.{}", role.name, entry.name),
        vc_kind: VcKind::Range,
        smtlib: b.finish(t.logic.name()),
        transition_smtlib: transition_only,
        outcome: None,
    })
}

// ── (b) Refinement: G ⟹ φ for a declared named invariant ─────────────────────

/// Build the refinement VCs declared by the app (spec §4(b)). Each is a real
/// implication `G ⟹ φ`, negated as `G ∧ ¬φ` and routed through the SAT(T) guard.
///
/// Generated ONLY for a **declared** named invariant whose meaning is a closed
/// arithmetic predicate over this entrypoint's args/state:
/// - `non_negative_amount` (when the entry takes an int `amount`): `G ⟹ amount >= 0`.
/// - `spending_cap` (when the entry takes int `amount` and a `limit` state field
///   or role param is in scope): `G ⟹ amount <= limit`.
///
/// A declared invariant that does not apply to this entrypoint (no `amount` arg,
/// no `limit`) yields no VC for it — not a failure, just nothing to prove here.
pub(crate) fn build_refinement_vcs(
    role: &Role,
    entry: &Entry,
    declared: &DeclaredInvariants,
) -> Vec<VcReport> {
    let mut out = Vec::new();
    if !matches!(entry.mode, portrait_syntax::CovenantMode::Transition) {
        return out;
    }
    if entry.body.iter().any(|s| matches!(s, Stmt::Raw(_))) {
        return out;
    }
    let Some(return_fields) = object_return_fields(entry) else {
        return out;
    };

    let has_amount = entry
        .args
        .iter()
        .any(|a| a.name == "amount" && a.ty == portrait_syntax::Type::Int);

    // non_negative_amount: G ⟹ amount >= 0.
    if declared.declares("non_negative_amount") && has_amount {
        if let Some(r) = emit_refinement(role, entry, return_fields, "(>= amount 0)") {
            out.push(r);
        }
    }

    // spending_cap: G ⟹ amount <= limit (limit a committed state field or role param).
    let has_limit = role.state.iter().any(|f| f.name == "limit")
        || role.params.iter().any(|p| p.name == "limit");
    if declared.declares("spending_cap") && has_amount && has_limit {
        if let Some(r) = emit_refinement(role, entry, return_fields, "(<= amount limit)") {
            out.push(r);
        }
    }

    out
}

/// Emit one refinement VC: `T ∧ ¬φ`, with `φ` a ground SMT-LIB Bool term over
/// names already declared by `T`. Returns `None` if `T` cannot be encoded.
fn emit_refinement(
    role: &Role,
    entry: &Entry,
    return_fields: &[(String, Expr)],
    phi: &str,
) -> Option<VcReport> {
    let t = build_transition(role, entry, return_fields).ok()?;
    let transition_only = t.transition_only();
    let mut b = t.builder;
    // ¬(G ⟹ φ) under the asserted G is just ¬φ.
    b.assert(&format!("(not {phi})"));
    Some(VcReport {
        entrypoint: format!("{}.{}", role.name, entry.name),
        vc_kind: VcKind::Refinement,
        smtlib: b.finish(t.logic.name()),
        transition_smtlib: transition_only,
        outcome: None,
    })
}

// ── (d) Invariant preservation: I(s) ∧ G ⟹ I(s') ─────────────────────────────

/// Build the invariant-preservation VCs declared by the app (spec §4(d)). Each is
/// the classic inductive obligation `I(s) ∧ G(s,p) ⟹ I(s')`, negated as
/// `I(s) ∧ G ∧ ¬I(s')` and routed through the SAT(T) guard.
///
/// Generated ONLY for a **declared** named invariant whose meaning is a closed
/// *stateful* arithmetic predicate:
/// - `bounded_supply` (when `supply` and `total` are state fields):
///   `I ≡ supply <= total`; prove `supply <= total ∧ G ⟹ supply' <= total'`.
/// - `monotonic_seq` (when `seq` is a state field): the step relation
///   `seq' = seq + 1` (the invariant a `monotonic_seq` transition must maintain).
///
/// SOUNDNESS: the negated VC adds `I(pre)` as a hypothesis, NOT to the vacuity
/// probe `T`. The probe is `T` alone (no `I(pre)`), so an `I(pre)` that happens
/// to contradict the guards cannot vacuously discharge the obligation — the SAT(T)
/// guard still requires the *guards* to be satisfiable for a `Proved`.
pub(crate) fn build_preservation_vcs(
    role: &Role,
    entry: &Entry,
    declared: &DeclaredInvariants,
) -> Vec<VcReport> {
    let mut out = Vec::new();
    if !matches!(entry.mode, portrait_syntax::CovenantMode::Transition) {
        return out;
    }
    if entry.body.iter().any(|s| matches!(s, Stmt::Raw(_))) {
        return out;
    }
    let Some(return_fields) = object_return_fields(entry) else {
        return out;
    };

    let has_supply = role.state.iter().any(|f| f.name == "supply");
    let has_total = role.state.iter().any(|f| f.name == "total");
    let has_seq = role.state.iter().any(|f| f.name == "seq");

    // bounded_supply: I ≡ supply <= total. Inductive: I(pre) ∧ G ⟹ I(post).
    if declared.declares("bounded_supply") && has_supply && has_total {
        if let Some(r) = emit_preservation(
            role,
            entry,
            return_fields,
            "(<= supply total)",
            &format!("(<= {} {})", primed("supply"), primed("total")),
        ) {
            out.push(r);
        }
    }

    // monotonic_seq: the step the invariant maintains is seq' = seq + 1. There is
    // no `I(pre)` hypothesis (it is a step relation, not a state predicate), so
    // I(pre) is `true` and the obligation is G ⟹ seq' = seq + 1.
    if declared.declares("monotonic_seq") && has_seq {
        if let Some(r) = emit_preservation(
            role,
            entry,
            return_fields,
            "true",
            &format!("(= {} (+ seq 1))", primed("seq")),
        ) {
            out.push(r);
        }
    }

    out
}

/// Emit one preservation VC: assert `I(pre)` as a hypothesis on top of `T`, then
/// the negated post-condition `¬I(post)`. The vacuity probe stays `T` alone (no
/// `I(pre)`), so the SAT(T) guard is unaffected. Returns `None` if `T` cannot be
/// encoded.
fn emit_preservation(
    role: &Role,
    entry: &Entry,
    return_fields: &[(String, Expr)],
    i_pre: &str,
    not_i_post_inner: &str,
) -> Option<VcReport> {
    let t = build_transition(role, entry, return_fields).ok()?;
    // Vacuity probe: T ALONE, WITHOUT the I(pre) hypothesis. A contradictory
    // I(pre) must not be able to vacuously discharge the obligation.
    let transition_only = t.transition_only();
    let mut b = t.builder;
    if i_pre != "true" {
        b.assert(i_pre);
    }
    b.assert(&format!("(not {not_i_post_inner})"));
    Some(VcReport {
        entrypoint: format!("{}.{}", role.name, entry.name),
        vc_kind: VcKind::InvariantPreservation,
        smtlib: b.finish(t.logic.name()),
        transition_smtlib: transition_only,
        outcome: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{discharge, Outcome};
    use portrait_syntax::parse;

    /// A `spending_cap` covenant whose guard does NOT establish `amount <= limit`.
    /// `portrait-sema`'s structural `spending_cap` check would REJECT this (it
    /// requires the literal `require amount <= limit`), pre-empting it before Lens
    /// in the normal `prove_program` pipeline — defence in depth. So the
    /// refinement-class REFUTED direction is NOT reachable end-to-end through
    /// `prove_program`; we exercise it here at the builder level (parse → build the
    /// refinement VC directly, sema-bypassed) to pin that the refinement class is
    /// genuinely NON-VACUOUS: it REFUTES a real violation over a *satisfiable* `T`,
    /// never falsely PROVED and never vacuously discharged.
    const REFINE_CAP_UNGUARDED: &str = r#"pragma portrait ^0.1.0;
app RefineCapUnguarded {
  role vault {
    param pubkey owner;
    param int    balance;
    param int    limit;
    state { pubkey owner; int balance; int limit; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int balance, int limit) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      return RefineCapUnguarded { owner: owner, balance: balance - amount, limit: limit };
    }
  }
  lifecycle { live -> live via vault.withdraw; }
  invariant spending_cap;
  invariant non_negative_amount;
  invariant no_undeclared_state;
}
"#;

    /// Build the `spending_cap` refinement VC directly (bypassing sema's structural
    /// pre-emption) and return it. Panics if the fixture does not yield exactly the
    /// expected one spending_cap refinement VC.
    fn unguarded_cap_vc() -> VcReport {
        let prog = parse(REFINE_CAP_UNGUARDED).expect("fixture should parse");
        let role = &prog.app.roles[0];
        let entry = &role.entrypoints[0];
        let declared = DeclaredInvariants::from_app(&prog.app.invariants);
        let mut vcs: Vec<VcReport> = build_refinement_vcs(role, entry, &declared)
            .into_iter()
            .filter(|r| r.smtlib.contains("(not (<= amount limit))"))
            .collect();
        assert_eq!(
            vcs.len(),
            1,
            "expected exactly one spending_cap refinement VC"
        );
        vcs.pop().unwrap()
    }

    #[test]
    fn refinement_vc_negates_phi_and_keeps_t_alone_probe() {
        // SMT-shape pin (runs without z3): the full query carries the negated φ; the
        // vacuity probe is T ALONE (no negated φ), so the SAT(T) guard governs it.
        let vc = unguarded_cap_vc();
        assert!(vc.smtlib.contains("(not (<= amount limit))"));
        assert!(
            !vc.transition_smtlib.contains("(not (<= amount limit))"),
            "vacuity probe must be T alone, without the negated VC; got:\n{}",
            vc.transition_smtlib
        );
    }

    #[test]
    fn refinement_class_is_non_vacuous_and_refutes_a_real_violation() {
        // SOUNDNESS / non-vacuity: an unbounded `amount` (guard does not establish
        // `amount <= limit`) must REFUTE the spending_cap refinement — over a
        // SATISFIABLE T (the guards `amount >= 0` etc. are consistent), so the
        // REFUTED is a genuine counter-example, NOT a vacuous discharge. Confirms
        // the refinement class can reject a real violation (the REFUTED direction
        // sema pre-empts end-to-end). z3-gated; self-skips when z3 is absent.
        if !crate::z3_available() {
            eprintln!("SKIP refinement_class_is_non_vacuous: z3 not found");
            return;
        }
        let mut vc = unguarded_cap_vc();
        discharge(&mut vc, 10_000);
        match &vc.outcome {
            Some(Outcome::Refuted { model, .. }) => assert!(
                !model.trim().is_empty(),
                "REFUTED must carry a non-empty counter-model"
            ),
            other => panic!(
                "an unbounded amount must REFUTE spending_cap (non-vacuous), got {other:?}; smtlib:\n{}",
                vc.smtlib
            ),
        }
    }
}
