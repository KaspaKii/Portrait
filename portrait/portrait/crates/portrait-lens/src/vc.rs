//! Verification-condition generation: the total value-conservation VC for one
//! transition entrypoint (spec §4(a), internal-flow form).
//!
//! Builds the transition relation `T(s, p, s') ≡ G(s, p) ∧ s' = ⟦return⟧` and the
//! negated conservation VC `(assert (not (= (+ f'...) (+ f...))))`, then assembles
//! the SMT-LIB document. Mint/burn entrypoints are exempt; a clean value-out spend
//! gets the dedicated **spend VC** ([`VcKind::Spend`], M5): a fresh `spent_out`
//! var bound to the drop `Σf − Σf'`, proving `spent_out >= 0` (the model spend
//! creates no value) — NOT bound to the on-chain output amount (translation
//! validation). A transition with no value-bearing field movement, or an empty
//! value-bearing set, is refused so the VC cannot collapse to the vacuous `0 = 0`
//! (the red-team trap).

use crate::encode::{
    encode_expr, is_coin, is_value_bearing_split, sanitize, sort_of, EncodeCtx, ExpectSort, Logic,
};
use crate::smt::SmtBuilder;
use crate::LensError;
use portrait_syntax::{Entry, Expr, Field, ReturnExpr, Role, Stmt};

/// Which VC class a report covers.
///
/// M1/M2 shipped only [`VcKind::ValueConservation`]. M3 adds three further
/// classes, each grounded in the **real** surface AST and each routed through the
/// *same* SAT(T) vacuity-safe discharge (only a satisfiable `T` plus an unsat
/// negated-VC yields `Proved`):
/// - [`VcKind::Range`] — §4(c): value-field arithmetic stays within the on-chain
///   `u64` sompi width `[0, 2^64)`.
/// - [`VcKind::Refinement`] — §4(b): a declared named refinement invariant
///   (`non_negative_amount`, `spending_cap`) as a real implication `G ⟹ φ`.
/// - [`VcKind::InvariantPreservation`] — §4(d): a declared stateful invariant
///   (`bounded_supply`, `monotonic_seq`) preserved across the transition,
///   `I(s) ∧ G ⟹ I(s')`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcKind {
    /// Total value conservation over the value-bearing field set (supersedes
    /// the structural C1 + D4 checks). Spec §4(a).
    ValueConservation,
    /// Spend (value-out): on a spend transition the state total must NOT
    /// INCREASE. A fresh `spent_out >= 0` var is bound to `Σf − Σf'`, so a
    /// `Proved` says the model spend creates no value. M5 / spec §4(e). This is
    /// the formerly-deferred Q3 class, now honestly scoped: it proves the MODEL
    /// mints nothing on a spend; it does NOT bind `spent_out` to the actual
    /// on-chain output amount (that remains translation-validation / M6).
    Spend,
    /// Range / overflow: every value-field arithmetic result fits the on-chain
    /// `u64` sompi width `[0, 2^64)`. Spec §4(c).
    Range,
    /// A declared named refinement invariant as a real implication `G ⟹ φ`.
    /// Spec §4(b).
    Refinement,
    /// A declared stateful invariant preserved across the transition,
    /// `I(s) ∧ G ⟹ I(s')`. Spec §4(d).
    InvariantPreservation,
}

impl VcKind {
    /// Short human label for CLI output.
    pub fn label(self) -> &'static str {
        match self {
            VcKind::ValueConservation => "value-conservation",
            VcKind::Spend => "spend-no-value-created",
            VcKind::Range => "range-overflow",
            VcKind::Refinement => "refinement",
            VcKind::InvariantPreservation => "invariant-preservation",
        }
    }
}

/// The documented on-chain integer width assumption for range/overflow VCs:
/// Kaspa coin amounts are **`u64` sompi**, so a value-field arithmetic result
/// must lie in `[0, 2^64)`. This is a *refinement of assumption A2* (the exact
/// integer encoding): the range VC checks, over mathematical integers, that the
/// computed result never leaves the `u64` window the engraver targets. `2^64` is
/// `18446744073709551616`.
pub const U64_EXCL_MAX: &str = "18446744073709551616";

/// The value-bearing field set `V` for a role, by the **wide** split predicate.
/// Public so a regression test can assert `V` is exactly the expected set.
pub fn value_bearing_fields(role: &Role) -> Vec<&Field> {
    role.state
        .iter()
        .filter(|f| is_value_bearing_split(&f.name, &f.ty))
        .collect()
}

