//! Structural static checks over the Portrait AST (BUILD_SPEC §4.4, §5).
//!
//! This is NOT a full type system. It is a set of *structural* static checks
//! plus a reject-vector suite — the honest core of "Portrait is a language, not
//! a templater". The checks operate purely on the surface AST shape:
//!
//! 1. Lifecycle reachability — every edge's `via_role` / `via_entry` resolves.
//! 2. Flow integrity — every `Step::Move` resolves to a real role + entrypoint.
//! 3. Transition/return consistency — a `Transition` entrypoint referenced by a
//!    non-terminal lifecycle edge must `return`; a `Verification` entrypoint must
//!    not. (Mirrors silverc's own transition/verification fn rule.)
//! 4. `value_conserved` invariant — every reachable `Transition` entrypoint must
//!    `return` (structural proxy for non-destruction of state).
//! 5. `no_undeclared_state` invariant — no dangling lifecycle state: every
//!    non-terminal edge target must itself originate an edge or be a terminal.
//!
//! Type inference, refinement checking, and linearity are explicitly out of scope
//! here — see the module-level note and BUILD_SPEC §5 for the type-stack roadmap.
//!
//! # SOUNDNESS — what `value_conserved` (C1) does NOT prove
//!
//! C1 is a *per-field structural shape* guard, not a flow solver. Read this
//! before relying on it for an economic safety argument:
//!
//! * **No cross-field flow conservation (in C1 itself).** C1 checks each
//!   value-bearing field's new value *in isolation* — that it is a bare carry
//!   `f: f` or a single additive `f: f ± e`. It does NOT verify that value
//!   *moved between* fields nets to zero. A transition returning `{ balance:
//!   balance - amount, fee: fee + amount }` and one returning `{ balance:
//!   balance - amount, fee: fee + amount + amount }` are INDISTINGUISHABLE to
//!   C1: both fields pass the per-field shape rule, yet only the first conserves
//!   total value. C1 never sums the deltas across fields.
//!
//!   The OPT-IN `conservation_split` invariant (D4, below) now closes this gap
//!   *structurally* for INTERNAL transfers/splits across N value-bearing fields:
//!   when declared, it computes the additive delta of every value-bearing field
//!   in the return and requires the added `+`-atoms to cancel the subtracted
//!   `-`-atoms by `Expr` structural equality (so `a: a - (x + y)`, `b: b + x`,
//!   `c: c + y` is accepted, but a delta that does not net to zero is rejected).
//!   This is STRUCTURAL N-field additive-delta arithmetic, NOT an SMT proof — it
//!   does not reason about numeric values, conditionals, or arithmetic
//!   identities (`x * 2` is not seen as `x + x`), and it models INTERNAL flows
//!   only, NOT a spend that moves value OUT of the covenant. Covenants that
//!   legitimately spend value out keep using `value_conserved` and do not
//!   declare `conservation_split`. Full SMT / arbitrary value properties remain
//!   future work.
//! * **No arithmetic reasoning.** C1 does not know that `balance - amount` can
//!   underflow, that `supply + amount` can overflow, or that the operands have
//!   any particular range. It matches the *expression shape*, not its values.
//! * **No magnitude check on the adjustment term.** The `e` in `f ± e` is
//!   unconstrained by C1 — it only forbids `e` from re-referencing `f`. Bounding
//!   `e` (e.g. non-negativity, or a ceiling) is the job of the opt-in C3
//!   refinements (`non_negative_amount`, `bounded_supply`), each itself a narrow
//!   structural pattern match, not a proof.
//!
//! In short: C1/C3 reject the blunt supply-inflation / value-destruction /
//! missing-authorization shapes and pin a small set of explicitly-declared
//! refinement patterns; the opt-in `conservation_split` adds structural N-field
//! internal cross-field cancellation on top. They are NOT a proof of economic
//! soundness — a covenant that passes them can still be wrong about properties
//! that need a solver (numeric ranges, conditionals, arithmetic identities, and
//! value flow that crosses the covenant boundary).

use std::collections::HashMap;

use portrait_syntax::{
    BinOp, CovenantMode, Expr, Flow, Invariant, Program, ReturnExpr, Role, Step, Stmt, Type, UnOp,
};

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
}