/// Per-field additive delta classification for a value-bearing field's return
/// value (mirrors sema's `classify_split_adjust` shape, but we only need the
/// direction; the term itself is encoded numerically, not structurally matched).
enum Delta {
    /// `f: f` — carried unchanged.
    Carry,
    /// `f: f + e` — increases.
    Increase,
    /// `f: f - e` — decreases.
    Decrease,
    /// Any other shape (constant, multiplicative, etc.). Still encoded
    /// numerically into the sum; the SMT solver reasons about the actual value.
    Other,
}

fn classify_delta(field: &str, value: &Expr) -> Delta {
    match value {
        Expr::Var(name) if name == field => Delta::Carry,
        Expr::Binary {
            op: portrait_syntax::BinOp::Add,
            lhs,
            ..
        } if matches!(lhs.as_ref(), Expr::Var(n) if n == field) => Delta::Increase,
        Expr::Binary {
            op: portrait_syntax::BinOp::Sub,
            lhs,
            ..
        } if matches!(lhs.as_ref(), Expr::Var(n) if n == field) => Delta::Decrease,
        _ => Delta::Other,
    }
}

/// A term is "constant" iff built only from literals — no state field or
/// parameter reference. Distinguishes a counter increment (`seq + 1`, constant)
/// from a value accumulation (`pot + x`, which references the moved amount `x`).
fn expr_is_constant(e: &Expr) -> bool {
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::Bytes(_) => true,
        Expr::Unary { rhs, .. } => expr_is_constant(rhs),
        Expr::Binary { lhs, rhs, .. } => expr_is_constant(lhs) && expr_is_constant(rhs),
        _ => false, // Var / Field / Index / Call → references state or a param
    }
}

/// Build the conservation VC for one transition entrypoint (M1: fills smtlib).
pub fn build_conservation_vc(role: &Role, entry: &Entry) -> Result<crate::VcReport, LensError> {
    if !matches!(entry.mode, portrait_syntax::CovenantMode::Transition) {
        return Err(LensError::Unsupported(
            "not a transition entrypoint".to_string(),
        ));
    }
    // Mint/burn exemption (parity with sema `is_mint_or_burn`): an authorised
    // supply change has no conservation VC.
    if entry.name.starts_with("mint") || entry.name.starts_with("burn") {
        return Err(LensError::Unsupported(format!(
            "{} is a mint/burn entrypoint (exempt from conservation)",
            entry.name
        )));
    }

    // A Raw (untyped) statement has no sound encoding — hard refusal (spec §3.2).
    if entry.body.iter().any(|s| matches!(s, Stmt::Raw(_))) {
        return Err(LensError::RawStmtInCovenant);
    }

    // The object return is the next-state function. (Scalar returns reference at
    // most one state field; conservation over multi-leg value sets needs the
    // object form. A scalar return is out of the conservation VC's scope.)
    let return_fields = match entry.body.iter().find_map(|s| match s {
        Stmt::Return(ReturnExpr::Object { fields, .. }) => Some(fields),
        _ => None,
    }) {
        Some(f) => f,
        None => {
            return Err(LensError::Unsupported(
                "no object-return next-state to conserve over".to_string(),
            ))
        }
    };

    let value_fields = value_bearing_fields(role);
    // Empty V ⇒ the conservation sum is the vacuous `0 = 0`, which trivially
    // PROVES. Refuse (NEVER a silent PROVED) — the exact red-team failure mode.
    if value_fields.is_empty() {
        return Err(LensError::Unsupported(
            "no value-bearing fields; conservation VC would be vacuous (0 = 0)".to_string(),
        ));
    }

    // H-3 (fail-closed): a state field that accumulates a NON-CONSTANT amount
    // (`f: f + e` / `f: f - e`, where `e` references a variable/param) yet sits
    // OUTSIDE the value-bearing set V would let value enter or leave the covenant
    // OUTSIDE the conservation sum — the "false PROVED" red-team finding
    // (`pot: pot + x` beside `pool_a: pool_a - x`). Refuse rather than prove
    // conservation over a partial V. A constant counter (`seq: seq + 1`) is not
    // value and is allowed through.
    for (field, value) in return_fields {
        if value_fields.iter().any(|f| &f.name == field) {
            continue; // inside V — covered by the conservation sum below
        }
        let accumulates_value = matches!(
            value,
            Expr::Binary { op: portrait_syntax::BinOp::Add | portrait_syntax::BinOp::Sub, lhs, rhs }
                if matches!(lhs.as_ref(), Expr::Var(n) if n == field) && !expr_is_constant(rhs)
        );
        if accumulates_value {
            return Err(LensError::Unsupported(format!(
                "field `{field}` accumulates a non-constant amount but is not in the \
                 value-bearing set V; conservation over a partial V could be a false proof \
                 — refusing (fail-closed). Make it coin-typed or name it `*balance` so it \
                 is conserved."
            )));
        }
    }

    // Classify the movement of value-bearing fields named in the return. A
    // `Delta::Other` (e.g. `f + x + x`, `f - x*2`, a constant) is a value move
    // whose *direction* is not structurally obvious — but the conservation VC
    // reasons about it NUMERICALLY, so it stays in internal-flow scope (this is
    // exactly the strength over structural D4).
    let mut increases = 0usize;
    let mut decreases = 0usize;
    let mut others = 0usize;
    let mut moved = 0usize;
    for (field, value) in return_fields {
        if !value_fields.iter().any(|f| &f.name == field) {
            continue;
        }
        match classify_delta(field, value) {
            Delta::Carry => {}
            Delta::Increase => {
                increases += 1;
                moved += 1;
            }
            Delta::Decrease => {
                decreases += 1;
                moved += 1;
            }
            Delta::Other => {
                others += 1;
                moved += 1;
            }
        }
    }
    if moved == 0 {
        return Err(LensError::Unsupported(
            "no value-bearing field moves; nothing to conserve".to_string(),
        ));
    }
    // A CLEAN value-OUT spend: one or more pure decreases, NO increase, and NO
    // ambiguous (`Other`) shape. The internal-flow conservation VC (`Σf' = Σf`)
    // would FALSELY REFUTE such a legitimate spend (value legitimately leaves the
    // covenant). M5 (formerly the Q3-deferred class) gives it its OWN, honestly
    // scoped VC: a fresh `spent_out >= 0` var bound to `Σf − Σf'`, proving the
    // MODEL spend CREATES NO value (the state total does not increase). It does
    // NOT bind `spent_out` to the actual on-chain output amount — that stays
    // translation-validation (M6). So this branch builds a `VcKind::Spend` VC
    // rather than refusing, and returns early.
    if decreases > 0 && increases == 0 && others == 0 {
        let (transition_smtlib, smtlib) = emit_spend_vc(role, entry, return_fields, &value_fields)?;
        return Ok(crate::VcReport {
            entrypoint: format!("{}.{}", role.name, entry.name),
            vc_kind: VcKind::Spend,
            smtlib,
            transition_smtlib,
            outcome: None,
        });
    }
    // A CLEAN lone increase mints value into the covenant without an authorised
    // mint/burn name. Out of the internal-flow conservation scope; defer (never a
    // silent PROVED). An `Other` shape keeps us in internal-flow scope instead.
    if increases > 0 && decreases == 0 && others == 0 {
        return Err(LensError::Unsupported(format!(
            "{} increases value with no counter-decrease (mint-like); out of internal-flow scope",
            entry.name
        )));
    }

    // Genuine internal flow: value moves BETWEEN value-bearing fields (a decrease
    // AND an increase, or an `Other` shape). The VC is `Σ_{f∈V} f' = Σ_{f∈V} f`
    // (spec §4(a)).
    let (transition_smtlib, smtlib) =
        emit_internal_flow_vc(role, entry, return_fields, &value_fields)?;

    Ok(crate::VcReport {
        entrypoint: format!("{}.{}", role.name, entry.name),
        vc_kind: VcKind::ValueConservation,
        smtlib,
        transition_smtlib,
        outcome: None,
    })
}