impl Diagnostic {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Run the structural static checks. Returns `Err` with one diagnostic per
/// violation found; `Ok(())` if the program is structurally well-formed.
pub fn check(program: &Program) -> Result<(), Vec<Diagnostic>> {
    let app = &program.app;
    let mut diags: Vec<Diagnostic> = Vec::new();

    // 1. Lifecycle reachability: every via_role / via_entry must resolve.
    for edge in &app.lifecycle {
        match find_role(&app.roles, &edge.via_role) {
            None => diags.push(Diagnostic::new(format!(
                "lifecycle edge {} -> {} references unknown role `{}`",
                edge.from, edge.to, edge.via_role
            ))),
            Some(role) => {
                if find_entry(role, &edge.via_entry).is_none() {
                    diags.push(Diagnostic::new(format!(
                        "lifecycle edge {} -> {} references unknown entrypoint `{}.{}`",
                        edge.from, edge.to, edge.via_role, edge.via_entry
                    )));
                }
            }
        }
    }

    // 2. Flow integrity: every Step::Move must resolve to a real role + entry.
    if let Some(flow) = &app.flow {
        check_flow(flow, &app.roles, &mut diags);
    }

    // 3. Transition/return consistency.
    //    - A Transition entrypoint named by a *non-terminal* lifecycle edge must
    //      contain a Return (it cannot silently drop the new state).
    //    - A Verification entrypoint must NOT contain a Return.
    for edge in &app.lifecycle {
        if let Some(role) = find_role(&app.roles, &edge.via_role) {
            if let Some(entry) = find_entry(role, &edge.via_entry) {
                let returns = has_return(&entry.body);
                match entry.mode {
                    CovenantMode::Transition if !edge.terminal && !returns => {
                        diags.push(Diagnostic::new(format!(
                            "transition entrypoint `{}.{}` is reachable by non-terminal edge \
                             {} -> {} but has no return statement (would silently drop state)",
                            edge.via_role, edge.via_entry, edge.from, edge.to
                        )));
                    }
                    CovenantMode::Verification if returns => {
                        diags.push(Diagnostic::new(format!(
                            "verification entrypoint `{}.{}` must not return a value",
                            edge.via_role, edge.via_entry
                        )));
                    }
                    _ => {}
                }
            }
        }
    }

    // 4. value_conserved: every Transition entrypoint reachable from the
    //    lifecycle must have a return (structural proxy for non-destruction).
    if app.invariants.iter().any(is_value_conserved) {
        for edge in &app.lifecycle {
            if let Some(role) = find_role(&app.roles, &edge.via_role) {
                if let Some(entry) = find_entry(role, &edge.via_entry) {
                    if matches!(entry.mode, CovenantMode::Transition) && !has_return(&entry.body) {
                        diags.push(Diagnostic::new(format!(
                            "invariant `value_conserved` violated: reachable transition \
                             entrypoint `{}.{}` has no return statement (state not conserved)",
                            edge.via_role, edge.via_entry
                        )));
                    }
                }
            }
        }
    }

    // 5. no_undeclared_state: no dangling lifecycle state. Every non-terminal
    //    edge target must itself originate an edge, OR be a recognized terminal
    //    (named as the `to` of some edge whose `terminal` flag is set).
    if app.invariants.iter().any(is_no_undeclared_state) {
        let froms: Vec<&str> = app.lifecycle.iter().map(|e| e.from.as_str()).collect();
        let terminals: Vec<&str> = app
            .lifecycle
            .iter()
            .filter(|e| e.terminal)
            .map(|e| e.to.as_str())
            .collect();
        for edge in &app.lifecycle {
            if edge.terminal {
                continue;
            }
            let to = edge.to.as_str();
            let originates = froms.contains(&to);
            let is_terminal_state = terminals.contains(&to);
            if !originates && !is_terminal_state {
                diags.push(Diagnostic::new(format!(
                    "invariant `no_undeclared_state` violated: state `{}` is entered \
                     (edge {} -> {}) but never declared as a source state or terminal",
                    to, edge.from, edge.to
                )));
            }
        }
    }

    // 6. Expression typing: walk every typed entrypoint body and reject
    //    ill-typed `require`/`return` expressions. `Stmt::Raw` bodies are untyped
    //    holes (the parser could not parse them) — they are recorded but skipped
    //    for typing, never crashed on. See `check_role_exprs`.
    for role in &app.roles {
        check_role_exprs(role, &mut diags);
    }

    // 7. C1–C3 type-stack checks (structural / simple-relational — NOT SMT).
    //    These walk the typed `Expr` tree per role and add:
    //      C1  value-conservation (real arithmetic, not the §4 structural proxy)
    //      C2  capability / authorization (checkSig must bind committed state)
    //      C3  refinement predicates (monotonic seq, non-negativity)
    //    declared via custom invariants. All are conservative and never fire on
    //    a `Stmt::Raw` hole. See the module-level note and each `check_c*` fn.
    let value_conserved = app.invariants.iter().any(is_value_conserved);
    let declares = |needle: &str| {
        app.invariants
            .iter()
            .any(|inv| matches!(inv, Invariant::Custom(s) if s == needle))
    };
    let want_monotonic_seq = declares("monotonic_seq");
    let want_non_negative_amount = declares("non_negative_amount");
    let want_bounded_supply = declares("bounded_supply");
    let want_spending_cap = declares("spending_cap");
    let want_multisig_threshold = declares("multisig_threshold");
    let want_temporal_guard = declares("temporal_guard");
    let want_conservation_split = declares("conservation_split");
    // LOW-2: require authorization on state-mutating transitions when the app
    // declares a protection invariant (`value_conserved` or custom `authorized`).
    let require_auth = value_conserved || declares(AUTH_INVARIANT);
    for role in &app.roles {
        check_c1_value_conservation(role, value_conserved, &mut diags);
        check_c2_authorization(role, require_auth, &mut diags);
        check_c3_refinements(
            role,
            want_monotonic_seq,
            want_non_negative_amount,
            want_bounded_supply,
            want_spending_cap,
            want_multisig_threshold,
            want_temporal_guard,
            &committed_keys(role),
            &mut diags,
        );
        if want_conservation_split {
            check_conservation_split(role, &mut diags);
        }
    }

    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}

// ── Expression type checker (Phase B3) ──────────────────────────────────────
//
// Builds a typing environment per entrypoint (role params + role state fields +
// entrypoint args + the implicit `prev_states: State[]`) and walks every typed
// `Expr` in the body, rejecting:
//   * arithmetic on non-int operands              (int × int -> int)
//   * comparisons across mismatched operand types (T × T -> bool)
//   * `&&`/`||`/`require()` operands that are not bool
//   * unary `-` on non-int / unary `!` on non-bool
//   * unknown variables / unknown fields
//   * `return` object/scalar field exprs whose type does not match the declared
//     state field type
//   * mis-typed builtin calls (checkSig(sig, pubkey) -> bool;
//     OpInputCovenantId(int) -> bytes32)
//
// `Stmt::Raw` bodies are skipped (untyped holes) — recorded honestly, not typed.

/// An expression type. Mostly mirrors `portrait_syntax::Type`, plus the two
/// synthetic shapes the implicit `prev_states` binding introduces.
#[derive(Debug, Clone, PartialEq)]
enum Ty {
    /// A concrete surface type (int, bool, bytes32, pubkey, sig, coin, …).
    Surface(Type),
    /// The record type of a single prior state — its fields are the role's
    /// declared `state { … }` fields. Produced by indexing `prev_states`.
    State,
    /// `State[]` — the type of the implicit `prev_states` binding itself.
    StateArray,
}

impl Ty {
    fn int() -> Ty {
        Ty::Surface(Type::Int)
    }
    fn bool() -> Ty {
        Ty::Surface(Type::Bool)
    }
    fn display(&self) -> String {
        match self {
            Ty::Surface(t) => format!("{t:?}"),
            Ty::State => "State".to_string(),
            Ty::StateArray => "State[]".to_string(),
        }
    }
    /// True for the scalar surface types that may appear as a comparison operand
    /// (int/bool/bytes32/pubkey/sig/coin). `set`/`map`/`Named` aggregates and
    /// the synthetic `State`/`State[]` shapes are excluded (fail-closed).
    fn is_scalar_surface(&self) -> bool {
        matches!(
            self,
            Ty::Surface(
                Type::Int | Type::Bool | Type::PubKey | Type::Sig | Type::Bytes32 | Type::Coin
            )
        )
    }
}

/// Per-entrypoint typing environment.
struct TyEnv {
    /// Variable name -> type (params, state fields, args, `prev_states`).
    vars: HashMap<String, Ty>,
    /// State field name -> declared type, for `prev_states[i].field` resolution
    /// and for checking return object fields.
    state_fields: HashMap<String, Type>,
}

/// Type-check every entrypoint body in a role, pushing one diagnostic per defect.
fn check_role_exprs(role: &Role, diags: &mut Vec<Diagnostic>) {
    // Base environment shared by all entrypoints: params + state fields, plus the
    // implicit `prev_states: State[]`.
    let mut state_fields: HashMap<String, Type> = HashMap::new();
    for f in &role.state {
        state_fields.insert(f.name.clone(), f.ty.clone());
    }

    for entry in &role.entrypoints {
        let mut vars: HashMap<String, Ty> = HashMap::new();
        // Role params (constructor / policy params) are in scope.
        for p in &role.params {
            vars.insert(p.name.clone(), Ty::Surface(p.ty.clone()));
        }
        // State fields are referenced bare in entrypoint bodies (the emitter
        // lowers them to prev_states[0].field).
        for f in &role.state {
            vars.insert(f.name.clone(), Ty::Surface(f.ty.clone()));
        }
        // Entrypoint arguments.
        for a in &entry.args {
            vars.insert(a.name.clone(), Ty::Surface(a.ty.clone()));
        }
        // Implicit prior-states binding.
        vars.insert("prev_states".to_string(), Ty::StateArray);

        let env = TyEnv {
            vars,
            state_fields: state_fields.clone(),
        };

        let where_ = |what: &str| format!("`{}.{}`: {}", role.name, entry.name, what);

        for stmt in &entry.body {
            match stmt {
                Stmt::Require(expr) => match type_of(expr, &env) {
                    Ok(ty) if ty == Ty::bool() => {}
                    Ok(ty) => diags.push(Diagnostic::new(where_(&format!(
                        "require(...) operand must be bool, found {}",
                        ty.display()
                    )))),
                    Err(msg) => diags.push(Diagnostic::new(where_(&msg))),
                },
                Stmt::Return(ret) => check_return(ret, entry, &env, diags, &where_),
                // Fail-CLOSED guard (robust fix, adversarial-verify follow-up): a
                // `Stmt::Raw` is an untyped hole the parser fell back on. The
                // emitter only consumes `Require`/`Return`, so a Raw that survives
                // to a COVENANT-role entrypoint (transition / verification) is
                // silently dropped — the covenant would carry state forward while
                // enforcing none of that statement's intent (a FALSE ACCEPT). The
                // blacklisted out-of-subset forms are already loud-rejected at
                // parse via REJECTION_SET; this closes the remaining class —
                // *non-blacklisted* unrecognised forms — so nothing untyped can
                // reach emit. Empirically safe: all 31 covenant sources lower with
                // ZERO Raw in any covenant transition (audited 2026-06-28), so no
                // legitimate covenant relies on Raw reaching emit. NonCovenant
                // (vProgs) entrypoints are NOT emitted to a .sil here (Atelier owns
                // them), so a Raw there is not projected to a covenant and is left
                // as a recorded hole rather than a hard error.
                Stmt::Raw(text) => {
                    if !matches!(entry.mode, CovenantMode::NonCovenant) {
                        diags.push(Diagnostic::new(where_(&format!(
                            "unsupported/untyped statement `{}` cannot be projected to a \
                             covenant; route it to the vProgs (Tier-3) layer (it parsed to an \
                             untyped Stmt::Raw hole the emitter would silently drop)",
                            text.trim()
                        ))));
                    }
                }
            }
        }
    }
}

/// Check a `return` against the declared state/return types.
fn check_return(
    ret: &ReturnExpr,
    entry: &portrait_syntax::Entry,
    env: &TyEnv,
    diags: &mut Vec<Diagnostic>,
    where_: &dyn Fn(&str) -> String,
) {
    match ret {
        ReturnExpr::Scalar(expr) => {
            // Red-team LOW (c): a scalar return is broadcast by the emitter into
            // EVERY state field its expression references (see portrait-emit's
            // `expr_references_var` loop). When a scalar references more than one
            // state field, that over-broadcast silently overwrites multiple
            // fields with the same expression — almost never what the author
            // means. Fail-closed: require an explicit object return in that case.
            let mut referenced: Vec<&str> = env
                .state_fields
                .keys()
                .filter(|f| references_var(expr, f))
                .map(|s| s.as_str())
                .collect();
            if referenced.len() > 1 {
                referenced.sort_unstable();
                diags.push(Diagnostic::new(where_(&format!(
                    "scalar return references multiple state fields ({}); a scalar return is \
                     broadcast into every referenced field, which over-writes them all — use an \
                     explicit object return (`Name {{ field: expr, ... }}`) to assign each field",
                    referenced.join(", ")
                ))));
            }
            // A scalar return maps to the entrypoint's single declared return
            // field; check the expression types and (when a return type is
            // declared) that it matches.
            match type_of(expr, env) {
                Ok(ty) => {
                    if let Some(declared) = &entry.returns {
                        let want = Ty::Surface(declared.clone());
                        if ty != want {
                            diags.push(Diagnostic::new(where_(&format!(
                                "return expression has type {} but the entrypoint declares \
                                 return type {}",
                                ty.display(),
                                want.display()
                            ))));
                        }
                    }
                }
                Err(msg) => diags.push(Diagnostic::new(where_(&msg))),
            }
        }
        ReturnExpr::Object { fields, .. } => {
            for (field, expr) in fields {
                let value_ty = match type_of(expr, env) {
                    Ok(t) => t,
                    Err(msg) => {
                        diags.push(Diagnostic::new(where_(&format!(
                            "in return field `{field}`: {msg}"
                        ))));
                        continue;
                    }
                };
                match env.state_fields.get(field) {
                    None => diags.push(Diagnostic::new(where_(&format!(
                        "return assigns unknown state field `{field}`"
                    )))),
                    Some(declared) => {
                        let want = Ty::Surface(declared.clone());
                        if value_ty != want {
                            diags.push(Diagnostic::new(where_(&format!(
                                "return field `{field}` has type {} but state declares it as {}",
                                value_ty.display(),
                                want.display()
                            ))));
                        }
                    }
                }
            }
        }
    }
}

// ── C1: value-conservation (structural arithmetic, NOT SMT) ─────────────────
//
// HONEST SCOPE: this is a *structural* arithmetic check over the typed return
// object, not a solver. Under the `value_conserved` invariant, a state field is
// "value-bearing" iff its declared type is `coin` OR its name is one of the
// conventional balance names (`balance`, `amount`, `supply`). A value-bearing
// field's new value must be *conservation-preserving* — exactly one of:
//   * a bare carry            `f: f`
//   * an additive adjustment  `f: f + e`  /  `f: f - e`  /  `f: e + f`
//     where the field token `f` appears EXACTLY ONCE at the top level and the
//     top-level operator is `+` or `-` (never `*`), and the other operand `e`
//     does not itself reference `f`.
//
// Hardening (Phase C red-team LOW-1): the previous rule accepted ANY expression
// that merely *referenced* the field somewhere, which let inflation/zeroing slip
// through via self-reference — e.g. `balance: balance * 1000` (scaling),
// `balance: balance - balance` (self-zeroing), or a constant replacement
// `balance: 0`. The conservation-preserving form below rejects all of these:
//   · multiplicative/scaling  (`*`, or any non-±  top-level op)
//   · the field on BOTH sides  (`f - f`, `f + f`, `f * f`)
//   · constant / foreign-only replacement that never carries the prior `f`
// while still accepting the only two legitimate shapes a conserving transition
// uses (bare carry and single additive ±). Mint/burn entrypoints (name begins
// with `mint`/`burn`) remain exempt as an authorised supply change.
//
// This deliberately does NOT reason about cross-field flow (e.g. "amount moved
// from balance to fee" sums to zero) — that needs a solver. It is a structural
// guard against the blunt supply-inflation / value-destruction shapes only.

/// Conventional value-bearing field names (see C1 note). A field is also
/// value-bearing if its declared type is `coin`.
const VALUE_BEARING_NAMES: &[&str] = &["balance", "amount", "supply"];

fn is_value_bearing(name: &str, ty: &Type) -> bool {
    matches!(ty, Type::Coin) || VALUE_BEARING_NAMES.contains(&name)
}

/// Mint/burn convention: an entrypoint whose name begins with `mint` or `burn`
/// is an authorised supply change and is exempt from C1.
fn is_mint_or_burn(entry_name: &str) -> bool {
    entry_name.starts_with("mint") || entry_name.starts_with("burn")
}

/// True if `expr` references the bare variable `field` somewhere in its tree —
/// the structural test for "derives from its own prior value".
fn references_var(expr: &Expr, field: &str) -> bool {
    match expr {
        Expr::Var(name) => name == field,
        Expr::Field { base, .. } => references_var(base, field),
        Expr::Index { base, index } => references_var(base, field) || references_var(index, field),
        Expr::Unary { rhs, .. } => references_var(rhs, field),
        Expr::Binary { lhs, rhs, .. } => references_var(lhs, field) || references_var(rhs, field),
        Expr::Call { args, .. } => args.iter().any(|a| references_var(a, field)),
        Expr::Int(_) | Expr::Bool(_) | Expr::Bytes(_) => false,
    }
}

/// True if `value` is a *conservation-preserving* assignment to value-bearing
/// field `field` (LOW-1). Exactly two shapes are accepted:
///
/// 1. bare carry            `field`
/// 2. additive adjustment   `field ± e` / `e ± field`  (top-level op `+`/`-`)
///    where `field` appears on exactly ONE side at the top level and the other
///    operand `e` does not itself reference `field`.
///
/// Everything else — multiplicative (`*`), constant replacement, foreign vars,
/// or the field on both sides (`field - field`) — is NOT conservation-preserving.
fn is_conservation_preserving(field: &str, value: &Expr) -> bool {
    // Shape 1: bare carry `field`.
    if matches!(value, Expr::Var(name) if name == field) {
        return true;
    }
    // Shape 2: a single top-level additive adjustment with `field` on exactly
    // one side and no second reference to `field` anywhere in the expression.
    if let Expr::Binary { op, lhs, rhs } = value {
        if matches!(op, BinOp::Add | BinOp::Sub) {
            let lhs_is_field = matches!(lhs.as_ref(), Expr::Var(name) if name == field);
            let rhs_is_field = matches!(rhs.as_ref(), Expr::Var(name) if name == field);
            // Exactly one side must be the bare field, and the OTHER side must
            // not reference it (rejects `field - field`, `field + field`, and
            // nested re-references like `field - (field + 1)`).
            if lhs_is_field && !rhs_is_field && !references_var(rhs, field) {
                return true;
            }
            if rhs_is_field && !lhs_is_field && !references_var(lhs, field) {
                return true;
            }
        }
    }
    false
}

fn check_c1_value_conservation(role: &Role, value_conserved: bool, diags: &mut Vec<Diagnostic>) {
    if !value_conserved {
        return;
    }
    let state: HashMap<&str, &Type> = role
        .state
        .iter()
        .map(|f| (f.name.as_str(), &f.ty))
        .collect();
    for entry in &role.entrypoints {
        if !matches!(entry.mode, CovenantMode::Transition) {
            continue;
        }
        if is_mint_or_burn(&entry.name) {
            continue;
        }
        for stmt in &entry.body {
            match stmt {
                Stmt::Return(ReturnExpr::Object { fields, .. }) => {
                    for (field, value) in fields {
                        let Some(ty) = state.get(field.as_str()) else {
                            continue; // unknown field already reported by the type pass
                        };
                        if is_value_bearing(field, ty) && !is_conservation_preserving(field, value)
                        {
                            diags.push(Diagnostic::new(format!(
                                "`{}.{}`: invariant `value_conserved` violated: value-bearing \
                                 field `{f}` is assigned `{}` which is not conservation-preserving \
                                 (must be a bare carry `{f}: {f}` or a single additive adjustment \
                                 `{f}: {f} + e` / `{f}: {f} - e`; multiplicative, constant, or \
                                 double-self forms create or destroy value); mark the entrypoint \
                                 as a mint/burn if intentional",
                                role.name,
                                entry.name,
                                value.to_silverscript(),
                                f = field,
                            )));
                        }
                    }
                }
                Stmt::Return(ReturnExpr::Scalar(expr)) => {
                    // A scalar return is broadcast by the emitter into the single
                    // state field it references (at most one, already enforced by
                    // LOW-c). If that field is value-bearing under value_conserved,
                    // apply the same conservation-preserving shape guard.
                    if let Some((field, ty)) =
                        state.iter().find(|(fname, _)| references_var(expr, fname))
                    {
                        if is_value_bearing(field, ty) && !is_conservation_preserving(field, expr) {
                            diags.push(Diagnostic::new(format!(
                                "`{}.{}`: invariant `value_conserved` violated: scalar return \
                                 assigns value-bearing field `{f}` via expression `{}` which is \
                                 not conservation-preserving (must be a bare carry `{f}` or a \
                                 single additive adjustment `{f} + e` / `{f} - e`; multiplicative, \
                                 constant, or double-self forms create or destroy value); mark the \
                                 entrypoint as a mint/burn if intentional",
                                role.name,
                                entry.name,
                                expr.to_silverscript(),
                                f = field,
                            )));
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

// ── C2: capability / authorization ──────────────────────────────────────────
//
// A state-mutating transition (mode = transition with an object return) must
// authorize against COMMITTED state, not a caller-supplied value. `checkSig`'s
// second operand (the pubkey) must be a role param or a state field (both are
// baked into the covenant at genesis — immutable, part of the covenant ID) or a
// `prev_states[i].field` access. If the only checkSig in the body authorizes
// against an entrypoint ARGUMENT (a caller-supplied pubkey), the spend can be
// authorised by any key the caller chooses — the DigitalReit L1 finding — and
// is rejected at compile time.
//
// LOW-2 (Phase C red-team) — no-checkSig state mutation. A state-mutating
// transition with ZERO authorization was previously accepted silently (deemed
// "out of capability scope"). The honest, conservative fail-safe: when the app
// declares a stake in correctness that authorization protects — i.e.
// `value_conserved` is declared, or a custom `authorized` invariant is declared
// — a state-mutating transition with NO checkSig at all is REJECTED with a clear
// message. The author must either add a committed-key checkSig or, if the
// transition is genuinely gated by covenant-ID lineage rather than a signature,
// mark it mint/burn (the documented authorised-change convention) to opt out.
//
// Without such an invariant, no-auth transitions remain PERMITTED and documented
// as such: they may be legitimately gated by other on-chain means (e.g. the
// covenant-ID lineage edge `parent_kov_id == OpInputCovenantId(0)`), which C2
// cannot see. We do not block those — we only require authorization where the
// app has explicitly asked for the protection an invariant implies.

/// Custom invariant that opts a role into the LOW-2 no-auth fail-safe even when
/// `value_conserved` is absent.
const AUTH_INVARIANT: &str = "authorized";

fn check_c2_authorization(role: &Role, require_auth: bool, diags: &mut Vec<Diagnostic>) {
    let arg_pubkeys_per_entry = |entry: &portrait_syntax::Entry| -> Vec<String> {
        entry
            .args
            .iter()
            .filter(|a| a.ty == Type::PubKey)
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
    };
    // Committed names: role params + state fields (both genesis-baked).
    let mut committed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in &role.params {
        committed.insert(p.name.clone());
    }
    for f in &role.state {
        committed.insert(f.name.clone());
    }

    for entry in &role.entrypoints {
        if !matches!(entry.mode, CovenantMode::Transition) {
            continue;
        }
        // Judge state-mutating transitions: those with an object return OR a
        // scalar return that references at least one state field (the emitter
        // broadcasts the scalar expr into that field, so it is a mutation).
        let state_field_names: Vec<&str> = role.state.iter().map(|f| f.name.as_str()).collect();
        let mutates = entry.body.iter().any(|s| match s {
            Stmt::Return(ReturnExpr::Object { .. }) => true,
            Stmt::Return(ReturnExpr::Scalar(expr)) => {
                state_field_names.iter().any(|f| references_var(expr, f))
            }
            _ => false,
        });
        if !mutates {
            continue;
        }
        let arg_pubkeys = arg_pubkeys_per_entry(entry);
        let checksigs: Vec<&Expr> = entry
            .body
            .iter()
            .filter_map(|s| match s {
                Stmt::Require(e) => Some(e),
                _ => None,
            })
            .flat_map(collect_checksig_pubkeys)
            .collect();
        if checksigs.is_empty() {
            // LOW-2 fail-safe: a state-mutating transition with NO authorization
            // at all. Reject only when the app declared a protection invariant
            // (`value_conserved` or custom `authorized`). Mint/burn entrypoints
            // are the documented authorised-change opt-out.
            if require_auth && !is_mint_or_burn(&entry.name) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: capability check failed: state-mutating transition has NO \
                     authorization (no checkSig) under a declared protection invariant \
                     (`value_conserved`/`authorized`); add a checkSig binding a committed key \
                     (role param, state field, or prev_states[..]), or mark the entrypoint \
                     mint/burn if it is gated by covenant-ID lineage rather than a signature",
                    role.name, entry.name
                )));
            }
            continue; // no checkSig target to judge below
        }
        // The transition is authorized against committed state iff at least one
        // checkSig binds a committed pubkey (param / state field / prev_states).
        let any_committed = checksigs
            .iter()
            .any(|pk| pubkey_is_committed(pk, &committed));
        if !any_committed {
            // Every checkSig binds a caller-supplied arg → under-authorized.
            let offending = checksigs
                .iter()
                .find_map(|pk| match pk {
                    Expr::Var(n) if arg_pubkeys.contains(n) => Some(n.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "<caller-supplied>".to_string());
            diags.push(Diagnostic::new(format!(
                "`{}.{}`: capability check failed: state-mutating transition authorizes only \
                 against a caller-supplied pubkey `{}`; checkSig must bind a committed key (a \
                 role param, state field, or prev_states[..] field)",
                role.name, entry.name, offending
            )));
        }
    }
}

/// Collect the pubkey (2nd) operand of every `checkSig(sig, pubkey)` call inside
/// an expression tree.
fn collect_checksig_pubkeys(expr: &Expr) -> Vec<&Expr> {
    let mut out = Vec::new();
    fn walk<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
        match e {
            Expr::Call { name, args } if name == "checkSig" && args.len() == 2 => {
                out.push(&args[1]);
                for a in args {
                    walk(a, out);
                }
            }
            Expr::Call { args, .. } => {
                for a in args {
                    walk(a, out);
                }
            }
            Expr::Unary { rhs, .. } => walk(rhs, out),
            Expr::Binary { lhs, rhs, .. } => {
                walk(lhs, out);
                walk(rhs, out);
            }
            Expr::Field { base, .. } => walk(base, out),
            Expr::Index { base, index } => {
                walk(base, out);
                walk(index, out);
            }
            Expr::Int(_) | Expr::Bool(_) | Expr::Bytes(_) | Expr::Var(_) => {}
        }
    }
    walk(expr, &mut out);
    out
}

/// True if a checkSig pubkey operand is bound to committed state: a bare
/// committed var (param / state field) or a `prev_states[i].field` access.
fn pubkey_is_committed(pk: &Expr, committed: &std::collections::HashSet<String>) -> bool {
    match pk {
        Expr::Var(name) => committed.contains(name),
        // prev_states[i].field — any field of a prior committed state.
        Expr::Field { base, .. } => matches!(
            base.as_ref(),
            Expr::Index { base: arr, .. } if matches!(arr.as_ref(), Expr::Var(n) if n == "prev_states")
        ),
        _ => false,
    }
}

// ── C3: refinement predicates (simple relational, opt-in via invariants) ─────
//
// NARROW + opt-in: these checks fire only when the app explicitly declares the
// matching custom invariant, so no existing source is ever false-rejected.
//
//   invariant monotonic_seq;       — every state-mutating transition must
//                                    advance a `seq` field by exactly one, i.e.
//                                    the return assigns `seq: seq + 1` (or the
//                                    body requires `next_seq == seq + 1`).
//   invariant non_negative_amount; — every transition taking an int `amount`
//                                    arg must `require amount >= 0` (or `> ...`
//                                    with a 0 lower bound). Absence is rejected.
//   invariant bounded_supply;      — a ceiling/envelope predicate. When the role
//                                    has int fields named `supply` and `total`
//                                    and a transition takes an int `amount` arg,
//                                    that transition must `require supply +
//                                    amount <= total` (in either operand order:
//                                    `amount + supply <= total` is equivalent).
//                                    This is exactly the StreamingVesting
//                                    cumulative-draw envelope ("the running
//                                    accumulator plus this draw never exceeds the
//                                    committed grant ceiling"). It is a STRUCTURAL
//                                    pattern match on the require shape — NOT an
//                                    SMT proof that the arithmetic cannot
//                                    overflow or that `total` is itself sound.

// ── D3: round-2 refinement predicates (simple relational, opt-in) ────────────
//
// Three further NARROW, opt-in refinements, in the same style as the C3 set
// above — each fires ONLY when its custom invariant is declared, so no existing
// source is false-rejected. Each is a STRUCTURAL shape match on the `require`
// AST, NOT a semantic/SMT proof:
//
//   invariant spending_cap;        — when a transition takes an int `amount`
//                                    arg, it must `require amount <= <committed
//                                    limit>` where the cap is a committed field
//                                    or role param named `limit` (bare `limit`
//                                    or `prev_states[i].limit`). Models the
//                                    SpendingLimitVault per-tx cap. It does NOT
//                                    prove the cap is itself sound, only that a
//                                    cap require of this shape is present.
//   invariant multisig_threshold;  — a capability refinement: a state-mutating
//                                    release-style transition must authorise via
//                                    >= 2 DISTINCT committed-key checkSig
//                                    operands (distinct committed
//                                    `prev_states[i].<key>` / param / state-field
//                                    pubkeys counted inside checkSig). Models
//                                    ArbiterEscrow 2-of-3 and MultisigTreasury
//                                    2-of-2. It counts distinct committed-key
//                                    operands structurally; it does NOT prove the
//                                    boolean combination is a true k-of-n
//                                    threshold.
//   invariant temporal_guard;      — when declared, a time-gated transition must
//                                    `require now_bucket >= <committed time
//                                    expr>` — a committed deadline field
//                                    (`now_bucket >= deadline`) or a committed
//                                    `last_active + timeout` window
//                                    (`now_bucket >= last_active + timeout`).
//                                    Models HTLC.refund and DeadMansSwitch.claim.
//                                    `now_bucket` is caller-asserted and coarse;
//                                    this is a shape match on the gate require,
//                                    NOT a wall-clock proof.

/// The committed pubkey/field names of a role: its params plus its state fields.
/// Shared by C2 (capability) and D3 (`multisig_threshold`).
fn committed_keys(role: &Role) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for p in &role.params {
        set.insert(p.name.clone());
    }
    for f in &role.state {
        set.insert(f.name.clone());
    }
    set
}

#[allow(clippy::too_many_arguments)]
fn check_c3_refinements(
    role: &Role,
    want_monotonic_seq: bool,
    want_non_negative_amount: bool,
    want_bounded_supply: bool,
    want_spending_cap: bool,
    want_multisig_threshold: bool,
    want_temporal_guard: bool,
    committed: &std::collections::HashSet<String>,
    diags: &mut Vec<Diagnostic>,
) {
    let has_supply_field = role.state.iter().any(|f| f.name == "supply");
    let has_total_field = role.state.iter().any(|f| f.name == "total");
    let has_seq_field = role.state.iter().any(|f| f.name == "seq");
    let has_limit = role.state.iter().any(|f| f.name == "limit")
        || role.params.iter().any(|p| p.name == "limit");
    for entry in &role.entrypoints {
        if !matches!(entry.mode, CovenantMode::Transition) {
            continue;
        }
        if want_monotonic_seq && has_seq_field {
            let mutates = entry
                .body
                .iter()
                .any(|s| matches!(s, Stmt::Return(ReturnExpr::Object { .. })));
            if mutates && !asserts_seq_increment(entry) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `monotonic_seq` violated: state-mutating transition does \
                     not advance `seq` by exactly one (expected `seq: seq + 1` in the return or a \
                     `require <next> == seq + 1`)",
                    role.name, entry.name
                )));
            }
        }
        if want_non_negative_amount {
            let has_amount_arg = entry
                .args
                .iter()
                .any(|a| a.name == "amount" && a.ty == Type::Int);
            if has_amount_arg && !asserts_amount_non_negative(entry) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `non_negative_amount` violated: int `amount` is taken but \
                     never bounded non-negative (expected `require amount >= 0`)",
                    role.name, entry.name
                )));
            }
        }
        if want_bounded_supply && has_supply_field && has_total_field {
            let has_amount_arg = entry
                .args
                .iter()
                .any(|a| a.name == "amount" && a.ty == Type::Int);
            if has_amount_arg && !asserts_supply_within_total(entry) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `bounded_supply` violated: the cumulative draw is not \
                     bounded by the committed ceiling (expected `require supply + amount <= total`)",
                    role.name, entry.name
                )));
            }
        }
        if want_spending_cap {
            let has_amount_arg = entry
                .args
                .iter()
                .any(|a| a.name == "amount" && a.ty == Type::Int);
            if has_amount_arg && has_limit && !asserts_amount_within_limit(entry) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `spending_cap` violated: int `amount` is taken but is not \
                     bounded by a committed cap (expected `require amount <= limit` where `limit` \
                     is a committed state field or role param)",
                    role.name, entry.name
                )));
            }
        }
        if want_multisig_threshold {
            // A state-mutating transition (object return, or a scalar return
            // that broadcasts into a state field — same mutation test C2 uses).
            let state_field_names: Vec<&str> = role.state.iter().map(|f| f.name.as_str()).collect();
            let mutates = entry.body.iter().any(|s| match s {
                Stmt::Return(ReturnExpr::Object { .. }) => true,
                Stmt::Return(ReturnExpr::Scalar(expr)) => {
                    state_field_names.iter().any(|f| references_var(expr, f))
                }
                _ => false,
            });
            if mutates && distinct_committed_checksig_keys(entry, committed) < 2 {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `multisig_threshold` violated: a state-mutating transition \
                     authorizes with fewer than 2 distinct committed-key checkSig operands \
                     (expected at least two distinct committed keys signing, e.g. \
                     `checkSig(a, signer_a) && checkSig(b, signer_b)`)",
                    role.name, entry.name
                )));
            }
        }
        if want_temporal_guard {
            // A time-gated transition is one whose REQUIRE guards read
            // `now_bucket` (the caller-asserted coarse time bucket). Reading
            // `now_bucket` only in the return (e.g. DeadMansSwitch.heartbeat
            // refreshing `last_active: now_bucket`) is a liveness refresh, NOT a
            // gate, so it does not trigger the requirement — only transitions
            // that already use `now_bucket` in a guard are checked.
            let gated_on_now = entry.body.iter().any(|s| match s {
                Stmt::Require(e) => expr_reads_now_bucket(e),
                _ => false,
            });
            if gated_on_now && !asserts_temporal_gate(entry, committed) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `temporal_guard` violated: a time-gated transition reads \
                     `now_bucket` but does not gate on a committed time (expected \
                     `require now_bucket >= <committed deadline>` or \
                     `require now_bucket >= last_active + timeout`)",
                    role.name, entry.name
                )));
            }
        }
    }
}

// ── D4: conservation_split — N-field internal value-flow balance (structural) ─
//
// NARROW + opt-in: fires ONLY when `invariant conservation_split;` is declared.
//
//   invariant conservation_split; — addresses the documented C1 per-field-only
//                                   limit (C1 checks each value-bearing field in
//                                   ISOLATION and CANNOT see that `amount`
//                                   leaving field `f` arrives in field `g`). When
//                                   declared, every state-mutating transition's
//                                   object return must conserve value across ALL
//                                   value-bearing fields it touches: the additive
//                                   delta of every value-bearing field is
//                                   computed (a field `f: f + e` has delta `+e`,
//                                   `f: f - e` has delta `-e`, a bare carry `f: f`
//                                   has delta `0`), and the +deltas and -deltas
//                                   must CANCEL as AST terms — the multiset of all
//                                   added atoms must equal the multiset of all
//                                   subtracted atoms (each `+`-separated summand
//                                   is one atom, matched by `Expr` structural
//                                   equality). The existing 2-field transfer
//                                   (`f: f - x`, `g: g + x`) is the N=2 instance.
//                                   N>2 splits work too: `a: a - (x + y)`,
//                                   `b: b + x`, `c: c + y` balances because the
//                                   subtracted atoms {x, y} equal the added atoms
//                                   {x, y}. Rejected: deltas that do not net to
//                                   zero (value created or destroyed across N
//                                   fields), or a non-additive (multiplicative /
//                                   constant / double-self) mutation on a
//                                   value-bearing field (not analyzable).
//
// HONEST SCOPE: this is STRUCTURAL N-field additive-delta arithmetic — it sums
// the per-field deltas as multisets of `+`-separated AST atoms and requires them
// to cancel by `Expr` structural equality. It proves INTERNAL value conservation
// (value moved BETWEEN fields of the same covenant nets to zero) for transfers
// and splits across N>=2 value-bearing fields. It is NOT a general SMT
// conservation proof: it does not reason about the numeric VALUES of the terms,
// does not reason about conditionals or arbitrary arithmetic identities (it only
// cancels the syntactic `+`-atoms; e.g. it will not see that `x*2` equals
// `x + x`), does not prove `x >= 0` (combine with `non_negative_amount` for
// that), and does not read on-chain coin values. Critically it is for INTERNAL
// transfers/splits ONLY — it does NOT model a SPEND that moves value OUT of the
// covenant to an external output (a single value-bearing field decreasing with
// no in-covenant counter-field). Spend covenants do NOT declare this invariant;
// they use `value_conserved` (single-additive per-field) instead.

/// What a value-bearing field's object-return assignment does, for the
/// `conservation_split` shape match.
enum SplitAdjust<'a> {
    /// `f: f` — bare carry, value unchanged.
    Carry,
    /// `f: f - term` — value-bearing field decreases by `term`.
    Decrease(&'a Expr),
    /// `f: f + term` — value-bearing field increases by `term`.
    Increase(&'a Expr),
    /// Anything else (multiplicative, constant, double-self, foreign) — not a
    /// recognised conserving adjustment.
    Other,
}

/// Value-bearing for the split check: `coin` type, a conventional balance name
/// (`balance`/`amount`/`supply`), or any field whose name ends in `balance`
/// (e.g. `from_balance`, `to_balance`). Broader than C1's `is_value_bearing`
/// only by the `*balance` suffix, so a transfer covenant can name its two legs
/// `from_balance` / `to_balance`.
fn is_value_bearing_split(name: &str, ty: &Type) -> bool {
    is_value_bearing(name, ty) || name.ends_with("balance")
}

/// Classify an object-return assignment `field: value` as a split adjustment.
/// Only the bare-carry and single-additive (`field ± term`) shapes are
/// recognised; `term` must not itself reference `field`.
fn classify_split_adjust<'a>(field: &str, value: &'a Expr) -> SplitAdjust<'a> {
    if matches!(value, Expr::Var(name) if name == field) {
        return SplitAdjust::Carry;
    }
    if let Expr::Binary { op, lhs, rhs } = value {
        let lhs_is_field = matches!(lhs.as_ref(), Expr::Var(name) if name == field);
        let rhs_is_field = matches!(rhs.as_ref(), Expr::Var(name) if name == field);
        match op {
            // `field + term` or `term + field` — an increase by `term`.
            BinOp::Add => {
                if lhs_is_field && !rhs_is_field && !references_var(rhs, field) {
                    return SplitAdjust::Increase(rhs);
                }
                if rhs_is_field && !lhs_is_field && !references_var(lhs, field) {
                    return SplitAdjust::Increase(lhs);
                }
            }
            // `field - term` — a decrease by `term`. (Subtraction is not
            // commutative; only `field - term` is a decrease of `field`.)
            BinOp::Sub if lhs_is_field && !rhs_is_field && !references_var(rhs, field) => {
                return SplitAdjust::Decrease(rhs);
            }
            _ => {}
        }
    }
    SplitAdjust::Other
}