/// The encoded transition relation `T(s, p, s') ≡ G(s, p) ∧ s' = ⟦return⟧`,
/// shared by every VC class. Carries the populated [`SmtBuilder`] (declarations,
/// domain axioms, guards, next-state equalities — but NO negated VC) and the
/// [`Logic`] level reached. A VC class clones the builder, appends its own
/// `(assert (not VC))`, and renders.
///
/// SOUNDNESS: rendering `b` *as-is* (no negated VC) is the **vacuity probe** —
/// `T` alone. Every VC class hands that as `transition_smtlib` to [`discharge`],
/// so the SAT(T) guard (only a satisfiable `T` plus an unsat negated-VC yields
/// `Proved`) applies UNIFORMLY across all classes — no class can report `Proved`
/// on a vacuous / unreachable transition.
pub(crate) struct Transition {
    /// The SMT builder with `T` (decls + axioms + guards + next-state) asserted.
    pub builder: SmtBuilder,
    /// The least logic covering every term emitted so far.
    pub logic: Logic,
}

impl Transition {
    /// The SMT-LIB document for `T` ALONE (the vacuity probe).
    pub fn transition_only(&self) -> String {
        self.builder.finish(self.logic.name())
    }
}

/// Encode the transition relation `T` for one entrypoint, returning a populated
/// builder ready for any VC class to extend with its negated VC. Refuses (a hard
/// [`LensError`], never a verdict) on any node it cannot encode soundly.
pub(crate) fn build_transition(
    role: &Role,
    entry: &Entry,
    return_fields: &[(String, Expr)],
) -> Result<Transition, LensError> {
    // Sort bindings for every in-scope bare name (state fields + args).
    let mut bindings: Vec<(String, String)> = Vec::new();
    for f in &role.state {
        bindings.push((sanitize(&f.name), sort_of(&f.ty).to_string()));
    }
    for a in &entry.args {
        bindings.push((sanitize(&a.name), sort_of(&a.ty).to_string()));
    }
    let mut ctx = EncodeCtx::new(bindings);
    let mut logic = Logic::QfLia;
    let mut b = SmtBuilder::new();

    // ── Declarations ───────────────────────────────────────────────────────
    // Pre-state: one const per state field (bare name).
    for f in &role.state {
        b.declare_const(&sanitize(&f.name), sort_of(&f.ty));
    }
    // Params / args: one const per entrypoint arg (caller-supplied, unconstrained
    // except by the guards).
    for a in &entry.args {
        b.declare_const(&sanitize(&a.name), sort_of(&a.ty));
    }
    // Post-state: one primed const per state field.
    for f in &role.state {
        b.declare_const(&primed(&f.name), sort_of(&f.ty));
    }

    // ── Encode the transition relation T = G ∧ s' = ⟦return⟧ ──────────────────
    // (Encode FIRST so all uninterpreted functions are collected before we emit
    //  their declarations; ordering of asserts below is sound either way.)
    // Guard G: conjunction of every require (boolean context).
    let mut guard_terms: Vec<String> = Vec::new();
    for stmt in &entry.body {
        if let Stmt::Require(e) = stmt {
            let t = encode_expr(e, &mut ctx, &mut logic, ExpectSort::Bool)
                .map_err(|err| LensError::Unsupported(err.0))?;
            guard_terms.push(t);
        }
    }
    // Next-state: mentioned fields get `f' = ⟦value⟧`; unmentioned carry (`f' = f`).
    let mut next_state_terms: Vec<String> = Vec::new();
    let mut mentioned: Vec<&str> = Vec::new();
    for (field, value) in return_fields {
        mentioned.push(field.as_str());
        let t = encode_expr(value, &mut ctx, &mut logic, ExpectSort::Int)
            .map_err(|err| LensError::Unsupported(err.0))?;
        next_state_terms.push(format!("(= {} {})", primed(field), t));
    }
    for f in &role.state {
        if !mentioned.iter().any(|m| *m == f.name) {
            // Frame rule: unmentioned state field carries unchanged.
            next_state_terms.push(format!("(= {} {})", primed(&f.name), sanitize(&f.name)));
        }
    }

    // ── Declare any uninterpreted functions seen (e.g. checkSig). Kept opaque:
    //    NOT asserted true, so conservation holds independent of *who* signs.
    for (name, sig) in ctx.ufs() {
        b.declare_fun(name, &sig.args, &sig.ret);
    }

    // ── Domain axioms: coin vars are non-negative on BOTH pre- and post-state
    //    (spec §3.3) so a VC cannot be vacuously discharged by ignoring domain.
    for f in &role.state {
        if is_coin(f) {
            b.assert(&format!("(>= {} 0)", sanitize(&f.name)));
            b.assert(&format!("(>= {} 0)", primed(&f.name)));
        }
    }
    for a in &entry.args {
        if matches!(a.ty, portrait_syntax::Type::Coin) {
            b.assert(&format!("(>= {} 0)", sanitize(&a.name)));
        }
    }

    // ── Emit T: guards then next-state equalities.
    for g in &guard_terms {
        b.assert(g);
    }
    for n in &next_state_terms {
        b.assert(n);
    }

    Ok(Transition { builder: b, logic })
}