/// Flatten an additive-delta term into its `+`-separated summand atoms. A term
/// `x + y + z` becomes `[x, y, z]`; any non-`+` term (a bare var, a `*` product,
/// a call, …) is a single opaque atom. This is what lets an N-field split's
/// combined term `(x + y)` cancel against the separate `+x` and `+y` legs: both
/// sides flatten to the same multiset of atoms. Subtraction inside a term is
/// NOT decomposed (it would change the sign bookkeeping); a `-` sub-term is left
/// as one opaque atom and only cancels against an identical opaque atom.
fn flatten_add_atoms<'a>(term: &'a Expr, out: &mut Vec<&'a Expr>) {
    if let Expr::Binary {
        op: BinOp::Add,
        lhs,
        rhs,
    } = term
    {
        flatten_add_atoms(lhs, out);
        flatten_add_atoms(rhs, out);
    } else {
        out.push(term);
    }
}

/// D4: under `conservation_split`, every state-mutating transition's object
/// return must conserve value across ALL its value-bearing fields — the additive
/// deltas of every value-bearing field must net to zero, with the added atoms and
/// the subtracted atoms cancelling by `Expr` structural equality. Handles N>=2
/// value-bearing fields (the paired two-field transfer is the N=2 instance).
/// STRUCTURAL N-field additive-delta arithmetic for INTERNAL flows, NOT an SMT
/// conservation proof and NOT a model of value-out spends (see the D4 note).
fn check_conservation_split(role: &Role, diags: &mut Vec<Diagnostic>) {
    for entry in &role.entrypoints {
        if !matches!(entry.mode, CovenantMode::Transition) {
            continue;
        }
        if is_mint_or_burn(&entry.name) {
            continue; // mint/burn is an authorised supply change, exempt (as in C1)
        }
        for stmt in &entry.body {
            let Stmt::Return(ReturnExpr::Object { fields, .. }) = stmt else {
                continue;
            };
            // Per-field deltas across every value-bearing field in the return.
            let mut increase_terms: Vec<&Expr> = Vec::new();
            let mut decrease_terms: Vec<&Expr> = Vec::new();
            let mut others: Vec<&str> = Vec::new();
            let mut moved_fields = 0usize;
            for (field, value) in fields {
                // Only value-bearing fields participate in the conservation
                // shape; non-value fields (keys, ids, periods) are ignored.
                let Some(f) = role.state.iter().find(|f| &f.name == field) else {
                    continue;
                };
                if !is_value_bearing_split(field, &f.ty) {
                    continue;
                }
                match classify_split_adjust(field, value) {
                    SplitAdjust::Carry => {}
                    SplitAdjust::Decrease(term) => {
                        decrease_terms.push(term);
                        moved_fields += 1;
                    }
                    SplitAdjust::Increase(term) => {
                        increase_terms.push(term);
                        moved_fields += 1;
                    }
                    SplitAdjust::Other => others.push(field),
                }
            }
            // No value-bearing field moved → nothing to conserve here.
            if moved_fields == 0 && others.is_empty() {
                continue;
            }
            // A value-bearing field changed in a non-additive way (constant,
            // multiplicative, double-self): not analyzable as an additive delta.
            if let Some(bad) = others.first() {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `conservation_split` violated: value-bearing field `{}` \
                     changes in a non-additive shape (expected each value-bearing field to carry \
                     `f: f`, increase `f: f + e`, or decrease `f: f - e`; multiplicative, constant, \
                     or double-self forms are not analyzable as a value delta)",
                    role.name, entry.name, bad
                )));
                continue;
            }
            // Net-zero requirement: the value moved must stay INSIDE the covenant
            // — at least one field decreases AND at least one field increases. A
            // lone decrease (drain to an external output) or lone increase (mint)
            // is NOT an internal split; that is a `value_conserved` spend shape,
            // not a `conservation_split` transfer.
            if increase_terms.is_empty() || decrease_terms.is_empty() {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `conservation_split` violated: an internal split must move \
                     value BETWEEN value-bearing fields — found {} field(s) increasing and {} \
                     decreasing (a lone increase mints value; a lone decrease drains it with no \
                     matching counter-field). Use `value_conserved` for a value-out spend.",
                    role.name,
                    entry.name,
                    increase_terms.len(),
                    decrease_terms.len()
                )));
                continue;
            }
            // Flatten every delta term into its `+`-atoms, then require the added
            // multiset to equal the subtracted multiset (structural cancellation).
            let mut plus_atoms: Vec<&Expr> = Vec::new();
            for t in &increase_terms {
                flatten_add_atoms(t, &mut plus_atoms);
            }
            let mut minus_atoms: Vec<&Expr> = Vec::new();
            for t in &decrease_terms {
                flatten_add_atoms(t, &mut minus_atoms);
            }
            // Multiset-difference: remove each plus atom from the minus pool by
            // the FIRST structurally-equal match. Anything left on either side
            // means the deltas do not cancel (value created or destroyed).
            let mut remaining_minus: Vec<&Expr> = minus_atoms.clone();
            let mut unmatched_plus: Vec<&Expr> = Vec::new();
            for p in &plus_atoms {
                if let Some(pos) = remaining_minus.iter().position(|m| *m == *p) {
                    remaining_minus.remove(pos);
                } else {
                    unmatched_plus.push(p);
                }
            }
            if !unmatched_plus.is_empty() || !remaining_minus.is_empty() {
                let added: Vec<String> = plus_atoms.iter().map(|e| e.to_silverscript()).collect();
                let subtracted: Vec<String> =
                    minus_atoms.iter().map(|e| e.to_silverscript()).collect();
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `conservation_split` violated: the value added across \
                     fields ({{{}}}) does not cancel the value subtracted ({{{}}}) — the deltas \
                     must net to zero (the same terms moved out of some fields and into others)",
                    role.name,
                    entry.name,
                    added.join(", "),
                    subtracted.join(", "),
                )));
            }
        }
    }
}

/// True if the entrypoint bounds `amount` by a committed cap: a require of the
/// form `amount <= <cap>` (or `<cap> >= amount`) where `<cap>` is a bare
/// committed name `limit` or a `prev_states[i].limit` access. Structural shape
/// match — not a solver.
fn asserts_amount_within_limit(entry: &portrait_syntax::Entry) -> bool {
    let is_amount = |e: &Expr| matches!(e, Expr::Var(n) if n == "amount");
    let is_committed_limit = |e: &Expr| -> bool {
        match e {
            // bare committed field / param named `limit`
            Expr::Var(n) => n == "limit",
            // prev_states[i].limit
            Expr::Field { base, field } => {
                field == "limit"
                    && matches!(
                        base.as_ref(),
                        Expr::Index { base: arr, .. }
                            if matches!(arr.as_ref(), Expr::Var(n) if n == "prev_states")
                    )
            }
            _ => false,
        }
    };
    for stmt in &entry.body {
        if let Stmt::Require(Expr::Binary { op, lhs, rhs }) = stmt {
            match op {
                // amount <= limit
                BinOp::Le if is_amount(lhs) && is_committed_limit(rhs) => return true,
                // limit >= amount
                BinOp::Ge if is_committed_limit(lhs) && is_amount(rhs) => return true,
                _ => {}
            }
        }
    }
    false
}

/// Count the DISTINCT committed-key pubkey operands that appear inside `checkSig`
/// calls across the entrypoint's `require` statements. A committed key is a bare
/// committed name (param / state field) or a `prev_states[i].field` access; two
/// `prev_states[i].field` operands are counted distinct by their field name.
/// Structural count — it does not prove the boolean combination is a true
/// k-of-n threshold, only that >= k distinct committed keys are signed.
fn distinct_committed_checksig_keys(
    entry: &portrait_syntax::Entry,
    committed: &std::collections::HashSet<String>,
) -> usize {
    let mut keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for stmt in &entry.body {
        if let Stmt::Require(e) = stmt {
            for pk in collect_checksig_pubkeys(e) {
                match pk {
                    Expr::Var(n) if committed.contains(n) => {
                        keys.insert(n.clone());
                    }
                    Expr::Field { base, field }
                        if matches!(
                            base.as_ref(),
                            Expr::Index { base: arr, .. }
                                if matches!(arr.as_ref(), Expr::Var(n) if n == "prev_states")
                        ) =>
                    {
                        keys.insert(format!("prev_states.{field}"));
                    }
                    _ => {}
                }
            }
        }
    }
    keys.len()
}

/// True if the expression reads the caller-asserted coarse time bucket
/// `now_bucket`.
fn expr_reads_now_bucket(e: &Expr) -> bool {
    match e {
        Expr::Var(n) => n == "now_bucket",
        Expr::Unary { rhs, .. } => expr_reads_now_bucket(rhs),
        Expr::Binary { lhs, rhs, .. } => expr_reads_now_bucket(lhs) || expr_reads_now_bucket(rhs),
        Expr::Field { base, .. } => expr_reads_now_bucket(base),
        Expr::Index { base, index } => expr_reads_now_bucket(base) || expr_reads_now_bucket(index),
        Expr::Call { args, .. } => args.iter().any(expr_reads_now_bucket),
        Expr::Int(_) | Expr::Bool(_) | Expr::Bytes(_) => false,
    }
}

/// True if the entrypoint gates on a committed time: a require of the form
/// `now_bucket >= <committed time expr>`, where the RHS is a committed deadline
/// field (a bare committed name, e.g. `deadline`, or `prev_states[i].deadline`)
/// or a committed window sum `last_active + timeout` (committed names, either
/// operand order). Structural shape match — not a wall-clock proof.
fn asserts_temporal_gate(
    entry: &portrait_syntax::Entry,
    committed: &std::collections::HashSet<String>,
) -> bool {
    let is_now_bucket = |e: &Expr| matches!(e, Expr::Var(n) if n == "now_bucket");
    // A committed deadline-like atom: a bare COMMITTED name (param / state
    // field), or a `prev_states[i].<name>` access. A caller-supplied arg is NOT
    // committed and does not count.
    let is_committed_time_atom = |e: &Expr| -> bool {
        match e {
            Expr::Var(n) => committed.contains(n),
            Expr::Field { base, .. } => matches!(
                base.as_ref(),
                Expr::Index { base: arr, .. }
                    if matches!(arr.as_ref(), Expr::Var(n) if n == "prev_states")
            ),
            _ => false,
        }
    };
    // A committed time expression: a single committed atom, or a sum of two
    // committed atoms (the `last_active + timeout` window form).
    let is_committed_time = |e: &Expr| -> bool {
        if is_committed_time_atom(e) {
            return true;
        }
        if let Expr::Binary {
            op: BinOp::Add,
            lhs,
            rhs,
        } = e
        {
            return is_committed_time_atom(lhs) && is_committed_time_atom(rhs);
        }
        false
    };
    for stmt in &entry.body {
        if let Stmt::Require(Expr::Binary { op, lhs, rhs }) = stmt {
            match op {
                // now_bucket >= <committed time>
                BinOp::Ge if is_now_bucket(lhs) && is_committed_time(rhs) => return true,
                // <committed time> <= now_bucket
                BinOp::Le if is_committed_time(lhs) && is_now_bucket(rhs) => return true,
                _ => {}
            }
        }
    }
    false
}

/// True if the entrypoint bounds the running accumulator within the committed
/// ceiling: a require of the form `supply + amount <= total` (with the `+`
/// operands in either order). Structural pattern match — not a solver.
fn asserts_supply_within_total(entry: &portrait_syntax::Entry) -> bool {
    // `supply + amount` / `amount + supply`.
    let is_supply_plus_amount = |e: &Expr| -> bool {
        if let Expr::Binary {
            op: BinOp::Add,
            lhs,
            rhs,
        } = e
        {
            let is_supply = |x: &Expr| matches!(x, Expr::Var(n) if n == "supply");
            let is_amount = |x: &Expr| matches!(x, Expr::Var(n) if n == "amount");
            return (is_supply(lhs) && is_amount(rhs)) || (is_amount(lhs) && is_supply(rhs));
        }
        false
    };
    let is_total = |e: &Expr| matches!(e, Expr::Var(n) if n == "total");
    for stmt in &entry.body {
        if let Stmt::Require(Expr::Binary { op, lhs, rhs }) = stmt {
            match op {
                // supply + amount <= total
                BinOp::Le if is_supply_plus_amount(lhs) && is_total(rhs) => return true,
                // total >= supply + amount
                BinOp::Ge if is_total(lhs) && is_supply_plus_amount(rhs) => return true,
                _ => {}
            }
        }
    }
    false
}