/// Emit the SMT-LIB documents for the internal-flow conservation VC.
///
/// Returns `(transition_only, full)`:
/// - `transition_only` asserts just the transition relation `T` (guards +
///   next-state binding + domain axioms) — the vacuity probe.
/// - `full` adds the negated VC `(assert (not (= Σf' Σf)))` — the conservation
///   query proper.
fn emit_internal_flow_vc(
    role: &Role,
    entry: &Entry,
    return_fields: &[(String, Expr)],
    value_fields: &[&Field],
) -> Result<(String, String), LensError> {
    let t = build_transition(role, entry, return_fields)?;
    let transition_only = t.transition_only();

    // ── Negated VC: Σ f' ≠ Σ f over the value-bearing set V ──────────────────
    let mut b = t.builder;
    let pre_sum = sum_terms(value_fields.iter().map(|f| sanitize(&f.name)));
    let post_sum = sum_terms(value_fields.iter().map(|f| primed(&f.name)));
    b.assert(&format!("(not (= {post_sum} {pre_sum}))"));

    Ok((transition_only, b.finish(t.logic.name())))
}

/// Emit the SMT-LIB documents for the **spend** VC (M5, formerly Q3-deferred).
///
/// For a clean value-out spend the obligation is that the model creates NO value:
/// the state total `Σ_{f∈V} f` does not INCREASE across the transition. We
/// introduce a FRESH SMT var `spent_out` constrained `(>= spent_out 0)` and bound
/// to the drop `(= spent_out (- Σf Σf'))`; a `Proved` then says `spent_out >= 0`
/// holds for every guard-satisfying transition, i.e. value only ever LEAVES (or
/// is conserved), never appears.
///
/// Returns `(transition_only, full)`:
/// - `transition_only` asserts just the transition relation `T` (the shared
///   vacuity probe; it does NOT include `spent_out`, so the SAT(T) guard governs
///   reachability of the *guards* alone, uniformly with the other classes).
/// - `full` adds the `spent_out` declaration + non-negativity + binding, then the
///   negated VC `(assert (> Σf' Σf))` — "value was created on the spend". `unsat`
///   ⇒ no spend creates value ⇒ PROVED; `sat` ⇒ a guard-satisfying spend mints
///   value ⇒ REFUTED with the counter-model.
///
/// HONEST SCOPE: this proves the MODEL spend mints no value. It does NOT bind
/// `spent_out` to the real on-chain output amount — the covenant model does not
/// read UTXO coin values; that binding remains translation-validation (M6).
fn emit_spend_vc(
    role: &Role,
    entry: &Entry,
    return_fields: &[(String, Expr)],
    value_fields: &[&Field],
) -> Result<(String, String), LensError> {
    let t = build_transition(role, entry, return_fields)?;
    let transition_only = t.transition_only();

    let mut b = t.builder;
    let pre_sum = sum_terms(value_fields.iter().map(|f| sanitize(&f.name)));
    let post_sum = sum_terms(value_fields.iter().map(|f| primed(&f.name)));

    // Fresh accounting var for the value that leaves the covenant on this spend,
    // bound to the drop in the conserved total. The OBLIGATION we want to prove is
    // `spent_out >= 0` (the spend mints nothing); so `(>= spent_out 0)` is the VC,
    // NOT a premise — asserting it as a hypothesis would poison the query (it would
    // constrain the free args and could make a value-minting spend vacuously hold).
    // We therefore declare + bind `spent_out`, then negate the obligation:
    // `(< spent_out 0)`, which is exactly `(> Σf' Σf)` ("value was created") under
    // the binding. `unsat` ⇒ `spent_out >= 0` always ⇒ PROVED (no value created).
    b.declare_const("spent_out", "Int");
    b.assert(&format!("(= spent_out (- {pre_sum} {post_sum}))"));

    // ── Negated VC: spent_out < 0, i.e. the state total INCREASED (value created) ─
    b.assert("(< spent_out 0)");

    Ok((transition_only, b.finish(t.logic.name())))
}