/// True if the entrypoint advances `seq` by exactly one: either the object
/// return assigns `seq: seq + 1`, or a require asserts `<x> == seq + 1`.
fn asserts_seq_increment(entry: &portrait_syntax::Entry) -> bool {
    let is_seq_plus_one = |e: &Expr| -> bool {
        matches!(
            e,
            Expr::Binary { op: BinOp::Add, lhs, rhs }
                if matches!(lhs.as_ref(), Expr::Var(n) if n == "seq")
                    && matches!(rhs.as_ref(), Expr::Int(1))
        )
    };
    for stmt in &entry.body {
        match stmt {
            Stmt::Return(ReturnExpr::Object { fields, .. }) => {
                if fields.iter().any(|(f, v)| f == "seq" && is_seq_plus_one(v)) {
                    return true;
                }
            }
            Stmt::Require(Expr::Binary {
                op: BinOp::Eq,
                lhs,
                rhs,
            }) if is_seq_plus_one(lhs) || is_seq_plus_one(rhs) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// True if the entrypoint bounds `amount` non-negative: a require of the form
/// `amount >= <int>=0` or `amount > <int>=-1`.
fn asserts_amount_non_negative(entry: &portrait_syntax::Entry) -> bool {
    for stmt in &entry.body {
        if let Stmt::Require(Expr::Binary { op, lhs, rhs }) = stmt {
            let lhs_is_amount = matches!(lhs.as_ref(), Expr::Var(n) if n == "amount");
            if !lhs_is_amount {
                continue;
            }
            match (op, rhs.as_ref()) {
                (BinOp::Ge, Expr::Int(n)) if *n >= 0 => return true,
                (BinOp::Gt, Expr::Int(n)) if *n >= -1 => return true,
                _ => {}
            }
        }
    }
    false
}

/// Infer the type of an expression in the given environment, or return a
/// human-readable rejection message.
fn type_of(expr: &Expr, env: &TyEnv) -> Result<Ty, String> {
    match expr {
        Expr::Int(_) => Ok(Ty::int()),
        Expr::Bool(_) => Ok(Ty::bool()),
        Expr::Bytes(_) => Ok(Ty::Surface(Type::Bytes32)),
        Expr::Var(name) => env
            .vars
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown variable `{name}`")),
        Expr::Field { base, field } => {
            let base_ty = type_of(base, env)?;
            match base_ty {
                // prev_states[i].field — resolve against the role's state fields.
                Ty::State => env
                    .state_fields
                    .get(field)
                    .cloned()
                    .map(Ty::Surface)
                    .ok_or_else(|| format!("unknown state field `{field}` on a prior state")),
                other => Err(format!(
                    "field access `.{field}` on a value of type {} (only prior states have fields)",
                    other.display()
                )),
            }
        }
        Expr::Index { base, index } => {
            let base_ty = type_of(base, env)?;
            let idx_ty = type_of(index, env)?;
            if idx_ty != Ty::int() {
                return Err(format!("index must be int, found {}", idx_ty.display()));
            }
            match base_ty {
                // prev_states[i] : State
                Ty::StateArray => Ok(Ty::State),
                other => Err(format!(
                    "cannot index a value of type {} (only `prev_states` is indexable)",
                    other.display()
                )),
            }
        }
        Expr::Unary { op, rhs } => {
            let rhs_ty = type_of(rhs, env)?;
            match op {
                UnOp::Neg => {
                    if rhs_ty == Ty::int() {
                        Ok(Ty::int())
                    } else {
                        Err(format!(
                            "unary `-` operand must be int, found {}",
                            rhs_ty.display()
                        ))
                    }
                }
                UnOp::Not => {
                    if rhs_ty == Ty::bool() {
                        Ok(Ty::bool())
                    } else {
                        Err(format!(
                            "unary `!` operand must be bool, found {}",
                            rhs_ty.display()
                        ))
                    }
                }
            }
        }
        Expr::Binary { op, lhs, rhs } => {
            let lt = type_of(lhs, env)?;
            let rt = type_of(rhs, env)?;
            match op {
                // Arithmetic: int × int -> int.
                BinOp::Add | BinOp::Sub | BinOp::Mul => {
                    if lt == Ty::int() && rt == Ty::int() {
                        Ok(Ty::int())
                    } else {
                        Err(format!(
                            "arithmetic `{}` requires int operands, found {} and {}",
                            op.as_str(),
                            lt.display(),
                            rt.display()
                        ))
                    }
                }
                // Comparison: T × T -> bool (operands must match). Red-team LOW
                // (a): comparison operands must be scalar *surface* types — a
                // bare `State` / `State[]` operand (e.g. `prev_states == ...` or
                // `prev_states[0] == ...`) is rejected fail-closed, since record
                // / array equality has no covenant lowering and would silently
                // type-launder a meaningless guard.
                BinOp::Eq | BinOp::Ne | BinOp::Ge | BinOp::Le | BinOp::Gt | BinOp::Lt => {
                    if !lt.is_scalar_surface() || !rt.is_scalar_surface() {
                        return Err(format!(
                            "comparison `{}` requires scalar operands (int/bool/bytes32/pubkey/\
                             sig/coin), found {} and {}",
                            op.as_str(),
                            lt.display(),
                            rt.display()
                        ));
                    }
                    if lt == rt {
                        Ok(Ty::bool())
                    } else {
                        Err(format!(
                            "comparison `{}` requires operands of the same type, found {} and {}",
                            op.as_str(),
                            lt.display(),
                            rt.display()
                        ))
                    }
                }
                // Logical: bool && bool / bool || bool -> bool.
                BinOp::And | BinOp::Or => {
                    if lt == Ty::bool() && rt == Ty::bool() {
                        Ok(Ty::bool())
                    } else {
                        Err(format!(
                            "logical `{}` requires bool operands, found {} and {}",
                            op.as_str(),
                            lt.display(),
                            rt.display()
                        ))
                    }
                }
            }
        }
        Expr::Call { name, args } => type_of_call(name, args, env),
    }
}

/// Type the recognised builtin calls. Unknown builtins are rejected (they would
/// otherwise type-launder anything); the recognised set mirrors the engine
/// intrinsics the emitter lowers verbatim.
fn type_of_call(name: &str, args: &[Expr], env: &TyEnv) -> Result<Ty, String> {
    // Type all arguments first so arg-internal errors surface.
    let arg_tys = args
        .iter()
        .map(|a| type_of(a, env))
        .collect::<Result<Vec<_>, _>>()?;
    match name {
        // checkSig(sig, pubkey) -> bool
        "checkSig" => {
            if arg_tys.len() != 2 {
                return Err(format!(
                    "checkSig expects 2 arguments (sig, pubkey), found {}",
                    arg_tys.len()
                ));
            }
            if arg_tys[0] != Ty::Surface(Type::Sig) {
                return Err(format!(
                    "checkSig: first argument must be sig, found {}",
                    arg_tys[0].display()
                ));
            }
            if arg_tys[1] != Ty::Surface(Type::PubKey) {
                return Err(format!(
                    "checkSig: second argument must be pubkey, found {}",
                    arg_tys[1].display()
                ));
            }
            Ok(Ty::bool())
        }
        // OpInputCovenantId(int) -> bytes32
        "OpInputCovenantId" => {
            if arg_tys.len() != 1 {
                return Err(format!(
                    "OpInputCovenantId expects 1 argument (int index), found {}",
                    arg_tys.len()
                ));
            }
            if arg_tys[0] != Ty::int() {
                return Err(format!(
                    "OpInputCovenantId: argument must be int, found {}",
                    arg_tys[0].display()
                ));
            }
            Ok(Ty::Surface(Type::Bytes32))
        }
        // blake2b(bytes32) -> bytes32
        //
        // The engine hashing intrinsic (OpBlake2b, 0xaa) that silverc lowers
        // verbatim. The only surface byte type Portrait carries is `bytes32`, so
        // the honest signature is a single `bytes32` preimage → `bytes32` digest
        // (silverc itself types `blake2b(_)` as `byte[32]`). One argument only;
        // arity / type misuse is rejected fail-closed.
        "blake2b" => {
            if arg_tys.len() != 1 {
                return Err(format!(
                    "blake2b expects 1 argument (bytes32 preimage), found {}",
                    arg_tys.len()
                ));
            }
            if arg_tys[0] != Ty::Surface(Type::Bytes32) {
                return Err(format!(
                    "blake2b: argument must be bytes32, found {}",
                    arg_tys[0].display()
                ));
            }
            Ok(Ty::Surface(Type::Bytes32))
        }
        other => Err(format!("call to unknown function `{other}`")),
    }
}

fn find_role<'a>(roles: &'a [Role], name: &str) -> Option<&'a Role> {
    roles.iter().find(|r| r.name == name)
}

fn find_entry<'a>(role: &'a Role, name: &str) -> Option<&'a portrait_syntax::Entry> {
    role.entrypoints.iter().find(|e| e.name == name)
}

fn has_return(body: &[Stmt]) -> bool {
    body.iter().any(|s| matches!(s, Stmt::Return(_)))
}

fn is_value_conserved(inv: &Invariant) -> bool {
    matches!(inv, Invariant::ValueConserved)
}

fn is_no_undeclared_state(inv: &Invariant) -> bool {
    matches!(inv, Invariant::NoUndeclaredState)
}

/// Recursively check a flow (and its nested Choose/Par/Repeat sub-flows) for
/// `Step::Move`s that reference an unknown role or entrypoint.
fn check_flow(flow: &Flow, roles: &[Role], diags: &mut Vec<Diagnostic>) {
    for step in &flow.steps {
        match step {
            Step::Move { role, entry } => match find_role(roles, role) {
                None => diags.push(Diagnostic::new(format!(
                    "flow step references unknown role `{}`",
                    role
                ))),
                Some(r) => {
                    if find_entry(r, entry).is_none() {
                        diags.push(Diagnostic::new(format!(
                            "flow step references unknown entrypoint `{}.{}`",
                            role, entry
                        )));
                    }
                }
            },
            Step::Choose(flows) | Step::Par(flows) => {
                for f in flows {
                    check_flow(f, roles, diags);
                }
            }
            Step::Repeat(_, f) => check_flow(f, roles, diags),
        }
    }
}

// ── Allocation advisor (read-only) ───────────────────────────────────────────
//
// HONEST SCOPE: this is an ADVISOR / checker, NOT a full automatic allocator and
// NOT a vProg synthesizer. It does NOT move code between layers, does NOT decide
// the layer (that is still attribute-driven: `#[covenant]` → covenant, no
// attribute → vProg), and does NOT parse loop/mapping semantics. It only INSPECTS
// the already-allocated entrypoints and emits per-entrypoint routing notes,
// reusing the syntax crate's single-source-of-truth `REJECTION_SET` so the
// advice and the parser's rejection logic cannot drift.
//
// Two signals:
//   * COVENANT entrypoint (Transition/Verification) carrying a `Stmt::Raw` hole
//     that *names* a REJECTION_SET construct → FLAG: it is marked covenant but
//     uses a construct that cannot be a covenant; route it to the vProgs layer.
//     (Standalone rejection-set constructs are already loud-rejected at parse for
//     covenant modes, so this fires for the residual embedded/holey forms — it is
//     a defensive cross-check, not the primary gate.)
//   * NonCovenant (vProg) entrypoint whose body is fully covenant-legal (only
//     typed Require/Return, no Raw hole) → NOTE: it looks covenant-suitable, so
//     the author could promote it by adding a `#[covenant(...)]` attribute.

/// One per-entrypoint allocation advisory. Read-only; carries no side effects.
#[derive(Debug, Clone, PartialEq)]
pub struct Advisory {
    pub role: String,
    pub entry: String,
    /// The layer this entrypoint is allocated to today ("Covenant" / "VProg"),
    /// derived from its `CovenantMode`.
    pub layer: &'static str,
    /// Human-readable routing note.
    pub message: String,
}

/// The layer label an entrypoint mode allocates to (matches Pounce's mapping for
/// the attribute-driven decision: attribute present → Covenant, absent → VProg).
fn layer_label(mode: &CovenantMode) -> &'static str {
    match mode {
        CovenantMode::Transition | CovenantMode::Verification => "Covenant",
        CovenantMode::NonCovenant => "VProg",
    }
}

/// If a `Stmt::Raw` hole's text names a REJECTION_SET construct, return that
/// construct's lead. Reuses the syntax crate's table so prose and code cannot
/// drift. Statement-head match (leading word) or a `.<lead>(` method-call shape.
fn raw_names_rejected_construct(text: &str) -> Option<&'static str> {
    let trimmed = text.trim_start();
    let head = trimmed
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("");
    for rc in portrait_syntax::REJECTION_SET {
        if !rc.as_method_call && rc.lead == head {
            return Some(rc.lead);
        }
        if rc.as_method_call && text.contains(&format!(".{}(", rc.lead)) {
            return Some(rc.lead);
        }
    }
    None
}

/// Run the read-only allocation advisor over a program. Returns one advisory per
/// entrypoint that warrants a routing note (mismatches first, suitability notes
/// second). An empty result means every entrypoint sits cleanly on its layer.
pub fn advise(program: &Program) -> Vec<Advisory> {
    let mut out = Vec::new();
    for role in &program.app.roles {
        for entry in &role.entrypoints {
            let is_covenant = !matches!(entry.mode, CovenantMode::NonCovenant);
            // Find any Raw hole and whether it names a rejection-set construct.
            let rejected_in_body = entry.body.iter().find_map(|s| match s {
                Stmt::Raw(text) => raw_names_rejected_construct(text).map(|lead| (lead, text)),
                _ => None,
            });
            let has_any_raw = entry.body.iter().any(|s| matches!(s, Stmt::Raw(_)));

            if is_covenant {
                if let Some((lead, _text)) = rejected_in_body {
                    out.push(Advisory {
                        role: role.name.clone(),
                        entry: entry.name.clone(),
                        layer: layer_label(&entry.mode),
                        message: format!(
                            "marked covenant but uses `{lead}`, which cannot be a covenant \
                             construct — route this entrypoint to the vProgs (Tier-3) layer by \
                             removing its `#[covenant(...)]` attribute"
                        ),
                    });
                }
            } else {
                // vProg entrypoint. If it holds a rejection-set construct, that is
                // the expected, correct placement — note it as confirmation. If it
                // is fully covenant-legal (no Raw at all), note it as promotable.
                if let Some((lead, _text)) = rejected_in_body {
                    out.push(Advisory {
                        role: role.name.clone(),
                        entry: entry.name.clone(),
                        layer: layer_label(&entry.mode),
                        message: format!(
                            "vProg entrypoint uses `{lead}` (a construct that cannot be a \
                             covenant) — correctly allocated to the vProgs layer"
                        ),
                    });
                } else if !has_any_raw && !entry.body.is_empty() {
                    out.push(Advisory {
                        role: role.name.clone(),
                        entry: entry.name.clone(),
                        layer: layer_label(&entry.mode),
                        message: "vProg entrypoint is fully covenant-legal (only typed \
                             require/return) — it could be promoted to a covenant by adding a \
                             `#[covenant(mode = ...)]` attribute"
                            .to_string(),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use portrait_syntax::parse;

    // ---- ACCEPT cases -----------------------------------------------------

    #[test]
    fn accepts_counter_program() {
        let src = include_str!("../../../../examples/counter.portrait");
        let program = parse(src).expect("counter.portrait should parse");
        assert!(
            check(&program).is_ok(),
            "counter program should pass structural checks: {:?}",
            check(&program).err()
        );
    }

    #[test]
    fn accepts_compliance_token_program() {
        let src = include_str!("../../../../examples/tier3-demo/ComplianceToken.portrait");
        let program = parse(src).expect("ComplianceToken.portrait should parse");
        assert!(
            check(&program).is_ok(),
            "ComplianceToken program should pass structural checks: {:?}",
            check(&program).err()
        );
    }

    /// Every shipped `.portrait` source that parses today must still pass the
    /// full checker (structural + the new B3 expression typing) — no false
    /// rejects. This exercises the harder typed expressions: `checkSig(sig,
    /// pubkey)`, `OpInputCovenantId(int) == bytes32`, multi-field object returns,
    /// and `+`/comparison/`<=` precedence over int operands.
    #[test]
    fn accepts_all_shipped_round_trip_sources() {
        let cases: &[(&str, &str)] = &[
            (
                "counter",
                include_str!("../../../../examples/counter.portrait"),
            ),
            (
                "ComplianceToken",
                include_str!("../../../../examples/tier3-demo/ComplianceToken.portrait"),
            ),
            (
                "EvidenceLineage",
                include_str!("../../../../library/attestation/EvidenceLineage.portrait"),
            ),
            (
                "DigitalReit",
                include_str!("../../../../library/finance/reit/DigitalReit.portrait"),
            ),
            (
                "TimeVault",
                include_str!("../../../../library/custody/time-vault/TimeVault.portrait"),
            ),
            (
                "SimpleToken",
                include_str!("../../../../examples/engraver-demo/SimpleToken.portrait"),
            ),
            (
                "PausableToken",
                include_str!("../../../../examples/engraver-demo/PausableToken.portrait"),
            ),
            (
                "VestingWallet",
                include_str!("../../../../examples/engraver-demo/VestingWallet.portrait"),
            ),
            (
                "CsciInstrument",
                include_str!("../../../../library/state/CsciInstrument.portrait"),
            ),
        ];
        for (label, src) in cases {
            let program = parse(src).unwrap_or_else(|e| panic!("[{label}] should parse: {e}"));
            let result = check(&program);
            assert!(
                result.is_ok(),
                "[{label}] should pass the full checker, but was rejected: {:?}",
                result
                    .err()
                    .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
            );
        }
    }

    // ---- REJECT cases -----------------------------------------------------

    /// Helper: parse, run check, expect Err, and assert at least one diagnostic
    /// message contains the given substring.
    fn assert_rejects_with(src: &str, needle: &str) {
        let program = parse(src).expect("source should parse (the *check* must reject, not parse)");
        let diags = check(&program).expect_err("check should reject this program");
        assert!(
            diags.iter().any(|d| d.message.contains(needle)),
            "expected a diagnostic containing `{}`, got: {:?}",
            needle,
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_unknown_via_entry() {
        // Lifecycle names an entrypoint `bumpity` that does not exist on `counter`.
        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> live via counter.bumpity; }
}
"#;
        assert_rejects_with(src, "unknown entrypoint `counter.bumpity`");
    }

    #[test]
    fn rejects_unknown_via_role() {
        // Lifecycle names a role `ghost` that does not exist.
        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> live via ghost.bump; }
}
"#;
        assert_rejects_with(src, "unknown role `ghost`");
    }

    #[test]
    fn rejects_transition_missing_return() {
        // `bump` is a transition reached by a non-terminal edge but has no return.
        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      require delta > 0;
    }
  }
  lifecycle { live -> live via counter.bump; }
}
"#;
        assert_rejects_with(src, "has no return statement");
    }

    #[test]
    fn rejects_verification_with_return() {
        // `attest` is a verification entrypoint but returns a value.
        let src = r#"
pragma portrait ^0.1.0;
app Attestor {
  role attestor {
    param int start;
    state { int value; }
    #[covenant(mode = verification)]
    entrypoint function attest(int proof) : (int value) {
      return proof;
    }
  }
  lifecycle { live -> live via attestor.attest; }
}
"#;
        assert_rejects_with(src, "must not return a value");
    }

    #[test]
    fn rejects_value_conserved_with_dropping_transition() {
        // value_conserved is declared, but the reachable transition drops state
        // (no return). The edge is marked terminal so rule 3 does NOT fire — only
        // rule 4 (value_conserved) should reject this, isolating that check.
        let src = r#"
pragma portrait ^0.1.0;
app Drainer {
  role vault {
    param int start;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function drain(int amount) : (int balance) {
      require amount > 0;
    }
  }
  lifecycle { live -> closed via vault.drain terminal; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "invariant `value_conserved` violated");
    }

    #[test]
    fn rejects_dangling_no_undeclared_state() {
        // `closed` is entered by a non-terminal edge but is never a source state
        // nor a terminal — a dangling state under no_undeclared_state.
        let src = r#"
pragma portrait ^0.1.0;
app Machine {
  role m {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function step(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> closed via m.step; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "invariant `no_undeclared_state` violated");
    }

    #[test]
    fn rejects_unknown_flow_step() {
        // Flow references an entrypoint that does not exist.
        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> live via counter.bump; }
  flow { counter.nonexistent }
}
"#;
        assert_rejects_with(
            src,
            "flow step references unknown entrypoint `counter.nonexistent`",
        );
    }

    // ---- B3 EXPRESSION-TYPING REJECT VECTORS ------------------------------
    //
    // Each of these parses cleanly (no Raw fallback) but is ill-typed; the new
    // expression pass must reject it. Helper `assert_rejects_with` asserts the
    // program parses and that `check` returns a diagnostic containing `needle`.

    /// int + bool — arithmetic on a non-int operand.
    #[test]
    fn rejects_int_plus_bool() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + true;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "arithmetic `+` requires int operands");
    }

    /// require(<int>) — a require whose operand is an int, not bool.
    #[test]
    fn rejects_require_non_bool() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      require delta + 1;
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "require(...) operand must be bool");
    }

    /// return field type mismatch — assigning a bool-typed expr to an int field.
    #[test]
    fn rejects_return_field_type_mismatch() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return A { value: delta > 0 };
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "return field `value` has type");
    }

    /// unknown variable — a bare identifier not in params/state/args.
    #[test]
    fn rejects_unknown_variable() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + nonexistent;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "unknown variable `nonexistent`");
    }

    /// unknown return field — object return assigning a field not in `state`.
    #[test]
    fn rejects_unknown_return_field() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return A { ghost: value + delta };
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "return assigns unknown state field `ghost`");
    }

    /// comparison across mismatched types — int vs bool in `==`.
    #[test]
    fn rejects_comparison_type_mismatch() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      require delta == true;
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "comparison `==` requires operands of the same type");
    }

    /// checkSig with swapped argument types (pubkey, sig instead of sig, pubkey).
    #[test]
    fn rejects_checksig_wrong_arg_types() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig    s;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      require checkSig(owner, s);
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "checkSig: first argument must be sig");
    }

    /// call to an unknown builtin function must be rejected, not type-laundered.
    #[test]
    fn rejects_unknown_call() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      require mystery(delta) >= 0;
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "call to unknown function `mystery`");
    }

    /// blake2b(bytes32) -> bytes32: a covenant that hashes a committed-vs-supplied
    /// preimage and gates the spend on the digest matching a committed hashlock
    /// must type-check (the digest is bytes32, comparable to the committed
    /// bytes32 hashlock).
    #[test]
    fn accepts_blake2b_hashlock() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param bytes32 hashlock;
    state { pubkey owner; bytes32 hashlock; int settled; }
    #[covenant(mode = transition)]
    entrypoint function claim(sig auth, bytes32 preimage) : (pubkey owner, bytes32 hashlock, int settled) {
      require checkSig(auth, owner);
      require blake2b(preimage) == hashlock;
      require settled == 0;
      return A { owner: owner, hashlock: hashlock, settled: 1 };
    }
  }
  lifecycle { live -> live via r.claim; }
}
"#;
        assert_accepts(src);
    }

    /// blake2b arity misuse (two args) is rejected fail-closed.
    #[test]
    fn rejects_blake2b_wrong_arity() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param bytes32 hashlock;
    state { bytes32 hashlock; }
    #[covenant(mode = transition)]
    entrypoint function claim(bytes32 a, bytes32 b) : (bytes32 hashlock) {
      require blake2b(a, b) == hashlock;
      return A { hashlock: hashlock };
    }
  }
  lifecycle { live -> live via r.claim; }
}
"#;
        assert_rejects_with(src, "blake2b expects 1 argument");
    }

    /// blake2b type misuse (int argument instead of bytes32) is rejected
    /// fail-closed.
    #[test]
    fn rejects_blake2b_wrong_arg_type() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param bytes32 hashlock;
    state { bytes32 hashlock; }
    #[covenant(mode = transition)]
    entrypoint function claim(int n) : (bytes32 hashlock) {
      require blake2b(n) == hashlock;
      return A { hashlock: hashlock };
    }
  }
  lifecycle { live -> live via r.claim; }
}
"#;
        assert_rejects_with(src, "blake2b: argument must be bytes32");
    }

    /// A `Stmt::Raw` body (parser fallback) is an untyped hole. The emitter only
    /// consumes `Require`/`Return`, so a Raw surviving to a COVENANT-role
    /// entrypoint would be silently dropped — a FALSE ACCEPT. The robust
    /// fail-CLOSED guard (adversarial-verify follow-up) rejects it here, naming
    /// the statement and routing it to the vProgs layer. (This intentionally
    /// supersedes the old "Raw is skipped, not rejected" contract, which
    /// *documented the latent bug*: the typing pass must not crash on Raw — it no
    /// longer does, it fails-closed.)
    #[test]
    fn raw_body_in_covenant_role_is_fail_closed() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      requires delta @ 1;
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        let program = parse(src).expect("program with a raw require still parses");
        // Confirm the require really did fall back to Raw (otherwise this test
        // would not be exercising the fail-closed path).
        let body = &program.app.roles[0].entrypoints[0].body;
        assert!(
            body.iter().any(|s| matches!(s, Stmt::Raw(_))),
            "expected a Raw fallback in the body, got {body:?}"
        );
        // The typed checker must now REJECT it fail-closed: an untyped statement
        // cannot be projected to a covenant.
        let err = check(&program).expect_err("Raw in a covenant role must fail closed");
        let joined = err
            .into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
            .join("; ");
        assert!(
            joined.contains("cannot be projected to a covenant") && joined.contains("vProgs"),
            "fail-closed diagnostic must name the covenant-projection failure + vProgs route, \
             got: {joined}"
        );
    }

    /// Counterpart to the covenant-role guard: a `Stmt::Raw` in a NON-covenant
    /// (vProgs / Tier-3) entrypoint is NOT projected to a `.sil` covenant here
    /// (Atelier owns it), so it is left as a recorded hole rather than a hard
    /// error — the fail-closed guard is scoped to covenant roles only.
    #[test]
    fn raw_body_in_noncovenant_role_is_tolerated() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    entrypoint function audit(int delta) {
      requires delta @ 1;
    }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        let program = parse(src).expect("program parses");
        // The non-covenant `audit` body fell back to Raw...
        let audit_body = &program.app.roles[0].entrypoints[0].body;
        assert!(
            audit_body.iter().any(|s| matches!(s, Stmt::Raw(_))),
            "expected a Raw fallback in the non-covenant body, got {audit_body:?}"
        );
        // ...and sema tolerates it (not projected to a covenant).
        assert!(
            check(&program).is_ok(),
            "Raw in a non-covenant role should be tolerated: {:?}",
            check(&program).err()
        );
    }

    // ---- C1–C3 TYPE-STACK REJECT VECTORS ----------------------------------
    //
    // Structural / simple-relational checks (NOT an SMT solver). Each program
    // parses cleanly but violates one C-check; the relevant pass must reject it.
    // Helper to confirm acceptance of a hand-written program.
    fn assert_accepts(src: &str) {
        let program = parse(src).expect("source should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "expected acceptance, got: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    // C1: value CREATED — value-bearing `balance` assigned an arg, not derived
    // from its own prior value, under value_conserved.
    #[test]
    fn c1_rejects_value_created() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function transfer(int amount) : (int balance) {
      return A { balance: amount };
    }
  }
  lifecycle { live -> live via r.transfer; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "value-bearing field `balance`");
    }

    // C1: value DESTROYED/inflated — `supply` doubled (does not derive from its
    // own prior value via a conserving carry/adjust; `supply * 2` references
    // supply but the structural test only accepts carry or ±; here we use a
    // constant assignment to make the create/destroy unambiguous).
    #[test]
    fn c1_rejects_value_destroyed() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int supply; }
    #[covenant(mode = transition)]
    entrypoint function shrink(int amount) : (int supply) {
      return A { supply: 0 };
    }
  }
  lifecycle { live -> live via r.shrink; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "value-bearing field `supply`");
    }

    // C1: the mint/burn convention exempts an authorised supply change.
    #[test]
    fn c1_accepts_mint_exemption() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int supply; }
    #[covenant(mode = transition)]
    entrypoint function mint_more(int amount) : (int supply) {
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    // ---- LOW-1 (Phase C red-team): conservation-preserving forms only --------
    //
    // The old C1 rule accepted ANY self-referencing expression; these vectors
    // pin the tightened rule. A value-bearing field under value_conserved may
    // only be a bare carry or a single additive ± adjustment.

    /// LOW-1: value field MULTIPLIED (`balance: balance * 2`) — references its own
    /// prior value but scales it. Must now be rejected (was accepted before).
    #[test]
    fn c1_rejects_value_field_multiplied() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function inflate(int x) : (int balance) {
      requires checkSig(auth, owner);
      return A { balance: balance * 2 };
    }
    param pubkey owner;
    param sig auth;
  }
  lifecycle { live -> live via r.inflate; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "value-bearing field `balance`");
    }

    /// LOW-1: value field ZEROED via self-subtract (`balance: balance - balance`).
    /// References its own prior value twice → destroys value. Must be rejected.
    #[test]
    fn c1_rejects_value_field_self_zeroed() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function zero() : (int balance) {
      requires checkSig(auth, owner);
      return A { balance: balance - balance };
    }
  }
  lifecycle { live -> live via r.zero; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "value-bearing field `balance`");
    }

    /// LOW-1: value field CONSTANT-replaced (`balance: 0`). No carry of the prior
    /// value at all. Must be rejected (already was, kept as a regression guard).
    #[test]
    fn c1_rejects_value_field_constant_replaced() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function reset() : (int balance) {
      requires checkSig(auth, owner);
      return A { balance: 0 };
    }
  }
  lifecycle { live -> live via r.reset; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "value-bearing field `balance`");
    }

    /// LOW-1 ACCEPT: the legitimate additive adjustment `balance: balance - amount`
    /// (the real ComplianceToken / DigitalReit shape) still passes.
    #[test]
    fn c1_accepts_additive_adjustment() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(int amount) : (int balance) {
      requires checkSig(auth, owner);
      return A { balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.spend; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    /// LOW-1 ACCEPT: a bare carry `balance: balance` (the DigitalReit `supply:
    /// supply` / CsciInstrument `amount: amount` shape) still passes.
    #[test]
    fn c1_accepts_bare_carry() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; int seq; }
    #[covenant(mode = transition)]
    entrypoint function touch() : (int balance, int seq) {
      requires checkSig(auth, owner);
      return A { balance: balance, seq: seq };
    }
  }
  lifecycle { live -> live via r.touch; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    // ---- LOW-2 (Phase C red-team): no-checkSig state mutation -----------------

    /// LOW-2: a state-mutating transition with ZERO authorization under
    /// `value_conserved` is rejected (was silently accepted before).
    #[test]
    fn c2_rejects_unauthorized_mutation_under_value_conserved() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(int amount) : (int balance) {
      return A { balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.spend; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "state-mutating transition has NO");
    }

    /// LOW-2: the same no-auth mutation is PERMITTED when no protection invariant
    /// is declared (it may be gated by covenant-ID lineage C2 cannot see).
    #[test]
    fn c2_accepts_unauthorized_mutation_without_invariant() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int counter; }
    #[covenant(mode = transition)]
    entrypoint function tick() : (int counter) {
      return A { counter: counter };
    }
  }
  lifecycle { live -> live via r.tick; }
  invariant no_undeclared_state;
}
"#;
        assert_accepts(src);
    }

    // C2: under-authorized transition — checkSig binds a caller-supplied pubkey
    // arg (the DigitalReit L1 finding as a compile-time error).
    #[test]
    fn c2_rejects_caller_supplied_pubkey() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, pubkey who, int amount) : (int balance) {
      requires checkSig(auth, who);
      return A { balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
}
"#;
        assert_rejects_with(src, "capability check failed");
    }

    // C2: authorizing against committed state (a state field) is accepted.
    #[test]
    fn c2_accepts_committed_pubkey() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int balance) {
      requires checkSig(auth, owner);
      return A { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
}
"#;
        assert_accepts(src);
    }

    // C3: non-monotonic seq — monotonic_seq declared but the return does not
    // advance seq by exactly one.
    #[test]
    fn c3_rejects_non_monotonic_seq() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int seq; bytes32 commit; }
    #[covenant(mode = transition)]
    entrypoint function attest(bytes32 next_commit) : (int seq, bytes32 commit) {
      return A { seq: seq, commit: next_commit };
    }
  }
  lifecycle { live -> live via r.attest; }
  invariant monotonic_seq;
}
"#;
        assert_rejects_with(src, "invariant `monotonic_seq` violated");
    }

    // C3: a correct seq increment under monotonic_seq is accepted.
    #[test]
    fn c3_accepts_monotonic_seq() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int seq; bytes32 commit; }
    #[covenant(mode = transition)]
    entrypoint function attest(bytes32 next_commit) : (int seq, bytes32 commit) {
      return A { seq: seq + 1, commit: next_commit };
    }
  }
  lifecycle { live -> live via r.attest; }
  invariant monotonic_seq;
}
"#;
        assert_accepts(src);
    }

    // C3: negative amount — non_negative_amount declared but `amount` is never
    // bounded.
    #[test]
    fn c3_rejects_unbounded_amount() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function transfer(int amount) : (int balance) {
      return A { balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.transfer; }
  invariant non_negative_amount;
}
"#;
        assert_rejects_with(src, "invariant `non_negative_amount` violated");
    }

    // C3: a present `require amount >= 0` under non_negative_amount is accepted.
    #[test]
    fn c3_accepts_bounded_amount() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function transfer(int amount) : (int balance) {
      requires amount >= 0;
      return A { balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.transfer; }
  invariant non_negative_amount;
}
"#;
        assert_accepts(src);
    }

    // Red-team LOW (c): a scalar return referencing more than one state field is
    // rejected fail-closed (it would be broadcast into every referenced field).
    #[test]
    fn c_rejects_scalar_multi_field_return() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int a; int b; }
    #[covenant(mode = transition)]
    entrypoint function step() : (int a) {
      return a + b;
    }
  }
  lifecycle { live -> live via r.step; }
}
"#;
        assert_rejects_with(src, "scalar return references multiple state fields");
    }

    // Red-team LOW (c): a scalar return referencing exactly one state field is
    // fine (the emitter broadcasts it into just that one field).
    #[test]
    fn c_accepts_scalar_single_field_return() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int a; int b; }
    #[covenant(mode = transition)]
    entrypoint function step(int x) : (int a) {
      return a + x;
    }
  }
  lifecycle { live -> live via r.step; }
}
"#;
        assert_accepts(src);
    }

    // MEDIUM fix: a scalar return that inflates a value-bearing field (e.g.
    // `return balance * 2`) under value_conserved must be REJECTED by C1 even
    // though the return is not an object literal.
    #[test]
    fn c1_rejects_scalar_value_bearing_inflation() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function inflate(sig auth) : (int balance) {
      requires checkSig(auth, owner);
      return balance * 2;
    }
  }
  lifecycle { live -> live via r.inflate; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "scalar return assigns value-bearing field `balance`");
    }

    // MEDIUM fix: a no-checkSig scalar-return state mutation under value_conserved
    // must be REJECTED by C2/LOW-2 (the scalar `return balance - amount` is a
    // state mutation that must be authorized when an invariant is declared).
    #[test]
    fn c2_rejects_no_checksig_scalar_mutation_under_value_conserved() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function drain(int amount) : (int balance) {
      return balance - amount;
    }
  }
  lifecycle { live -> live via r.drain; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "state-mutating transition has NO");
    }

    // MEDIUM fix (accept-case): a scalar return on a non-value-bearing field
    // (`value` is int, not named balance/amount/supply, and type is not coin)
    // must still PASS C1 even when value_conserved is declared. A committed-key
    // checkSig satisfies C2 so C2 does not mask the C1 result.
    #[test]
    fn c1_accepts_scalar_non_value_bearing_field() {
        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param pubkey admin;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(sig auth, int delta) : (int value) {
      requires checkSig(auth, admin);
      return value + delta;
    }
  }
  lifecycle { live -> live via counter.bump; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    // Red-team LOW (a): State / State[] comparison operands are rejected
    // fail-closed. `prev_states[0] == prev_states[0]` compares two `State`
    // records — no scalar surface type — and must be rejected.
    #[test]
    fn rejects_state_equality() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      requires prev_states[0] == prev_states[0];
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "comparison `==` requires scalar operands");
    }

    // Red-team LOW (a): comparing the whole `prev_states` array is also rejected.
    #[test]
    fn rejects_state_array_equality() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      requires prev_states == prev_states;
      return value + delta;
    }
  }
  lifecycle { live -> live via r.bump; }
}
"#;
        assert_rejects_with(src, "comparison `==` requires scalar operands");
    }

    // ── D1: `coin` is a distinct strictly-conserved type ────────────────────
    //
    // The Portrait type checker treats `coin` as value-bearing and non-arithmetic:
    // a `coin` field may ONLY be a bare carry (the emitter lowers `coin` to `int`
    // in the .sil, so sema is the sole keeper of these guarantees).

    /// A covenant with a `coin` field whose value is a bare carry passes the
    /// full checker (value_conserved treats `coin` as value-bearing; bare carry
    /// is conservation-preserving).
    #[test]
    fn d1_accepts_coin_field_bare_carry() {
        let src = r#"
pragma portrait ^0.1.0;
app CoinHolder {
  role holder {
    param pubkey owner;
    param coin   amount;
    state {
      pubkey owner;
      coin   amount;
    }
    #[covenant(mode = transition)]
    entrypoint function carry(sig auth) : (pubkey owner, coin amount) {
      requires checkSig(auth, owner);
      return CoinHolder { owner: owner, amount: amount };
    }
  }
  lifecycle { live -> live via holder.carry; }
  invariant value_conserved;
  invariant no_undeclared_state;
}
"#;
        assert_accepts(src);
    }

    /// Arithmetic on a `coin` operand is rejected by the type checker — a `coin`
    /// can never be adjusted, only carried.
    #[test]
    fn d1_rejects_coin_arithmetic() {
        let src = r#"
pragma portrait ^0.1.0;
app CoinHolder {
  role holder {
    param pubkey owner;
    param coin   amount;
    state {
      pubkey owner;
      coin   amount;
    }
    #[covenant(mode = transition)]
    entrypoint function bump(sig auth) : (pubkey owner, coin amount) {
      requires checkSig(auth, owner);
      return CoinHolder { owner: owner, amount: amount + 1 };
    }
  }
  lifecycle { live -> live via holder.carry; }
  invariant value_conserved;
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "arithmetic `+` requires int operands");
    }

    /// Comparing a `coin` to an `int` is rejected (operands must match type) —
    /// a `coin` cannot be used as a comparable ceiling.
    #[test]
    fn d1_rejects_coin_vs_int_comparison() {
        let src = r#"
pragma portrait ^0.1.0;
app CoinHolder {
  role holder {
    param pubkey owner;
    param coin   amount;
    state {
      pubkey owner;
      coin   amount;
    }
    #[covenant(mode = transition)]
    entrypoint function carry(sig auth) : (pubkey owner, coin amount) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      return CoinHolder { owner: owner, amount: amount };
    }
  }
  lifecycle { live -> live via holder.carry; }
  invariant value_conserved;
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "comparison `>=` requires operands of the same type");
    }

    // ── D2: the `authorized` capability invariant drives C2 on its own ──────
    //
    // C2's no-auth fail-safe fires under EITHER `value_conserved` OR a custom
    // `authorized` invariant. These vectors isolate `authorized`: the program
    // does NOT declare `value_conserved`, so `authorized` is the SOLE reason a
    // no-checkSig state mutation is rejected — proving the invariant is wired in
    // and actually used, not merely recognized.

    /// D2: `invariant authorized;` (no value_conserved) — a state-mutating
    /// transition with NO checkSig must be rejected purely because `authorized`
    /// is declared.
    #[test]
    fn d2_authorized_invariant_rejects_unauthorized_mutation() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int counter; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int counter) {
      return A { counter: counter };
    }
  }
  lifecycle { live -> live via r.bump; }
  invariant authorized;
}
"#;
        assert_rejects_with(src, "state-mutating transition has NO");
    }

    /// D2 ACCEPT: under `invariant authorized;` (no value_conserved), a
    /// state-mutating transition that DOES bind a committed key passes C2.
    #[test]
    fn d2_authorized_invariant_accepts_committed_checksig() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int counter; }
    #[covenant(mode = transition)]
    entrypoint function bump(sig auth) : (pubkey owner, int counter) {
      requires checkSig(auth, owner);
      return A { owner: owner, counter: counter };
    }
  }
  lifecycle { live -> live via r.bump; }
  invariant authorized;
}
"#;
        assert_accepts(src);
    }

    /// D2 ACCEPT: the real MultisigTreasury source (now declaring `authorized`,
    /// `non_negative_amount`, and the `require amount >= 0` guard) passes the
    /// full checker.
    #[test]
    fn d2_accepts_multisig_treasury_source() {
        let src = include_str!("../../../../library/governance/treasury/MultisigTreasury.portrait");
        let program = parse(src).expect("MultisigTreasury.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "MultisigTreasury should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    // ── D2: `bounded_supply` ceiling refinement (C3) ────────────────────────
    //
    // NARROW + opt-in. Fires only when `invariant bounded_supply;` is declared
    // AND the role has int `supply` + `total` fields AND the transition takes an
    // int `amount` arg. It requires the StreamingVesting envelope guard
    // `require supply + amount <= total` (either operand order). Structural —
    // NOT a solver.

    /// D2: bounded_supply declared but the envelope guard is missing → reject.
    #[test]
    fn d2_bounded_supply_rejects_missing_envelope() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int total; int supply; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int total, int supply) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      return A { owner: owner, total: total, supply: supply + amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
  invariant bounded_supply;
}
"#;
        assert_rejects_with(src, "invariant `bounded_supply` violated");
    }

    /// D2 ACCEPT: the envelope guard present (`supply + amount <= total`) passes.
    #[test]
    fn d2_bounded_supply_accepts_envelope() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int total; int supply; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int total, int supply) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      requires supply + amount <= total;
      return A { owner: owner, total: total, supply: supply + amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
  invariant bounded_supply;
}
"#;
        assert_accepts(src);
    }

    /// D2 ACCEPT: the envelope guard in the reversed `amount + supply` order and
    /// `total >= ...` form is also accepted (operand-order tolerance).
    #[test]
    fn d2_bounded_supply_accepts_reversed_form() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int total; int supply; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int total, int supply) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      requires total >= amount + supply;
      return A { owner: owner, total: total, supply: supply + amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
  invariant bounded_supply;
}
"#;
        assert_accepts(src);
    }

    /// D2 ACCEPT: the real StreamingVesting source (now declaring
    /// `bounded_supply`) passes the full checker.
    #[test]
    fn d2_accepts_streaming_vesting_source() {
        let src = include_str!("../../../../library/finance/streaming/StreamingVesting.portrait");
        let program = parse(src).expect("StreamingVesting.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "StreamingVesting should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    // ── D3: round-2 refinements (spending_cap / multisig_threshold /
    //        temporal_guard) ────────────────────────────────────────────────
    //
    // Each is NARROW + opt-in (fires only when its custom invariant is declared)
    // and a STRUCTURAL shape match on the require AST — NOT an SMT proof. Accept
    // + reject vectors below.

    // ---- spending_cap ---------------------------------------------------------

    /// D3 REJECT: `spending_cap` declared, an int `amount` arg is taken, but no
    /// `require amount <= limit` cap is present → rejected.
    #[test]
    fn d3_spending_cap_rejects_missing_cap() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role vault {
    param pubkey owner;
    state { pubkey owner; int balance; int limit; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int balance, int limit) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      requires amount <= balance;
      return A { owner: owner, balance: balance - amount, limit: limit };
    }
  }
  lifecycle { live -> live via vault.withdraw; }
  invariant spending_cap;
}
"#;
        assert_rejects_with(src, "invariant `spending_cap` violated");
    }

    /// D3 ACCEPT: the cap require `amount <= limit` (committed `limit`) is present.
    #[test]
    fn d3_spending_cap_accepts_cap() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role vault {
    param pubkey owner;
    state { pubkey owner; int balance; int limit; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int balance, int limit) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      requires amount <= limit;
      requires amount <= balance;
      return A { owner: owner, balance: balance - amount, limit: limit };
    }
  }
  lifecycle { live -> live via vault.withdraw; }
  invariant spending_cap;
}
"#;
        assert_accepts(src);
    }

    /// D3 ACCEPT: the real SpendingLimitVault source (declaring `spending_cap`)
    /// passes the full checker.
    #[test]
    fn d3_accepts_spending_limit_vault_source() {
        let src =
            include_str!("../../../../library/custody/spending-limit/SpendingLimitVault.portrait");
        let program = parse(src).expect("SpendingLimitVault.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "SpendingLimitVault should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    // ---- multisig_threshold ---------------------------------------------------

    /// D3 REJECT: `multisig_threshold` declared but the state-mutating transition
    /// authorizes with only ONE committed-key checkSig → rejected.
    #[test]
    fn d3_multisig_threshold_rejects_single_signer() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role t {
    param pubkey signer_a;
    state { pubkey signer_a; pubkey signer_b; int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(sig auth_a, int amount) : (pubkey signer_a, pubkey signer_b, int balance) {
      requires checkSig(auth_a, signer_a);
      requires amount <= balance;
      return A { signer_a: signer_a, signer_b: signer_b, balance: balance - amount };
    }
  }
  lifecycle { live -> live via t.spend; }
  invariant multisig_threshold;
}
"#;
        assert_rejects_with(src, "invariant `multisig_threshold` violated");
    }

    /// D3 ACCEPT: two distinct committed-key checkSigs satisfy the threshold.
    #[test]
    fn d3_multisig_threshold_accepts_two_signers() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role t {
    param pubkey signer_a;
    state { pubkey signer_a; pubkey signer_b; int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(sig auth_a, sig auth_b, int amount) : (pubkey signer_a, pubkey signer_b, int balance) {
      requires checkSig(auth_a, signer_a);
      requires checkSig(auth_b, signer_b);
      requires amount <= balance;
      return A { signer_a: signer_a, signer_b: signer_b, balance: balance - amount };
    }
  }
  lifecycle { live -> live via t.spend; }
  invariant multisig_threshold;
}
"#;
        assert_accepts(src);
    }

    /// D3 ACCEPT: the real ArbiterEscrow source (2-of-3, declaring
    /// `multisig_threshold`) passes the full checker — the disjunction of
    /// conjunctive pairs binds three distinct committed keys.
    #[test]
    fn d3_accepts_arbiter_escrow_source() {
        let src = include_str!("../../../../library/finance/arbiter-escrow/ArbiterEscrow.portrait");
        let program = parse(src).expect("ArbiterEscrow.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "ArbiterEscrow should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    /// D3 ACCEPT: the real MultisigTreasury source (2-of-2, declaring
    /// `multisig_threshold`) passes the full checker.
    #[test]
    fn d3_accepts_multisig_treasury_source() {
        let src = include_str!("../../../../library/governance/treasury/MultisigTreasury.portrait");
        let program = parse(src).expect("MultisigTreasury.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "MultisigTreasury should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    // ---- temporal_guard -------------------------------------------------------

    /// D3 REJECT: `temporal_guard` declared, the transition reads `now_bucket` in
    /// a guard, but NOT in the `now_bucket >= <committed time>` gate form (here it
    /// compares against a caller-supplied arg, not committed state) → rejected.
    #[test]
    fn d3_temporal_guard_rejects_non_committed_gate() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int deadline; int settled; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth, int now_bucket, int claimed) : (pubkey owner, int deadline, int settled) {
      requires checkSig(auth, owner);
      requires now_bucket >= claimed;
      return A { owner: owner, deadline: deadline, settled: 1 };
    }
  }
  lifecycle { live -> live via r.refund; }
  invariant temporal_guard;
}
"#;
        assert_rejects_with(src, "invariant `temporal_guard` violated");
    }

    /// D3 ACCEPT: a `now_bucket >= deadline` gate against a committed `deadline`.
    #[test]
    fn d3_temporal_guard_accepts_committed_deadline() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int deadline; int settled; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth, int now_bucket) : (pubkey owner, int deadline, int settled) {
      requires checkSig(auth, owner);
      requires now_bucket >= deadline;
      return A { owner: owner, deadline: deadline, settled: 1 };
    }
  }
  lifecycle { live -> live via r.refund; }
  invariant temporal_guard;
}
"#;
        assert_accepts(src);
    }

    /// D3 ACCEPT: a `now_bucket >= last_active + timeout` committed window gate.
    #[test]
    fn d3_temporal_guard_accepts_committed_window() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; pubkey heir; int last_active; int timeout; }
    #[covenant(mode = transition)]
    entrypoint function claim(sig auth, int now_bucket) : (pubkey owner, pubkey heir, int last_active, int timeout) {
      requires checkSig(auth, heir);
      requires now_bucket >= last_active + timeout;
      return A { owner: heir, heir: heir, last_active: now_bucket, timeout: timeout };
    }
  }
  lifecycle { live -> live via r.claim; }
  invariant temporal_guard;
}
"#;
        assert_accepts(src);
    }

    /// D3 ACCEPT: the real HTLC source (declaring `temporal_guard`) passes the
    /// full checker — `refund` gates on the committed `deadline`, `claim` reads no
    /// `now_bucket` guard so it is untouched.
    #[test]
    fn d3_accepts_htlc_source() {
        let src = include_str!("../../../../library/finance/htlc/Htlc.portrait");
        let program = parse(src).expect("Htlc.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "Htlc should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    /// D3 ACCEPT: the real DeadMansSwitch source (declaring `temporal_guard`)
    /// passes — `claim` gates on the committed `last_active + timeout` window;
    /// `heartbeat` reads `now_bucket` only in its return (not a guard) so it is
    /// not treated as a gate.
    #[test]
    fn d3_accepts_dead_mans_switch_source() {
        let src =
            include_str!("../../../../library/custody/dead-mans-switch/DeadMansSwitch.portrait");
        let program = parse(src).expect("DeadMansSwitch.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "DeadMansSwitch should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    // ── D4: conservation_split (paired two-field transfer) ───────────────────

    /// D4 ACCEPT: a matched transfer — `from_balance: from_balance - amount`
    /// paired with `to_balance: to_balance + amount` (the SAME term `amount`).
    #[test]
    fn d4_accepts_matched_transfer() {
        let src = r#"
pragma portrait ^0.1.0;
app T {
  role acct {
    param int from_balance;
    param int to_balance;
    param pubkey owner;
    state { int from_balance; int to_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function transfer(sig auth, int amount) : (int from_balance, int to_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return T {
        from_balance: from_balance - amount,
        to_balance:   to_balance + amount,
        owner:        owner
      };
    }
  }
  lifecycle { live -> live via acct.transfer; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_accepts(src);
    }

    /// D4 REJECT: mismatched terms — `from_balance - amount` but
    /// `to_balance + fee` (the +term differs from the -term).
    #[test]
    fn d4_rejects_mismatched_term() {
        let src = r#"
pragma portrait ^0.1.0;
app T {
  role acct {
    param int from_balance;
    param int to_balance;
    param pubkey owner;
    state { int from_balance; int to_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function transfer(sig auth, int amount, int fee) : (int from_balance, int to_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return T {
        from_balance: from_balance - amount,
        to_balance:   to_balance + fee,
        owner:        owner
      };
    }
  }
  lifecycle { live -> live via acct.transfer; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_rejects_with(src, "invariant `conservation_split` violated");
    }

    /// D4 REJECT: a single-field drain — `from_balance` decreases with NO
    /// matching counter-increase on another value-bearing field.
    #[test]
    fn d4_rejects_single_field_drain() {
        let src = r#"
pragma portrait ^0.1.0;
app T {
  role acct {
    param int from_balance;
    param int to_balance;
    param pubkey owner;
    state { int from_balance; int to_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function drain(sig auth, int amount) : (int from_balance, int to_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return T {
        from_balance: from_balance - amount,
        to_balance:   to_balance,
        owner:        owner
      };
    }
  }
  lifecycle { live -> live via acct.drain; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_rejects_with(src, "invariant `conservation_split` violated");
    }

    /// D4 ACCEPT: the shipped InternalTransfer source passes the full checker.
    #[test]
    fn d4_accepts_internal_transfer_source() {
        let src = include_str!("../../../../library/finance/transfer/InternalTransfer.portrait");
        let program = parse(src).expect("InternalTransfer.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "InternalTransfer should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    // ── D4 (N-field generalization): N>2 internal splits ─────────────────────

    /// D4 ACCEPT (N=3): a true three-field split — `a: a - (x + y)` paired with
    /// `b: b + x` and `c: c + y`. The subtracted atoms {x, y} cancel the added
    /// atoms {x, y}, so the deltas net to zero across THREE value-bearing fields.
    #[test]
    fn d4_accepts_three_field_split() {
        let src = r#"
pragma portrait ^0.1.0;
app S {
  role pool {
    param int a_balance;
    param int b_balance;
    param int c_balance;
    param pubkey owner;
    state { int a_balance; int b_balance; int c_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function rebalance(sig auth, int x, int y) : (int a_balance, int b_balance, int c_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return S {
        a_balance: a_balance - (x + y),
        b_balance: b_balance + x,
        c_balance: c_balance + y,
        owner:     owner
      };
    }
  }
  lifecycle { live -> live via pool.rebalance; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_accepts(src);
    }

    /// D4 ACCEPT (N=3, carry leg): a two-field transfer with a THIRD value-bearing
    /// field carried unchanged — `a: a - x`, `b: b + x`, `c: c`. The carried leg
    /// has delta 0 and does not disturb the net-zero balance.
    #[test]
    fn d4_accepts_two_field_transfer_with_carry_leg() {
        let src = r#"
pragma portrait ^0.1.0;
app S {
  role pool {
    param int a_balance;
    param int b_balance;
    param int c_balance;
    param pubkey owner;
    state { int a_balance; int b_balance; int c_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function move_ab(sig auth, int x) : (int a_balance, int b_balance, int c_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return S {
        a_balance: a_balance - x,
        b_balance: b_balance + x,
        c_balance: c_balance,
        owner:     owner
      };
    }
  }
  lifecycle { live -> live via pool.move_ab; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_accepts(src);
    }

    /// D4 REJECT (N=3, value created): `a: a - x`, `b: b + x`, `c: c + y`. The
    /// added atoms {x, y} do NOT cancel the subtracted atoms {x} — `y` is created
    /// out of nothing across the three fields.
    #[test]
    fn d4_rejects_three_field_value_created() {
        let src = r#"
pragma portrait ^0.1.0;
app S {
  role pool {
    param int a_balance;
    param int b_balance;
    param int c_balance;
    param pubkey owner;
    state { int a_balance; int b_balance; int c_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function inflate(sig auth, int x, int y) : (int a_balance, int b_balance, int c_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return S {
        a_balance: a_balance - x,
        b_balance: b_balance + x,
        c_balance: c_balance + y,
        owner:     owner
      };
    }
  }
  lifecycle { live -> live via pool.inflate; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_rejects_with(src, "invariant `conservation_split` violated");
    }

    /// D4 REJECT (N=3, value destroyed): `a: a - (x + y)`, `b: b + x`, `c: c`.
    /// The subtracted atoms {x, y} do NOT cancel the added atoms {x} — `y` is
    /// destroyed (it leaves `a` but arrives nowhere in the covenant).
    #[test]
    fn d4_rejects_three_field_value_destroyed() {
        let src = r#"
pragma portrait ^0.1.0;
app S {
  role pool {
    param int a_balance;
    param int b_balance;
    param int c_balance;
    param pubkey owner;
    state { int a_balance; int b_balance; int c_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function lose_y(sig auth, int x, int y) : (int a_balance, int b_balance, int c_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return S {
        a_balance: a_balance - (x + y),
        b_balance: b_balance + x,
        c_balance: c_balance,
        owner:     owner
      };
    }
  }
  lifecycle { live -> live via pool.lose_y; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_rejects_with(src, "invariant `conservation_split` violated");
    }

    /// D4 REJECT (N=3, non-additive value-field mutation): one leg scales
    /// multiplicatively (`c: c * 2`), which is not analyzable as a value delta —
    /// even though the other two legs (`a: a - x`, `b: b + x`) balance.
    #[test]
    fn d4_rejects_three_field_non_additive_mutation() {
        let src = r#"
pragma portrait ^0.1.0;
app S {
  role pool {
    param int a_balance;
    param int b_balance;
    param int c_balance;
    param pubkey owner;
    state { int a_balance; int b_balance; int c_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function scale_c(sig auth, int x) : (int a_balance, int b_balance, int c_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return S {
        a_balance: a_balance - x,
        b_balance: b_balance + x,
        c_balance: c_balance * 2,
        owner:     owner
      };
    }
  }
  lifecycle { live -> live via pool.scale_c; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_rejects_with(src, "invariant `conservation_split` violated");
    }

    /// D4 ACCEPT: the shipped 3-field InternalSplit source passes the full
    /// checker (parse + sema; the engrave→silverc path is exercised by the CLI).
    #[test]
    fn d4_accepts_internal_split_source() {
        let src = include_str!("../../../../library/finance/internal-split/InternalSplit.portrait");
        let program = parse(src).expect("InternalSplit.portrait should parse");
        let result = check(&program);
        assert!(
            result.is_ok(),
            "InternalSplit should pass the full checker: {:?}",
            result
                .err()
                .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
        );
    }

    /// D4 SCOPE GUARD: the shipped SPEND covenants must NOT be false-rejected.
    /// They use `value_conserved` (single-additive per-field, value moves OUT of
    /// the covenant) and do NOT declare `conservation_split`; the generalized
    /// N-field check must leave them passing.
    #[test]
    fn d4_spend_covenants_not_false_rejected() {
        let cases: &[(&str, &str)] = &[
            (
                "MultisigTreasury",
                include_str!("../../../../library/governance/treasury/MultisigTreasury.portrait"),
            ),
            (
                "SpendingLimitVault",
                include_str!(
                    "../../../../library/custody/spending-limit/SpendingLimitVault.portrait"
                ),
            ),
            (
                "Subscription",
                include_str!("../../../../library/finance/subscription/Subscription.portrait"),
            ),
        ];
        for (name, src) in cases {
            let program = parse(src).unwrap_or_else(|e| panic!("{name} should parse: {e:?}"));
            let result = check(&program);
            assert!(
                result.is_ok(),
                "spend covenant {name} must NOT be false-rejected by the N-field \
                 conservation_split generalization: {:?}",
                result
                    .err()
                    .map(|ds| ds.into_iter().map(|d| d.message).collect::<Vec<_>>())
            );
        }
    }

    // ── Allocation advisor (read-only) ──────────────────────────────────────

    /// A two-layer source: a clean covenant entrypoint + a vProg entrypoint whose
    /// body holds a real out-of-subset construct (a `for` loop).
    fn two_layer_loop_src() -> &'static str {
        r#"pragma portrait ^0.1.0;
app A {
  role r {
    state { int v; }
    #[covenant(mode = transition)]
    entrypoint function settle(int amount) : (int v) {
      return v - amount;
    }
    entrypoint function tally(int n) {
      for (i = 0; i < n; i = i + 1) { x = x + 1 };
      return v;
    }
  }
  lifecycle { live -> live via r.settle; }
  invariant no_undeclared_state;
}
"#
    }

    #[test]
    fn advisor_flags_covenant_entrypoint_holding_rejected_construct() {
        // A `#[covenant]` entrypoint that (defensively) carries a rejection-set
        // construct as a Raw hole must be FLAGGED with a clear route-to-vProg note.
        // We build the program AST directly (the parser would reject this at parse
        // for a covenant mode, which is the primary gate; the advisor is the
        // defensive cross-check, so we assert it on a constructed mismatch).
        use portrait_syntax::{App, CovenantMode, Entry, Role, Stmt};
        let program = Program {
            pragma: "portrait ^0.1.0".into(),
            uses: vec![],
            app: App {
                name: "A".into(),
                roles: vec![Role {
                    name: "r".into(),
                    component: None,
                    params: vec![],
                    state: vec![],
                    entrypoints: vec![Entry {
                        name: "bad".into(),
                        mode: CovenantMode::Transition,
                        args: vec![],
                        returns: None,
                        requires: vec![],
                        body: vec![Stmt::Raw("for (i = 0; i < n) { }".into())],
                    }],
                }],
                lifecycle: vec![],
                flow: None,
                invariants: vec![],
            },
        };
        let advisories = advise(&program);
        assert!(
            advisories.iter().any(|a| a.role == "r"
                && a.entry == "bad"
                && a.layer == "Covenant"
                && a.message.contains("marked covenant but uses `for`")
                && a.message.contains("vProgs")),
            "covenant entrypoint with a `for` hole must be flagged, got: {advisories:?}"
        );
    }

    #[test]
    fn advisor_notes_vprog_entrypoint_holding_rejected_construct() {
        // The vProg `tally` entrypoint holds a `for` loop (now accepted as a Raw
        // hole). The advisor should confirm it is correctly on the vProgs layer.
        let program = parse(two_layer_loop_src()).expect("two-layer source parses");
        let advisories = advise(&program);
        assert!(
            advisories.iter().any(|a| a.entry == "tally"
                && a.layer == "VProg"
                && a.message.contains("`for`")
                && a.message.contains("correctly allocated to the vProgs layer")),
            "vProg entrypoint with a loop must be noted as correctly allocated, got: {advisories:?}"
        );
    }

    #[test]
    fn advisor_does_not_false_flag_clean_covenant() {
        // A clean covenant entrypoint (only typed require/return) must NOT be
        // flagged at all — no false positives on legitimate covenants.
        let program = parse(two_layer_loop_src()).expect("two-layer source parses");
        let advisories = advise(&program);
        assert!(
            !advisories.iter().any(|a| a.entry == "settle"),
            "clean covenant `settle` must not be flagged, got: {advisories:?}"
        );
    }

    #[test]
    fn advisor_notes_covenant_legal_vprog_as_promotable() {
        // The tier3-demo vProg entrypoint (`verify_compliance`) is fully
        // covenant-legal (only a typed return). The advisor should note it as
        // promotable rather than flag it as a mismatch.
        let src = include_str!("../../../../examples/tier3-demo/ComplianceToken.portrait");
        let program = parse(src).expect("ComplianceToken parses");
        let advisories = advise(&program);
        assert!(
            advisories.iter().any(|a| a.entry == "verify_compliance"
                && a.layer == "VProg"
                && a.message.contains("could be promoted to a covenant")),
            "covenant-legal vProg entrypoint should be noted promotable, got: {advisories:?}"
        );
    }
}