/// `f_p` — the post-state (primed) const name for a state field.
pub(crate) fn primed(name: &str) -> String {
    format!("{}_p", sanitize(name))
}

/// Build an SMT-LIB sum `(+ a b c)` from a list of term strings. A single term
/// is returned bare (so a one-field V is `balance`, not `(+ balance)`).
pub(crate) fn sum_terms<I: IntoIterator<Item = String>>(terms: I) -> String {
    let v: Vec<String> = terms.into_iter().collect();
    match v.len() {
        0 => "0".to_string(),
        1 => v.into_iter().next().unwrap(),
        _ => format!("(+ {})", v.join(" ")),
    }
}

#[cfg(test)]
mod h3_fail_closed_tests {
    use super::*;

    fn rhs_of(src: &str) -> Expr {
        match portrait_syntax::parse_expr(src).unwrap() {
            Expr::Binary { rhs, .. } => *rhs,
            other => other,
        }
    }

    #[test]
    fn constant_counter_term_vs_value_term() {
        // `seq + 1` — the delta `1` is constant (a legitimate counter increment).
        assert!(expr_is_constant(&rhs_of("seq + 1")));
        // `pot + x` — the delta `x` references a variable (a value amount).
        assert!(!expr_is_constant(&rhs_of("pot + x")));
        assert!(expr_is_constant(
            &portrait_syntax::parse_expr("2 + 3").unwrap()
        ));
        assert!(!expr_is_constant(
            &portrait_syntax::parse_expr("x + 3").unwrap()
        ));
    }

    // A value amount `x` is moved out of a real value leg AND accumulated into a
    // NON-value-bearing int field `pot` — the money-printing shape. Conservation
    // over V = {pool_*_balance} would falsely PROVE (pot escapes the sum).
    const ADV_POT: &str = r#"pragma portrait ^0.1.0;
app AdvPot {
  role pool {
    param int pool_a_balance; param int pool_b_balance; param int pot; param pubkey owner;
    state { int pool_a_balance; int pool_b_balance; int pot; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function rebalance(sig auth, int x) : (int pool_a_balance, int pool_b_balance, int pot, pubkey owner) {
      requires checkSig(auth, owner); requires x >= 0; requires x <= pool_a_balance;
      return AdvPot { pool_a_balance: pool_a_balance - x, pool_b_balance: pool_b_balance + x, pot: pot + x, owner: owner };
    }
  }
  lifecycle { live -> live via pool.rebalance; }
  invariant authorized; invariant no_undeclared_state;
}"#;

    // Same split, but the non-V field is a CONSTANT counter (`seq + 1`) — legit;
    // conservation over V must still be built.
    const CTRL: &str = r#"pragma portrait ^0.1.0;
app CtrlCounter {
  role pool {
    param int pool_a_balance; param int pool_b_balance; param int seq; param pubkey owner;
    state { int pool_a_balance; int pool_b_balance; int seq; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function rebalance(sig auth, int x) : (int pool_a_balance, int pool_b_balance, int seq, pubkey owner) {
      requires checkSig(auth, owner); requires x >= 0; requires x <= pool_a_balance;
      return CtrlCounter { pool_a_balance: pool_a_balance - x, pool_b_balance: pool_b_balance + x, seq: seq + 1, owner: owner };
    }
  }
  lifecycle { live -> live via pool.rebalance; }
  invariant authorized; invariant no_undeclared_state;
}"#;

    #[test]
    fn conservation_fails_closed_on_value_into_non_v_field() {
        let prog = portrait_syntax::parse(ADV_POT).expect("adv_pot parses");
        let reports = crate::prove_program(&prog).expect("prove_program ok");
        assert!(
            !reports
                .iter()
                .any(|r| matches!(r.vc_kind, VcKind::ValueConservation)),
            "value accumulated into a non-V field must NOT yield a value-conservation VC (fail-closed)"
        );
    }

    #[test]
    fn conservation_still_built_with_constant_counter() {
        let prog = portrait_syntax::parse(CTRL).expect("ctrl parses");
        let reports = crate::prove_program(&prog).expect("prove_program ok");
        assert!(
            reports
                .iter()
                .any(|r| matches!(r.vc_kind, VcKind::ValueConservation)),
            "a constant counter (seq + 1) must not block the conservation VC"
        );
    }
}
