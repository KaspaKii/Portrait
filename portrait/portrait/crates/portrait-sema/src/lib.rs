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
//!   future work. Structural cancellation is sign-blind on its own — the same
//!   term appears on both legs whatever its sign — so the A6-sign non-negativity
//!   requirement below applies per leg as well.
//! * **No arithmetic reasoning.** C1 does not know that `balance - amount` can
//!   underflow, that `supply + amount` can overflow, or that the operands have
//!   any particular range. It matches the *expression shape*, not its values.
//! * **WHICH FIELDS C1 COVERS AT ALL.** Both checks act only on fields the
//!   checker considers *value-bearing*, and that is a NAME/TYPE rule, not an
//!   inference: for `value_conserved` a field qualifies iff its declared type is
//!   `coin` OR its name is exactly `balance`, `amount`, or `supply`; for
//!   `conservation_split`, the same plus any name ENDING in `balance`. A value
//!   field called `funds` or `principal` is outside both — the invariant is
//!   declared, reports ok, and has verified nothing about it. [`warnings`] flags
//!   the fully-vacuous case (a role where NO field qualifies); partial coverage
//!   (one qualifying field, three not) is silent. This is the honest scope of
//!   "unconditional": unconditional WITHIN that field set.
//! * **The adjustment term's SIGN is checked; its MAGNITUDE is not (A6-sign).** The
//!   `e` in `f ± e` must be established non-negative by the SAME entrypoint:
//!   every top-level `+`-atom of `e` has to be a non-negative int literal or a
//!   name that entrypoint guards with `require <name> >= 0` (or `> -1`). Without
//!   this, a negative `e` inverts the operator — `f - e` INCREASES the field
//!   (model money-printing) and `f + e` DECREASES it (value destruction, and
//!   under `conservation_split` a REVERSE transfer that drains the destination
//!   leg). Being COMMITTED at genesis does not qualify: genesis can commit a
//!   negative. What remains unconstrained is the term's UPPER bound — a ceiling
//!   is still the job of the opt-in C3 refinements (`bounded_supply`,
//!   `spending_cap`), each itself a narrow structural pattern match, not a proof.
//!
//! In short: C1/C3 reject the blunt supply-inflation / value-destruction /
//! missing-authorization shapes, require the sign of every adjustment term to be
//! established, and pin a small set of explicitly-declared refinement patterns;
//! the opt-in `conservation_split` adds structural N-field internal cross-field
//! cancellation on top. They are NOT a proof of economic soundness — a covenant
//! that passes them can still be wrong about properties that need a solver
//! (numeric ranges, conditionals, arithmetic identities, and value flow that
//! crosses the covenant boundary).
//!
//! # SCOPE LABEL — `value_conserved` is MODEL-ONLY, not a script/output check
//!
//! `value_conserved` reasons about the *model* (the declared state fields): it
//! forbids a value-bearing model field from mutating outside the carry/single-
//! additive shape. It does NOT bind the on-chain output value or payee — the
//! emitted `.sil` constrains no transaction output amount or destination. The
//! name is kept (label-now, rename-later); its true, per-pattern scope is the
//! enforcement matrix at `library/ENFORCEMENT.md`.

use std::collections::{HashMap, HashSet};

use portrait_syntax::{
    AfterDeadline, App, BinOp, CovenantMode, Entry, Expr, Flow, Invariant, Program, ReturnExpr,
    Role, Step, Stmt, Type, UnOp,
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
                    // B3: a TERMINAL transition releases the coin via `pays(...)` and the
                    // spending UTXO is consumed — there is no successor covenant. A `return`
                    // on that path is a contradiction (the emitter drops it; the reader is
                    // misled into thinking the covenant continues). Reject it fail-loud.
                    CovenantMode::Transition if edge.terminal && returns => {
                        diags.push(Diagnostic::new(format!(
                            "terminal transition `{}.{}` (edge {} -> {}) must not return a \
                             successor; the coin is released via pays and the UTXO is consumed",
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
        check_arg_shadowing(role, &mut diags);
        check_role_exprs(role, &mut diags);
        check_pays(role, &mut diags);
        check_after(role, &mut diags);
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
    let want_payout_bound = declares("payout_bound");
    // A4-full: formula-bearing temporal invariants bind a named entrypoint to
    // carry the matching `after(...)` consensus CLTV clause (STRUCTURAL — not an
    // SMT proof). Resolved across all roles, so checked once at app scope.
    check_temporal_path_invariants(app, &mut diags);
    // A6: payout_bound is app-scoped so the ZERO-settlement vacuity check counts
    // recognized settlements across every role (a per-role vacuity check would
    // false-reject a role that legitimately has no settling transition).
    if want_payout_bound {
        check_payout_bound(app, &mut diags);
    }
    // LOW-2: require authorization on state-mutating transitions when the app
    // declares a protection invariant (`value_conserved` or custom `authorized`).
    let require_auth = value_conserved || declares(AUTH_INVARIANT);
    // App-scoped terminal edges — a supply-change entry must not be a terminal
    // (coin-releasing) spend (A2-full RT-2), so `check_supply_change` needs the
    // same terminal set `payout_bound` uses.
    let terminal_entries = terminal_entry_set(app);
    for role in &app.roles {
        check_reserved_and_duplicate_names(role, &mut diags);
        check_c1_value_conservation(role, value_conserved, &mut diags);
        check_c2_authorization(role, require_auth, &mut diags);
        // A2-full: the explicit `supply_change = A` capability is checked
        // UNCONDITIONALLY (independent of any declared invariant): A must be a
        // committed key, guaranteed to sign on every satisfying path, and must
        // not release coin (no `pays` / not terminal).
        check_supply_change(role, &terminal_entries, &mut diags);
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

/// L-1: reject an entrypoint argument whose name collides with a role param or
/// state field (a genesis-committed name). Such a shadow makes a caller-supplied
/// value indistinguishable from committed state by name — C2 would mistake the
/// caller value for a committed key, and the emitter/silverc would otherwise
/// reject it downstream as a duplicate binding (a raw panic). The checker owns
/// the rejection here with a clean diagnostic. (A role param sharing a name with
/// a state field is the NORMAL constructor pattern and is NOT flagged — only a
/// caller-supplied *argument* reusing a committed name is.)
fn check_arg_shadowing(role: &Role, diags: &mut Vec<Diagnostic>) {
    let committed = committed_keys(role);
    for entry in &role.entrypoints {
        for a in &entry.args {
            if committed.contains(&a.name) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: entrypoint argument `{}` shadows a committed name (a role param \
                     or state field); rename the argument — a caller-supplied binding must not \
                     reuse a genesis-committed name (it would be mistaken for committed state and \
                     is a duplicate binding at emit)",
                    role.name, entry.name, a.name
                )));
            }
        }
    }
}

/// B2: validate every `pays(index, payee, amount)` clause in a role. A `pays`
/// clause makes the emitted covenant bind a transaction OUTPUT (amount + payee)
/// on-chain, so its operands must be trustworthy at genesis:
///
///   * it must sit in a `mode = transition` entrypoint — the emitter only lowers
///     transition bodies to a `.sil`, so a `pays` anywhere else would be silently
///     dropped, producing a covenant that LOOKS output-bound but enforces nothing
///     (the same false-accept hazard the `Stmt::Raw` guard closes);
///   * `payee` must resolve to a COMMITTED name (a role param or state field —
///     both baked into the covenant ID at genesis), NEVER a spender-supplied
///     argument: binding an output to a caller-chosen destination is no binding
///     at all, it just lets the spender pay themselves;
///   * `amount` must likewise be COMMITTED, and must be provably the quantity the
///     model gives up. Two accept paths, either of which suffices:
///     1. the operand is VALUE-BEARING (a `coin` field or a conventional balance
///        name) — the original rule; or
///     2. it is a committed `int` field that THIS entrypoint DRAWS DOWN — see
///        [`pays_amount_is_drawn_down`].
///
/// The output `index` is a non-negative literal by construction (parser); its
/// upper bound (`index < max_outs`) is enforced by the engine at spend time —
/// `OpTxOutputAmount`/`OpTxOutputSpk` fail the script on an out-of-range index.
///
/// SCOPE the checker CANNOT enforce (see the emitter site + `library/ENFORCEMENT.md`):
/// the emitter picks the payee's spk builtin from the payee's DECLARED type
/// (`pubkey` → `ScriptPubKeyP2PK`, `byte[32]` → `ScriptPubKeyP2SH`); which of those
/// two forms the payee's REAL settlement address uses is a ceremony fact the
/// checker cannot see, so a `pubkey`-declared payee who actually settles to a
/// script hash is a documented precondition, not a diagnostic. The binding also
/// constrains ONLY `output[index]` (no value-conservation / mass check), and is
/// only as trustworthy as the ceremony that committed `payee`.
fn check_pays(role: &Role, diags: &mut Vec<Diagnostic>) {
    let committed = committed_keys(role);
    // Committed name -> declared type, for the value-bearing check on `amount`.
    let mut committed_ty: HashMap<&str, &Type> = HashMap::new();
    for p in &role.params {
        committed_ty.insert(p.name.as_str(), &p.ty);
    }
    for f in &role.state {
        committed_ty.insert(f.name.as_str(), &f.ty);
    }
    for entry in &role.entrypoints {
        let args: HashSet<&str> = entry.args.iter().map(|a| a.name.as_str()).collect();
        // D2: two `pays` clauses at the SAME output index in one entrypoint are a
        // footgun — the second binding silently overwrites the first's intent on
        // the same output. Reject the collision; distinct indices are fine.
        let mut seen_indices: HashSet<usize> = HashSet::new();
        for stmt in &entry.body {
            let Stmt::Pays {
                index,
                payee,
                amount,
            } = stmt
            else {
                continue;
            };
            let where_ = format!("`{}.{}`", role.name, entry.name);
            if !seen_indices.insert(*index) {
                diags.push(Diagnostic::new(format!(
                    "{where_}: two `pays(...)` clauses bind output index {index}; each output \
                     index may be bound at most once per entrypoint (the second binding would \
                     silently overwrite the first)"
                )));
            }
            if !matches!(entry.mode, CovenantMode::Transition) {
                diags.push(Diagnostic::new(format!(
                    "{where_}: `pays(...)` is only valid in a `mode = transition` entrypoint \
                     (a non-transition body is not lowered to a covenant, so the output binding \
                     would be silently dropped)"
                )));
            }
            // payee: must be committed, not a spender-supplied arg.
            if !committed.contains(payee) {
                let why = if args.contains(payee.as_str()) {
                    "a spender-supplied argument — binding an output to a caller-chosen \
                     destination is no binding at all"
                } else {
                    "not a committed name (declare it as a role param or state field)"
                };
                diags.push(Diagnostic::new(format!(
                    "{where_}: `pays(...)` payee `{payee}` must resolve to COMMITTED state; it is {why}"
                )));
            }
            // amount: must be committed AND value-bearing.
            if !committed.contains(amount) {
                let why = if args.contains(amount.as_str()) {
                    "a spender-supplied argument — the bound amount must be the committed value"
                } else {
                    "not a committed name (declare it as a role param or state field)"
                };
                diags.push(Diagnostic::new(format!(
                    "{where_}: `pays(...)` amount `{amount}` must resolve to COMMITTED state; it is {why}"
                )));
            } else if let Some(ty) = committed_ty.get(amount.as_str()) {
                let drawn_down = matches!(ty, Type::Int)
                    && pays_amount_is_drawn_down(role, entry, amount.as_str());
                if !is_value_bearing(amount, ty) && !drawn_down {
                    diags.push(Diagnostic::new(format!(
                        "{where_}: `pays(...)` amount `{amount}` is not value-bearing and is not \
                         drawn down by this entrypoint; bind a `coin` field (or a conventional \
                         balance name), or have this entrypoint's return DECREASE a value-bearing \
                         field by a guarded additive term containing `{amount}` (e.g. \
                         `balance: balance - {amount}` with `requires {amount} >= 0;`), so the \
                         committed value is what is paid"
                    )));
                }
            }
        }
    }
}

/// The SECOND accept path for a `pays(...)` amount operand: the operand is a
/// committed `int` field that THIS entrypoint's object return DRAWS DOWN — some
/// value-bearing state field's new value is `field - <term>`, `<term>` carries
/// `amount` as one of its `+`-atoms, and every `+`-atom of `<term>` is A6-sign
/// guarded (see [`unguarded_additive_atom`]).
///
/// WHY this shape and not a type change or a rename. What `pays` needs from its
/// amount operand is (a) that it is COMMITTED — checked by the caller — and (b)
/// that it is provably the quantity LEAVING the model. Path 1 (`is_value_bearing`)
/// gets (b) from a `coin` type or a conventional balance name. But a fee that must
/// be SUBTRACTED from a running balance cannot be typed `coin` at all (the type
/// checker forbids arithmetic on `coin`, so a `coin` field can only ever be carried
/// verbatim), and buying value-bearing status by renaming the field to
/// `balance`/`amount`/`supply` would be naming-as-enforcement — the exact class the
/// A2/A5 capability work retired. The drawdown link gets (b) structurally instead:
/// the same entrypoint's successor subtracts the operand from a value-bearing
/// field, so the paid quantity IS a quantity the model gives up, under no name.
///
/// The A6-sign interlock is load-bearing for the same reason C1 needs it: a term
/// whose sign is unconstrained inverts the subtraction, and a "drawdown" by a
/// negative term is a top-up. An unguarded term therefore does NOT establish (b).
///
/// HONEST SCOPE: this is per-field structural arithmetic, not a flow analysis. It
/// establishes that the operand is subtracted from a value-bearing field HERE; it
/// does not establish that no other leg of the same return compensates (an internal
/// transfer that adds the same atom onto another value-bearing field still
/// qualifies). What the emitter binds is unaffected either way — consensus forces
/// `tx.outputs[index]` to pay exactly the committed operand to the committed payee.
fn pays_amount_is_drawn_down(role: &Role, entry: &Entry, amount: &str) -> bool {
    let Some(fields) = entry.body.iter().find_map(|s| match s {
        Stmt::Return(ReturnExpr::Object { fields, .. }) => Some(fields),
        _ => None,
    }) else {
        return false;
    };
    fields.iter().any(|(field, value)| {
        let is_value_field = role
            .state
            .iter()
            .any(|f| f.name == *field && is_value_bearing(&f.name, &f.ty));
        if !is_value_field {
            return false;
        }
        let SplitAdjust::Decrease(term) = classify_split_adjust(field, value) else {
            return false;
        };
        let mut atoms: Vec<&Expr> = Vec::new();
        flatten_add_atoms(term, &mut atoms);
        atoms
            .iter()
            .any(|atom| matches!(atom, Expr::Var(n) if n == amount))
            && unguarded_additive_atom(entry, term).is_none()
    })
}

/// Well-formedness of every `after(deadline)` time-gate clause (B1) — the
/// `pays`-parallel checker for the consensus CLTV gate. A clause is well-formed
/// only when:
///
///   * it appears in a `mode = transition` entrypoint (a non-transition body is
///     not lowered to a covenant, so the gate would be silently dropped);
///   * `deadline` resolves to COMMITTED state (a role param or state field), NOT a
///     spender-supplied argument — a caller-chosen deadline is no gate at all;
///   * `deadline` is a committed TIME field: an int-typed field carrying a
///     conventional time name (`time_committed_atoms`, the same allowlist the
///     `temporal_guard` invariant uses), so a non-time committed field
///     (`balance`, `owner`, …) cannot masquerade as a deadline;
///   * a `Sum(a, b)` window threshold is exactly one committed ANCHOR + one
///     committed DURATION (either order) — `anchor + anchor` overshoots and
///     `duration + duration` is no real gate (RT-1).
///
/// SCOPE the checker CANNOT enforce (see the emitter site + `library/ENFORCEMENT.md`):
/// the emitter lowers the gate to `require(tx.time >= prev_states[0].<deadline>)` →
/// `OpCheckLockTimeVerify`, which gates on a single monotone lock-time field. The
/// checker cannot see whether the committed `deadline` is a DAA score or a Unix
/// time, nor whether the spending tx's lock_time is in the same domain — those are
/// documented ceremony preconditions, not diagnostics.
fn check_after(role: &Role, diags: &mut Vec<Diagnostic>) {
    let committed = committed_keys(role);
    let time_atoms = time_committed_atoms(role);
    for entry in &role.entrypoints {
        let args: HashSet<&str> = entry.args.iter().map(|a| a.name.as_str()).collect();
        for stmt in &entry.body {
            let Stmt::After { deadline } = stmt else {
                continue;
            };
            let where_ = format!("`{}.{}`", role.name, entry.name);
            if !matches!(entry.mode, CovenantMode::Transition) {
                diags.push(Diagnostic::new(format!(
                    "{where_}: `after(...)` is only valid in a `mode = transition` entrypoint \
                     (a non-transition body is not lowered to a covenant, so the time gate \
                     would be silently dropped)"
                )));
            }
            // Each operand (one for `Field`, both for `Sum`) must resolve to a
            // committed TIME atom. A `Sum` is the two-atom window form
            // (`last_charged + period`): both addends carry the same requirement.
            let operands: Vec<&String> = match deadline {
                AfterDeadline::Field(f) => vec![f],
                AfterDeadline::Sum(a, b) => vec![a, b],
            };
            for operand in &operands {
                if !committed.contains(*operand) {
                    let why = if args.contains(operand.as_str()) {
                        "a spender-supplied argument — a caller-chosen deadline is no gate at all"
                    } else {
                        "not a committed name (declare it as a role param or state field)"
                    };
                    diags.push(Diagnostic::new(format!(
                        "{where_}: `after(...)` deadline `{operand}` must resolve to COMMITTED \
                         state; it is {why}"
                    )));
                } else if !time_atoms.contains(*operand) {
                    diags.push(Diagnostic::new(format!(
                        "{where_}: `after(...)` deadline `{operand}` must be a committed TIME \
                         field (an int-typed field carrying a conventional time name, e.g. \
                         `deadline`/`cliff`/`unlock_bucket`), so a non-time field cannot \
                         masquerade as a deadline"
                    )));
                }
            }
            // RT-1: a `Sum` window threshold must be exactly one committed ANCHOR
            // (an absolute time point: `deadline`/`cliff`/`last_charged`/…) PLUS one
            // committed DURATION (an interval: `period`/`timeout`), in either order.
            // `anchor + anchor` overshoots (can lock the UTXO past any real time) and
            // `duration + duration` is a tiny threshold (no real gate) — both are
            // authoring footguns, not deadlines. Only checked once both operands are
            // committed time atoms (else the per-operand diagnostics above already fire).
            if let AfterDeadline::Sum(a, b) = deadline {
                if time_atoms.contains(a) && time_atoms.contains(b) {
                    let is_anchor = |n: &str| TIME_ANCHOR_NAMES.contains(&n);
                    let is_duration = |n: &str| TIME_DURATION_NAMES.contains(&n);
                    let well_formed =
                        (is_anchor(a) && is_duration(b)) || (is_duration(a) && is_anchor(b));
                    if !well_formed {
                        diags.push(Diagnostic::new(format!(
                            "{where_}: `after({a} + {b})` must be a committed ANCHOR (an absolute \
                             time point, e.g. `deadline`/`cliff`/`last_charged`) plus a committed \
                             DURATION (an interval, e.g. `period`/`timeout`), in either order; \
                             anchor+anchor overshoots and duration+duration is no real gate"
                        )));
                    }
                }
            }
        }
    }
}

/// A4-full — formula-bearing temporal invariants
/// (`invariant <name>: <entry> => after(<deadline>);`).
///
/// For each declared `TemporalPath`, this resolves `entry` across every role and
/// requires that the named entrypoint CARRIES the matching `after(<deadline>)`
/// consensus CLTV clause (the exact `AfterDeadline` shape the invariant names).
///
/// This is a STRUCTURAL check — it verifies the entrypoint carries the matching
/// consensus gate the emitter lowers to `OpCheckLockTimeVerify`. It is NOT an
/// SMT-discharged temporal obligation, and it is a stronger, additional form of
/// the existence-only `temporal_guard` invariant, never a replacement. Deadline
/// well-formedness (committed + time-named) is already enforced by `check_after`
/// on the same clause, so it is not re-checked here.
///
/// Rejections: naming an entrypoint that no role declares; naming a non-transition
/// entrypoint (only a transition is lowered to a covenant carrying the gate); and
/// naming an entrypoint that lacks the matching `after(<deadline>)` clause — the
/// survives-deletion property (delete the `after(...)` clause and this fires).
///
/// The bare `entry` name binds by ENTRYPOINT NAME across roles: EVERY role that
/// declares an entrypoint of that name must carry the matching clause (M-1) — a
/// two-role app where `a.refund` gates but `b.refund` does not is rejected, so the
/// invariant cannot read as "refund is time-gated" while one occurrence is not.
fn check_temporal_path_invariants(app: &App, diags: &mut Vec<Diagnostic>) {
    for inv in &app.invariants {
        let Invariant::TemporalPath {
            name,
            entry,
            deadline,
        } = inv
        else {
            continue;
        };
        let occurrences: Vec<(&Role, &Entry)> = app
            .roles
            .iter()
            .filter_map(|role| find_entry(role, entry).map(|ep| (role, ep)))
            .collect();
        if occurrences.is_empty() {
            diags.push(Diagnostic::new(format!(
                "invariant `{name}`: entrypoint `{entry}` is not declared in any role"
            )));
            continue;
        }
        for (role, ep) in occurrences {
            if !matches!(ep.mode, CovenantMode::Transition) {
                diags.push(Diagnostic::new(format!(
                    "invariant `{name}`: entrypoint `{}.{entry}` is not a `mode = transition` \
                     entrypoint; only a transition is lowered to a covenant carrying the consensus \
                     CLTV gate the invariant binds",
                    role.name
                )));
                continue;
            }
            let carries = ep.body.iter().any(
                |s| matches!(s, Stmt::After { deadline: d } if after_deadline_matches(d, deadline)),
            );
            if !carries {
                diags.push(Diagnostic::new(format!(
                    "invariant `{name}`: entrypoint `{}.{entry}` must carry the matching \
                     `after({})` consensus CLTV clause (the STRUCTURAL check that the named \
                     entrypoint carries the matching consensus gate the emitter lowers to \
                     OpCheckLockTimeVerify — NOT an SMT-discharged temporal obligation); it does not",
                    role.name,
                    render_after_deadline(deadline)
                )));
            }
        }
    }
}

/// Two `after(...)` deadlines match iff they name the same committed time. A
/// `Sum` window is compared UNORDERED (L-2): `after(a + b)` and `after(b + a)`
/// are the same threshold, so operand order must not false-reject.
fn after_deadline_matches(a: &AfterDeadline, b: &AfterDeadline) -> bool {
    match (a, b) {
        (AfterDeadline::Field(x), AfterDeadline::Field(y)) => x == y,
        (AfterDeadline::Sum(x1, x2), AfterDeadline::Sum(y1, y2)) => {
            (x1 == y1 && x2 == y2) || (x1 == y2 && x2 == y1)
        }
        _ => false,
    }
}

/// Render an [`AfterDeadline`] back to its `after(...)` surface spelling for a
/// diagnostic message (`field` or `a + b`).
fn render_after_deadline(deadline: &AfterDeadline) -> String {
    match deadline {
        AfterDeadline::Field(f) => f.clone(),
        AfterDeadline::Sum(a, b) => format!("{a} + {b}"),
    }
}

/// The structural settlement signal `payout_bound` reuses. This is a RECOGNIZER,
/// not a complete settlement detector: an entrypoint SETTLES when it flips a
/// recognized ONE-SHOT FLAG from its unset value to a set value — pairing an entry
/// guard with the matching return flip. Exactly three shapes are recognized:
///
///   * int-literal flip — `require f == 0;` + return `f: <nonzero int>` (`f: 1`);
///   * computed int flip — `require f == 0;` + return `f: f + <nonzero int>`
///     (`f: settled + 1`), in either operand order;
///   * bool flip — `require f == false;` + return `f: true`.
///
/// A settlement written OUTSIDE these shapes is NOT recognized (H-1): `payout_bound`
/// obligates only recognized settlements, so an author must write the settlement in
/// a recognized shape. The app-level [`check_payout_bound`] fails loud when a
/// declared `payout_bound` recognizes ZERO settlements, so a vacuous pass cannot
/// masquerade as enforcement. This is the reveal-XOR-timeout spine of the
/// HTLC/Escrow patterns; it names the value-moving paths `payout_bound` governs.
fn settles(entry: &Entry) -> bool {
    let mut int_guarded: HashSet<&str> = HashSet::new();
    let mut bool_guarded: HashSet<&str> = HashSet::new();
    for s in &entry.body {
        if let Stmt::Require(expr) = s {
            if let Some(f) = eq_guarded_field(expr, &Expr::Int(0)) {
                int_guarded.insert(f);
            } else if let Some(f) = eq_guarded_field(expr, &Expr::Bool(false)) {
                bool_guarded.insert(f);
            }
        }
    }
    if int_guarded.is_empty() && bool_guarded.is_empty() {
        return false;
    }
    entry.body.iter().any(|s| match s {
        Stmt::Return(ReturnExpr::Object { fields, .. }) => fields.iter().any(|(k, v)| {
            let field = k.as_str();
            (int_guarded.contains(field) && flips_int_from_zero(field, v))
                || (bool_guarded.contains(field) && matches!(v, Expr::Bool(true)))
        }),
        _ => false,
    })
}

/// The field a `require lhs == rhs;` pins to `unset` (`f == unset` or `unset == f`),
/// if any — the entry half of a one-shot settlement flip. `unset` is `Int(0)` for
/// an int flag or `Bool(false)` for a bool flag.
fn eq_guarded_field<'a>(expr: &'a Expr, unset: &Expr) -> Option<&'a str> {
    let Expr::Binary {
        op: BinOp::Eq,
        lhs,
        rhs,
    } = expr
    else {
        return None;
    };
    match (lhs.as_ref(), rhs.as_ref()) {
        (Expr::Var(f), other) | (other, Expr::Var(f)) if other == unset => Some(f.as_str()),
        _ => None,
    }
}

/// A return value that flips int one-shot `field` from 0 to non-zero: a non-zero
/// int literal (`field: 1`) or a computed increment (`field: field + <nonzero>`,
/// either operand order).
fn flips_int_from_zero(field: &str, value: &Expr) -> bool {
    match value {
        Expr::Int(n) => *n != 0,
        Expr::Binary {
            op: BinOp::Add,
            lhs,
            rhs,
        } => {
            let refs_field = |e: &Expr| matches!(e, Expr::Var(v) if v == field);
            let nonzero_lit = |e: &Expr| matches!(e, Expr::Int(n) if *n != 0);
            (refs_field(lhs) && nonzero_lit(rhs)) || (nonzero_lit(lhs) && refs_field(rhs))
        }
        _ => false,
    }
}

/// The `(role, entry)` pairs bound by a TERMINAL lifecycle edge (B3). A terminal
/// edge (`... via role.entry terminal;`) ends the lifecycle: the coin is released
/// via `pays(...)` and the spending UTXO is consumed with no successor covenant,
/// so such an entrypoint is inherently a SETTLING transition. Keyed by the
/// ROLE-QUALIFIED pair (not the bare entry name), so a non-terminal `refund` in
/// role B is not spuriously forced to settle just because role A has a terminal
/// `refund` (R4 / same class as the M-1 cross-role binding).
fn terminal_entry_set(app: &App) -> HashSet<(&str, &str)> {
    app.lifecycle
        .iter()
        .filter(|e| e.terminal)
        .map(|e| (e.via_role.as_str(), e.via_entry.as_str()))
        .collect()
}

/// Whether `entry` (declared by role `role_name`) is a `payout_bound`-governed
/// settling transition. A `mode = transition` entrypoint settles when EITHER it is
/// named by a TERMINAL lifecycle edge (B3) — a terminal spend releases the coin and
/// ends the lifecycle, so it settles by construction REGARDLESS of a mint/burn name
/// (R2: a terminal `burn_out` still moves coin and must bind its payout) — OR it is
/// a non-mint/burn entrypoint that [`settles`] via a recognized one-shot-flag flip.
/// `terminal_entries` is the app-scoped [`terminal_entry_set`].
fn is_settling_transition(
    role_name: &str,
    entry: &Entry,
    terminal_entries: &HashSet<(&str, &str)>,
) -> bool {
    if !matches!(entry.mode, CovenantMode::Transition) {
        return false;
    }
    // A TERMINAL transition always releases coin — the supply-change exemption
    // (an authorised supply change releases no coin) applies ONLY to non-terminal
    // transitions.
    if terminal_entries.contains(&(role_name, entry.name.as_str())) {
        return true;
    }
    entry.supply_change.is_none() && settles(entry)
}

/// Count of recognized settling transitions across the whole app (coverage signal
/// for `payout_bound`, surfaced in `explain`). See [`settles`] for the recognized
/// one-shot-flag shapes and [`terminal_entry_set`] for terminal settles (B3).
pub fn settling_transition_count(app: &App) -> usize {
    let terminal_entries = terminal_entry_set(app);
    app.roles
        .iter()
        .flat_map(|role| role.entrypoints.iter().map(move |e| (role, e)))
        .filter(|(role, entry)| is_settling_transition(&role.name, entry, &terminal_entries))
        .count()
}

/// A6 (payout_bound) — NOTE: distinct from the A6-sign adjustment-term guard
/// in C1/`conservation_split`; the two review items share a number, not a rule.
/// `payout_bound` invariant (a `Custom` recognizer, no parser change).
///
/// For an app declaring `invariant payout_bound;`, every recognized settling
/// transition (see [`is_settling_transition`] / [`settles`]) MUST carry at least
/// one `Stmt::Pays { .. }` — a declared-settlement transition binds its payout to
/// a committed output via `pays` (→ `OpTxOutputAmount`/`OpTxOutputSpk`). Delete the
/// `pays` clause and this fires (symmetric with A4-full).
///
/// FAIL-LOUD ON VACUITY (H-1): if `payout_bound` recognizes ZERO settling
/// transitions, the invariant is rejected rather than passing green — a vacuous
/// `payout_bound` (0 matches) must never be indistinguishable from real
/// enforcement. Authors must write the settlement in a recognized one-shot-flag
/// shape or remove the invariant.
///
/// Honest scope: this is an EXISTENCE-ONLY check. It does NOT verify the `pays`
/// binds THIS settlement's own coin or the correct payee — a committed-but-unrelated
/// `pays(...)` satisfies it (payee/amount validity is checked separately by
/// `check_pays`). It is NOT a value-conservation / KIP-9 mass proof — the L1
/// surplus caveat still applies. It does not overload `value_conserved`.
fn check_payout_bound(app: &App, diags: &mut Vec<Diagnostic>) {
    let terminal_entries = terminal_entry_set(app);
    let mut recognized = 0usize;
    for role in &app.roles {
        for entry in &role.entrypoints {
            if !is_settling_transition(&role.name, entry, &terminal_entries) {
                continue;
            }
            recognized += 1;
            let has_pays = entry.body.iter().any(|s| matches!(s, Stmt::Pays { .. }));
            if !has_pays {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `payout_bound` violated: a settling transition (a TERMINAL \
                     release, or one that flips a recognized one-shot flag from its unset to its \
                     set value) must bind its payout to a committed output via `pays(...)`; this \
                     path carries no `pays` clause (payout_bound is EXISTENCE-ONLY — it does not \
                     verify the pays binds THIS settlement's own coin/payee, and it is NOT a \
                     value-conservation/KIP-9 mass proof; the L1 surplus caveat still applies)",
                    role.name, entry.name
                )));
            }
        }
    }
    if recognized == 0 {
        diags.push(Diagnostic::new(
            "invariant `payout_bound` declared but no settling transition was recognized: a \
             settlement must be written as a recognized one-shot flag flip — an int flip \
             (`require f == 0;` + return `f: <nonzero>` or `f: f + <nonzero>`) or a bool flip \
             (`require f == false;` + return `f: true`) — or as a TERMINAL transition (a \
             lifecycle edge marked `terminal`, which releases the coin and ends the lifecycle). \
             Write the settlement in a recognized shape, or remove the invariant (a vacuous \
             payout_bound must not pass green)"
                .to_string(),
        ));
    }
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
                // B2: a `pays(...)` output-binding clause. Its well-formedness
                // (payee/amount committed, index sane, transition-only) is checked
                // in `check_pays` where the role-level committed set is available;
                // the expression pass treats it as an opaque, already-structured
                // statement (no free `Expr` to type here).
                Stmt::Pays { .. } => {}
                // B1: an `after(...)` time-gate clause. Its well-formedness
                // (deadline committed + int-typed + time-named, transition-only) is
                // checked in `check_after`; the expression pass treats it as an
                // opaque, already-structured statement (no free `Expr` to type).
                Stmt::After { .. } => {}
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
//     top-level operator is `+` or `-` (never `*`), the other operand `e`
//     does not itself reference `f`, and (A6-sign) every top-level `+`-atom of `e`
//     is established non-negative BY THIS ENTRYPOINT.
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
// A6-sign (external review): the additive shape alone was still sign-blind. `f: f - e`
// with a NEGATIVE `e` increases `f` — the same money-printing the LOW-1 shapes
// were closed against, reached through the accepted shape instead. Its mirror,
// `f: f + e` with a negative `e`, destroys value. The adjustment term's sign is
// therefore now part of the rule (see `unguarded_additive_atom`). This is a
// per-entrypoint guard requirement, not a range solver: only the term's LOWER
// bound is established; its magnitude is still unconstrained.
//
// This deliberately does NOT reason about cross-field flow (e.g. "amount moved
// from balance to fee" sums to zero) — that needs a solver. It is a structural
// guard against the blunt supply-inflation / value-destruction shapes only.

/// Conventional value-bearing field names (see C1 note). A field is also
/// value-bearing if its declared type is `coin`.
const VALUE_BEARING_NAMES: &[&str] = &["balance", "amount", "supply"];

/// M1 — a conservation invariant declared on a role with NO value-bearing field
/// checks nothing and would report `ok`, which is the worst possible answer: the
/// author believes conservation is enforced when the field set C1 recognizes is
/// empty. C1/`conservation_split` protect a field only when it is typed `coin` or
/// named exactly `balance`/`amount`/`supply` (plus any `*balance` suffix for the
/// split), so e.g. a role whose value field is called `funds` or `principal` is
/// entirely outside their reach. Surface it loudly rather than pass vacuously,
/// naming the field-set rule so the author can rename the field or drop the
/// invariant.
fn vacuous_conservation_warnings(
    role: &Role,
    value_conserved: bool,
    want_conservation_split: bool,
    out: &mut Vec<String>,
) {
    for (declared, name, rule) in [
        (
            value_conserved,
            "value_conserved",
            "typed `coin`, or named exactly `balance`, `amount`, or `supply`",
        ),
        (
            want_conservation_split,
            "conservation_split",
            "typed `coin`, named exactly `balance`, `amount`, or `supply`, or ending in `balance`",
        ),
    ] {
        if !declared {
            continue;
        }
        let any_value_bearing = role.state.iter().any(|f| {
            if name == "conservation_split" {
                is_value_bearing_split(&f.name, &f.ty)
            } else {
                is_value_bearing(&f.name, &f.ty)
            }
        });
        if !any_value_bearing {
            out.push(format!(
                "warning: `{}`: invariant `{}` is declared but NO state field on this role is \
                 value-bearing, so it checks nothing and still reports ok. A field is \
                 value-bearing only if it is {}. Rename the field this invariant is meant to \
                 protect, or drop the invariant — a declared guarantee that matches no field is \
                 an overclaim.",
                role.name, name, rule
            ));
        }
    }
}

fn is_value_bearing(name: &str, ty: &Type) -> bool {
    matches!(ty, Type::Coin) || VALUE_BEARING_NAMES.contains(&name)
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

/// Verdict of the C1 conservation check on one value-bearing field.
enum Conservation<'a> {
    /// A bare carry, or an additive adjustment whose every `+`-atom the
    /// entrypoint establishes non-negative.
    Preserving,
    /// Not a carry / single-additive shape at all (multiplicative, constant,
    /// foreign-only, or the field on both sides).
    BadShape,
    /// A6-sign — the shape is additive, but the named atom of the adjustment term is
    /// never established non-negative, so its sign can invert the adjustment.
    UnguardedTerm(&'a Expr),
}

/// Classify an assignment to value-bearing field `field` (LOW-1 + A6-sign). Exactly
/// two shapes are conservation-preserving:
///
/// 1. bare carry            `field`
/// 2. additive adjustment   `field ± e` / `e ± field`  (top-level op `+`/`-`)
///    where `field` appears on exactly ONE side at the top level, the other
///    operand `e` does not itself reference `field`, and (A6-sign) every `+`-atom of
///    `e` is established non-negative by `entry` — see [`unguarded_additive_atom`].
///
/// Everything else — multiplicative (`*`), constant replacement, foreign vars,
/// or the field on both sides (`field - field`) — is NOT conservation-preserving.
fn is_conservation_preserving<'a>(
    entry: &portrait_syntax::Entry,
    field: &str,
    value: &'a Expr,
) -> Conservation<'a> {
    // Shape 1: bare carry `field`.
    if matches!(value, Expr::Var(name) if name == field) {
        return Conservation::Preserving;
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
            let term = if lhs_is_field && !rhs_is_field && !references_var(rhs, field) {
                Some(rhs.as_ref())
            } else if rhs_is_field && !lhs_is_field && !references_var(lhs, field) {
                Some(lhs.as_ref())
            } else {
                None
            };
            if let Some(term) = term {
                return match unguarded_additive_atom(entry, term) {
                    Some(atom) => Conservation::UnguardedTerm(atom),
                    None => Conservation::Preserving,
                };
            }
        }
    }
    Conservation::BadShape
}

/// The A6-sign diagnostic body shared by the C1 object and scalar return paths: the
/// adjustment term carries an atom whose sign is unconstrained, so it can invert
/// the adjustment.
fn unguarded_term_message(
    role: &Role,
    entry: &Entry,
    field: &str,
    atom: &Expr,
    supply_change_exempt: bool,
) -> String {
    let capability_note = if supply_change_exempt {
        " The `supply_change` capability authorises a SUPPLY CHANGE, not a sign inversion, so it \
         does NOT waive this check."
    } else {
        ""
    };
    format!(
        "`{}.{}`: invariant `value_conserved` violated: value-bearing field `{f}` is adjusted by a \
         term containing `{}`, whose sign this entrypoint never establishes — a negative value \
         INVERTS the adjustment (`{f} - e` inflates the field; `{f} + e` drains it). Every \
         `+`-atom of the adjustment must be a non-negative int literal or a name guarded HERE by \
         `requires <name> >= 0;` (a genesis-committed field does not qualify — genesis can commit \
         a negative).{}",
        role.name,
        entry.name,
        atom.to_silverscript(),
        capability_note,
        f = field,
    )
}

/// Identifiers the Engraver injects into every emitted contract signature
/// (`contract C(int max_ins, int max_outs, …)`) to carry the covenant's own
/// input/output bound.
const RESERVED_EMITTER_NAMES: &[&str] = &["max_ins", "max_outs"];

/// M3/M4 — role param and state-field NAMES must be unambiguous and must not
/// collide with the emitter's injected identifiers.
///
/// A param named `max_ins`/`max_outs` is emitted verbatim into the contract
/// signature ALONGSIDE the injected bound of the same name. silverc accepts the
/// duplicate and the user's param wins, which makes the covenant's output-count
/// bound deployer-controlled — a covenant that reads as `to = 1` but is not. A
/// duplicate param name is the M4 case: genesis binding is by name, so the
/// second declaration is unreachable and a `pubkey balance` shadowing an
/// `int balance` produced a misleading type-mismatch diagnostic downstream.
fn check_reserved_and_duplicate_names(role: &Role, diags: &mut Vec<Diagnostic>) {
    for p in &role.params {
        if RESERVED_EMITTER_NAMES.contains(&p.name.as_str()) {
            diags.push(Diagnostic::new(format!(
                "`{}`: role param `{}` is a RESERVED emitter identifier — the Engraver injects \
                 `int {}` into every contract signature as the covenant's own input/output bound. \
                 A param of that name shadows it and hands control of the bound to the deployer. \
                 Rename the param.",
                role.name, p.name, p.name
            )));
        }
    }
    for f in &role.state {
        if RESERVED_EMITTER_NAMES.contains(&f.name.as_str()) {
            diags.push(Diagnostic::new(format!(
                "`{}`: state field `{}` is a RESERVED emitter identifier — the Engraver injects \
                 `int {}` into every contract signature as the covenant's own input/output bound. \
                 Rename the field.",
                role.name, f.name, f.name
            )));
        }
    }
    for (i, p) in role.params.iter().enumerate() {
        if role.params[..i].iter().any(|prior| prior.name == p.name) {
            diags.push(Diagnostic::new(format!(
                "`{}`: role param `{}` is declared more than once. Genesis binding is by name, so \
                 a duplicate makes the initialiser for `{}` ambiguous and the second declaration \
                 unreachable; remove or rename it.",
                role.name, p.name, p.name
            )));
        }
    }
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
        // M2: the explicit checked supply-change capability waives the
        // CONSERVATION SHAPE (a real mint does not conserve) — it does NOT waive
        // the A6-sign check. Authorising a supply change is not authorising a
        // sign inversion: `balance: balance - fee` with an unguarded `fee` is a
        // covert increase whichever capability the entry declares.
        let conservation_exempt = entry.supply_change.is_some();
        for stmt in &entry.body {
            match stmt {
                Stmt::Return(ReturnExpr::Object { fields, .. }) => {
                    for (field, value) in fields {
                        let Some(ty) = state.get(field.as_str()) else {
                            continue; // unknown field already reported by the type pass
                        };
                        if !is_value_bearing(field, ty) {
                            continue;
                        }
                        match is_conservation_preserving(entry, field, value) {
                            Conservation::Preserving => {}
                            Conservation::UnguardedTerm(atom) => {
                                diags.push(Diagnostic::new(unguarded_term_message(
                                    role,
                                    entry,
                                    field,
                                    atom,
                                    conservation_exempt,
                                )));
                            }
                            Conservation::BadShape if conservation_exempt => {}
                            Conservation::BadShape => {
                                diags.push(Diagnostic::new(format!(
                                    "`{}.{}`: invariant `value_conserved` violated: value-bearing \
                                 field `{f}` is assigned `{}` which is not conservation-preserving \
                                 (must be a bare carry `{f}: {f}` or a single additive adjustment \
                                 `{f}: {f} + e` / `{f}: {f} - e`; multiplicative, constant, or \
                                 double-self forms create or destroy value); if intentional, \
                                 declare the entry `#[covenant(mode = transition, supply_change = \
                                 <authority>)]` (a checked-model capability whose named committed \
                                 authority must be guaranteed to sign)",
                                    role.name,
                                    entry.name,
                                    value.to_silverscript(),
                                    f = field,
                                )));
                            }
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
                        if !is_value_bearing(field, ty) {
                            continue;
                        }
                        match is_conservation_preserving(entry, field, expr) {
                            Conservation::Preserving => {}
                            Conservation::UnguardedTerm(atom) => {
                                diags.push(Diagnostic::new(unguarded_term_message(
                                    role,
                                    entry,
                                    field,
                                    atom,
                                    conservation_exempt,
                                )));
                            }
                            Conservation::BadShape if conservation_exempt => {}
                            Conservation::BadShape => {
                                diags.push(Diagnostic::new(format!(
                                    "`{}.{}`: invariant `value_conserved` violated: scalar return \
                                 assigns value-bearing field `{f}` via expression `{}` which is \
                                 not conservation-preserving (must be a bare carry `{f}` or a \
                                 single additive adjustment `{f} + e` / `{f} - e`; multiplicative, \
                                 constant, or double-self forms create or destroy value); if \
                                 intentional, declare the entry `#[covenant(mode = transition, \
                                 supply_change = <authority>)]` (a checked-model capability whose \
                                 named committed authority must be guaranteed to sign)",
                                    role.name,
                                    entry.name,
                                    expr.to_silverscript(),
                                    f = field,
                                )));
                            }
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
// `prev_states[i].field` access.
//
// GUARD-AWARE (external review A1/A3): it is NOT enough for a committed key to
// appear *somewhere* in the require guards. A disjunction
// (`checkSig(owner) || checkSig(attacker)`), a negation (`!checkSig(auth,
// owner)`), or a caller-supplied pubkey each leaves a SATISFYING PATH on which no
// committed key signs — so the earlier "does any committed key appear" test was
// bypassable. The check now computes, via `guaranteed_committed_signers`, a sound
// LOWER BOUND on the number of DISTINCT committed keys that MUST sign on EVERY
// satisfying assignment of the guards, and requires it to be `>= 1`. A `||`
// contributes only what its weakest arm forces, a `&&` unions its operands, a
// negated checkSig contributes nothing, and a caller-supplied pubkey contributes
// nothing — so all three bypass shapes are rejected while the shipped disjunctive
// 2-of-3 (ArbiterEscrow) and dual-require 2-of-2 (MultisigTreasury) still pass.
//
// LOW-2 (Phase C red-team) — no-checkSig state mutation. A state-mutating
// transition with ZERO authorization was previously accepted silently (deemed
// "out of capability scope"). The honest, conservative fail-safe: when the app
// declares a stake in correctness that authorization protects — i.e.
// `value_conserved` is declared, or a custom `authorized` invariant is declared
// — a state-mutating transition with NO checkSig at all is REJECTED with a clear
// message. The author must add a committed-key checkSig.
//
// A2 (external review) — the no-auth fail-safe no longer exempts mint/burn.
// Conservation and authorization are decoupled: a mint/burn is exempt from the
// C1 *conservation* shape (a real mint legitimately does not conserve supply),
// but a supply change with ZERO authorization is a distinct hole and is rejected
// under a declared protection invariant just like any other state mutation. A
// genuine mint therefore carries its own committed-key `checkSig(auth, issuer)`.
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
    // Committed names: role params + state fields (both genesis-baked).
    let committed = committed_keys(role);

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
        if !entry_has_checksig(entry) {
            // LOW-2 fail-safe: a state-mutating transition with NO authorization
            // at all. Reject when the app declared a protection invariant
            // (`value_conserved` or custom `authorized`). A2: mint/burn is NO
            // LONGER an opt-out — a supply change with zero authorization is a
            // hole regardless of whether conservation is exempt.
            if require_auth {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: capability check failed: state-mutating transition has NO \
                     authorization (no checkSig) under a declared protection invariant \
                     (`value_conserved`/`authorized`); add a checkSig binding a committed key \
                     (role param, state field, or prev_states[..])",
                    role.name, entry.name
                )));
            }
            continue; // no checkSig target to judge below
        }
        // Guard-aware bound: the number of DISTINCT committed keys guaranteed to
        // sign on EVERY satisfying path must be >= 1. A disjunctive `||` bypass, a
        // negated checkSig, or a caller-supplied pubkey all leave a satisfying
        // path with zero committed signers → guaranteed == 0 → rejected.
        if guaranteed_committed_signers(entry, &committed) == 0 {
            diags.push(Diagnostic::new(format!(
                "`{}.{}`: capability check failed: no committed key is guaranteed to sign on \
                 every satisfying path — a disjunctive `||`, a negated checkSig, or a \
                 caller-supplied pubkey leaves a path with no committed signature; every \
                 satisfying assignment must force at least one checkSig against a committed key \
                 (a role param, state field, or prev_states[..] field)",
                role.name, entry.name
            )));
        }
    }
}

/// A2-full — the explicit checked-model supply-change capability.
///
/// An entry declared `#[covenant(mode = transition, supply_change = A)]` waives
/// itself from value-conservation (C1 / `conservation_split`) checking. That
/// waiver is a CHECKED-MODEL capability, not a name convention and NOT an
/// on-chain minted-supply guarantee — a UTXO covenant cannot inflate real coin.
/// To earn the waiver the named authority `A` must be:
///   (a) a COMMITTED key (role param or state field — genesis-baked);
///   (b) GUARANTEED to sign on EVERY satisfying assignment of the guards
///       ([`authority_guaranteed`]) — a disjunctive or negated arm leaves a
///       satisfying path on which `A` does not sign, so it cannot guarantee `A`;
///       and
///   (c) release NO coin — a supply change adjusts committed supply only, so it
///       must NOT carry a `pays(...)` clause and must NOT be a terminal
///       (coin-releasing) spend. This makes the "releases no coin" premise that
///       excludes supply-change entries from `payout_bound` a CHECKED fact, not
///       an unverified comment (RT-2).
/// This runs UNCONDITIONALLY (no invariant need be declared): the annotation
/// itself is the request for the exemption, so its precondition is always
/// enforced.
fn check_supply_change(
    role: &Role,
    terminal_entries: &HashSet<(&str, &str)>,
    diags: &mut Vec<Diagnostic>,
) {
    let committed = committed_keys(role);
    for entry in &role.entrypoints {
        let Some(authority) = entry.supply_change.as_deref() else {
            continue;
        };
        // (a) the authority must be a committed key.
        if !committed.contains(authority) {
            diags.push(Diagnostic::new(format!(
                "`{}.{}`: supply-change capability invalid: authority `{authority}` is not a \
                 committed key (must be a role param or state field, genesis-baked into the \
                 covenant); a caller-supplied or spender-arg authority cannot gate a supply change",
                role.name, entry.name
            )));
            continue;
        }
        // (b) the authority must be guaranteed to sign on every satisfying path.
        // A per-authority MEMBERSHIP question — NOT the cardinality lower-bound
        // `guaranteed_committed_signers` answers (whose `Or` case returns the
        // smaller arm's SET, which would unsoundly admit an authority sitting in
        // a nested `||` under an `&&`). [`authority_guaranteed`] is the sound,
        // commutative per-key predicate.
        let guaranteed = entry.body.iter().any(|s| match s {
            Stmt::Require(e) => authority_guaranteed(e, authority, &committed),
            _ => false,
        });
        if !guaranteed {
            diags.push(Diagnostic::new(format!(
                "`{}.{}`: supply-change capability invalid: authority `{authority}` is not \
                 guaranteed to sign — a supply change must force `checkSig(_, {authority})` on \
                 every satisfying path (a mandatory `&&` require against the committed key), not \
                 in a disjunctive `||` or negated arm that leaves a path where `{authority}` does \
                 not sign",
                role.name, entry.name
            )));
        }
        // (c) a supply change must release no coin (RT-2): no `pays(...)` clause
        // and not a terminal spend.
        let has_pays = entry.body.iter().any(|s| matches!(s, Stmt::Pays { .. }));
        let is_terminal = terminal_entries.contains(&(role.name.as_str(), entry.name.as_str()));
        if has_pays || is_terminal {
            diags.push(Diagnostic::new(format!(
                "`{}.{}`: supply-change capability invalid: a supply change adjusts committed \
                 supply and must not release coin (no `pays(...)`, not a terminal spend); model a \
                 payout as a separate transition",
                role.name, entry.name
            )));
        }
    }
}

/// Sound, COMMUTATIVE test that positive `checkSig(_, authority)` fires on EVERY
/// satisfying assignment of guard `e` (A2-full RT-1). This is a per-KEY MEMBERSHIP
/// question, distinct from the per-COUNT lower bound [`guaranteed_committed_signers`]
/// computes for A1/A3 — do not conflate them:
///   * `And` → EITHER operand forcing the authority suffices (both must hold);
///   * `Or`  → BOTH operands must force it (the spender may take either arm);
///   * a positive `checkSig(_, pk)` → `pk` is committed AND names the authority;
///   * `Not`, comparisons, other calls / args → `false` (nothing forced).
fn authority_guaranteed(e: &Expr, authority: &str, committed: &HashSet<String>) -> bool {
    match e {
        Expr::Binary {
            op: BinOp::And,
            lhs,
            rhs,
        } => {
            authority_guaranteed(lhs, authority, committed)
                || authority_guaranteed(rhs, authority, committed)
        }
        Expr::Binary {
            op: BinOp::Or,
            lhs,
            rhs,
        } => {
            authority_guaranteed(lhs, authority, committed)
                && authority_guaranteed(rhs, authority, committed)
        }
        Expr::Call { name, args } if name == "checkSig" && args.len() == 2 => {
            pubkey_is_committed(&args[1], committed) && committed_key_name(&args[1]) == authority
        }
        _ => false,
    }
}

/// True if any `require` guard in the entrypoint contains a `checkSig(sig,
/// pubkey)` call anywhere in its tree.
fn entry_has_checksig(entry: &portrait_syntax::Entry) -> bool {
    entry.body.iter().any(|s| match s {
        Stmt::Require(e) => expr_contains_checksig(e),
        _ => false,
    })
}

/// True if `checkSig(sig, pubkey)` appears anywhere in the expression tree.
fn expr_contains_checksig(e: &Expr) -> bool {
    match e {
        Expr::Call { name, args } if name == "checkSig" && args.len() == 2 => true,
        Expr::Call { args, .. } => args.iter().any(expr_contains_checksig),
        Expr::Unary { rhs, .. } => expr_contains_checksig(rhs),
        Expr::Binary { lhs, rhs, .. } => expr_contains_checksig(lhs) || expr_contains_checksig(rhs),
        Expr::Field { base, .. } => expr_contains_checksig(base),
        Expr::Index { base, index } => {
            expr_contains_checksig(base) || expr_contains_checksig(index)
        }
        Expr::Int(_) | Expr::Bool(_) | Expr::Bytes(_) | Expr::Var(_) => false,
    }
}

/// A sound LOWER BOUND on the number of DISTINCT committed pubkeys guaranteed to
/// sign on EVERY satisfying assignment of the entrypoint's `require` guards
/// (external review A1/A3). Shared by C2 (`>= 1`) and `multisig_threshold`
/// (`>= 2`).
///
/// Aggregation: mandatory guards (top-level not `||`) contribute by SET UNION;
/// each disjunctive guard (top-level `||`) contributes its own lower bound, and
/// we take the strongest single one. The scalar is
/// `max(|mandatory ∪|, max over ||-guards of |guard_committed_keyset|)`.
fn guaranteed_committed_signers(
    entry: &portrait_syntax::Entry,
    committed: &HashSet<String>,
) -> usize {
    let mut mandatory: HashSet<String> = HashSet::new();
    let mut disj_max = 0usize;
    for stmt in &entry.body {
        if let Stmt::Require(e) = stmt {
            if matches!(e, Expr::Binary { op: BinOp::Or, .. }) {
                disj_max = disj_max.max(guard_committed_keyset(e, committed).len());
            } else {
                mandatory.extend(guard_committed_keyset(e, committed));
            }
        }
    }
    mandatory.len().max(disj_max)
}

/// The committed pubkey names a single guard is GUARANTEED to force a signature
/// against. The CARDINALITY of the returned set is a sound lower bound on the
/// distinct committed keys that must sign to satisfy the guard:
///
/// * `checkSig(_, pk)` with `pk` a committed key → `{pk}` (that key must sign).
/// * `Binary{And, l, r}` → UNION (both operands must be satisfied).
/// * `Binary{Or, l, r}` → the weaker arm by CARDINALITY (the spender may pick
///   either arm, so only the arm that forces the FEWER committed keys is
///   guaranteed). This branch-min is the tightest sound lower bound on the COUNT;
///   a plain set INTERSECTION would wrongly report zero for the shipped
///   disjunctive 2-of-3 (ArbiterEscrow), whose arms each force two keys but share
///   none across all three — so we take the smaller arm set, not the intersection.
/// * `Unary{Not, ..}` → empty (a negated signature check authorizes nothing).
/// * comparisons / other calls / non-committed or caller-supplied pubkeys → empty
///   (nothing committed is forced).
fn guard_committed_keyset(e: &Expr, committed: &HashSet<String>) -> HashSet<String> {
    match e {
        Expr::Binary {
            op: BinOp::Or,
            lhs,
            rhs,
        } => {
            let l = guard_committed_keyset(lhs, committed);
            let r = guard_committed_keyset(rhs, committed);
            if l.len() <= r.len() {
                l
            } else {
                r
            }
        }
        Expr::Binary {
            op: BinOp::And,
            lhs,
            rhs,
        } => {
            let mut set = guard_committed_keyset(lhs, committed);
            set.extend(guard_committed_keyset(rhs, committed));
            set
        }
        Expr::Call { name, args } if name == "checkSig" && args.len() == 2 => {
            let mut set = HashSet::new();
            if pubkey_is_committed(&args[1], committed) {
                set.insert(committed_key_name(&args[1]));
            }
            set
        }
        _ => HashSet::new(),
    }
}

/// A canonical distinct name for a committed checkSig pubkey operand. A
/// `prev_states[i].<field>` self-loop reference resolves to the SAME
/// genesis-committed state field as the bare `<field>` (in a self-loop covenant
/// the prior state's `signer` *is* the current committed `signer`), so both
/// canonicalize to the bare field name and DEDUPE (M-1: closes the key-aliasing
/// hole where `checkSig(a, signer) && checkSig(b, prev_states[0].signer)` counted
/// as two distinct keys but is really 1-of-1). Legitimate multisigs bind DISTINCT
/// fields (`signer_a` ≠ `signer_b`), so they are unaffected.
fn committed_key_name(pk: &Expr) -> String {
    match pk {
        Expr::Var(n) => n.clone(),
        Expr::Field { field, .. } => field.clone(),
        _ => String::new(),
    }
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
//                                    release-style transition must have >= 2
//                                    DISTINCT committed keys GUARANTEED to sign on
//                                    every satisfying path (via
//                                    `guaranteed_committed_signers`, the same
//                                    guard-aware bound C2 uses). This JUDGES the
//                                    boolean combination: a `&&` unions its
//                                    operands' committed keys, a `||` counts only
//                                    what its weakest arm forces, a negated
//                                    checkSig and a caller-supplied pubkey count
//                                    nothing. So ArbiterEscrow's 2-of-3
//                                    disjunction (every arm binds two committed
//                                    keys → 2) and MultisigTreasury's dual-require
//                                    2-of-2 (→ 2) pass, while a 1-of-n `||` (each
//                                    arm one key → 1) is rejected. It is a
//                                    STRUCTURAL lower bound on distinct committed
//                                    signers, not an SMT proof of k-of-n over
//                                    arbitrary predicates.
//   invariant temporal_guard;      — when declared, the role must have AT LEAST
//                                    ONE state-mutating transition that asserts a
//                                    committed-time gate `require now_bucket >=
//                                    <committed time expr>` — a committed deadline
//                                    field (`now_bucket >= deadline`) or a
//                                    committed `last_active + timeout` window
//                                    (`now_bucket >= last_active + timeout`).
//                                    Models HTLC.refund and DeadMansSwitch.claim.
//                                    This role-level EXISTENCE check (A4) means
//                                    deleting the only time comparison is
//                                    rejected, not passed vacuously. `now_bucket`
//                                    is caller-asserted and coarse; this is a
//                                    shape match that guarantees a committed-time
//                                    gate EXISTS on a mutating transition — it
//                                    does NOT yet bind a specific value-moving
//                                    path to a deadline (that needs a
//                                    formula-bearing invariant — tracked
//                                    follow-up), and is NOT a wall-clock proof.
//                                    SCOPE LABEL: this is WALLET-ASSUMED time —
//                                    `now_bucket` is a spender-supplied argument,
//                                    NOT consensus time; the emitted `.sil` reads
//                                    no wall-clock and emits no CLTV/sequence
//                                    timelock. The name is kept (label-now,
//                                    rename-later); true per-pattern scope is the
//                                    enforcement matrix at `library/ENFORCEMENT.md`.

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
            if has_amount_arg && !asserts_non_negative(entry, "amount") {
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
            if mutates && guaranteed_committed_signers(entry, committed) < 2 {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `multisig_threshold` violated: fewer than 2 distinct \
                     committed keys are GUARANTEED to sign on every satisfying path (a `||` arm \
                     that forces only one key, a negated checkSig, or a caller-supplied pubkey \
                     drops the threshold); expected at least two distinct committed keys forced \
                     on every path, e.g. `checkSig(a, signer_a) && checkSig(b, signer_b)` or a \
                     disjunction whose every arm binds two committed keys",
                    role.name, entry.name
                )));
            }
        }
    }

    if want_temporal_guard {
        // A4 (external review) — role-level EXISTENCE tightening. The previous
        // per-entrypoint check only inspected transitions that ALREADY read
        // `now_bucket` in a guard, so deleting the only time comparison removed
        // the thing being inspected and the invariant passed vacuously. Now a role
        // declaring `temporal_guard` must have AT LEAST ONE state-mutating
        // transition that asserts a committed-time gate (`now_bucket >= <committed
        // deadline>` or `now_bucket >= last_active + timeout`). Deleting or
        // weakening the only such gate → no mutating transition gates on committed
        // time → rejected.
        //
        // HONEST LIMIT: this guarantees a committed-time gate EXISTS on a mutating
        // transition; it does NOT yet bind a specific value-moving path to a
        // deadline (that needs a formula-bearing invariant — tracked follow-up).
        let state_field_names: Vec<&str> = role.state.iter().map(|f| f.name.as_str()).collect();
        let time_atoms = time_committed_atoms(role);
        let any_committed_gate = role.entrypoints.iter().any(|entry| {
            if !matches!(entry.mode, CovenantMode::Transition) {
                return false;
            }
            let mutates = entry.body.iter().any(|s| match s {
                Stmt::Return(ReturnExpr::Object { .. }) => true,
                Stmt::Return(ReturnExpr::Scalar(expr)) => {
                    state_field_names.iter().any(|f| references_var(expr, f))
                }
                _ => false,
            });
            mutates && asserts_temporal_gate(entry, &time_atoms)
        });
        if !any_committed_gate {
            diags.push(Diagnostic::new(format!(
                "role `{}`: invariant `temporal_guard` violated: no state-mutating transition \
                 asserts a committed-time gate (expected at least one transition to \
                 `require now_bucket >= <committed deadline>` or \
                 `require now_bucket >= last_active + timeout`)",
                role.name
            )));
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
        // M2 (as in C1): the capability waives the CANCELLATION requirement, not
        // the A6-sign check on each leg's term.
        let conservation_exempt = entry.supply_change.is_some();
        for stmt in &entry.body {
            let Stmt::Return(ReturnExpr::Object { fields, .. }) = stmt else {
                continue;
            };
            // Per-field deltas across every value-bearing field in the return.
            let mut increase_terms: Vec<&Expr> = Vec::new();
            let mut decrease_terms: Vec<&Expr> = Vec::new();
            let mut others: Vec<&str> = Vec::new();
            // Every moved leg as (field, adjustment term) — the A6-sign check
            // below inspects both directions, since the two legs cancel and a
            // negative term reverses the whole transfer.
            let mut adjusted: Vec<(&str, &Expr)> = Vec::new();
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
                        adjusted.push((field, term));
                        moved_fields += 1;
                    }
                    SplitAdjust::Increase(term) => {
                        increase_terms.push(term);
                        adjusted.push((field, term));
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
            if let Some(bad) = others.first().filter(|_| !conservation_exempt) {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `conservation_split` violated: value-bearing field `{}` \
                     changes in a non-additive shape (expected each value-bearing field to carry \
                     `f: f`, increase `f: f + e`, or decrease `f: f - e`; multiplicative, constant, \
                     or double-self forms are not analyzable as a value delta)",
                    role.name, entry.name, bad
                )));
                continue;
            }
            // A6-sign: a leg whose adjustment term has an unconstrained sign reverses
            // the split when that term is negative — the "source" leg gains and
            // the "destination" leg is drained. Structural cancellation cannot
            // see this: the same term appears on both sides either way.
            if let Some((field, atom)) = adjusted
                .iter()
                .find_map(|(f, t)| unguarded_additive_atom(entry, t).map(|a| (*f, a)))
            {
                diags.push(Diagnostic::new(format!(
                    "`{}.{}`: invariant `conservation_split` violated: the leg on value-bearing \
                     field `{}` is adjusted by a term containing `{}`, whose sign this entrypoint \
                     never establishes — a negative value REVERSES the transfer (the source leg \
                     gains and the destination leg is drained). Every `+`-atom of each leg's term \
                     must be a non-negative int literal or a name guarded HERE by \
                     `requires <name> >= 0;` (a genesis-committed field does not qualify — genesis \
                     can commit a negative).{}",
                    role.name,
                    entry.name,
                    field,
                    atom.to_silverscript(),
                    if conservation_exempt {
                        " The `supply_change` capability authorises a SUPPLY CHANGE, not a sign \
                         inversion, so it does NOT waive this check."
                    } else {
                        ""
                    }
                )));
                continue;
            }
            // Net-zero requirement: the value moved must stay INSIDE the covenant
            // — at least one field decreases AND at least one field increases. A
            // lone decrease (drain to an external output) or lone increase (mint)
            // is NOT an internal split; that is a `value_conserved` spend shape,
            // not a `conservation_split` transfer.
            if conservation_exempt {
                continue; // capability waives the CANCELLATION arithmetic below
            }
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

/// Committed field names that may serve as a `temporal_guard` deadline atom: an
/// int-typed committed field carrying a conventional TIME name. L-3 restricts the
/// gate to these so a non-time committed field (`balance`, `owner`, …) cannot
/// masquerade as a deadline (`now_bucket >= balance` is not a time gate).
const TIME_FIELD_NAMES: &[&str] = &[
    "deadline",
    "cliff",
    "timeout",
    "period",
    "last_paid",
    "last_charged",
    "last_active",
    "unlock_bucket",
];

/// ANCHOR time names — absolute points in time (a committed deadline / cliff / last
/// event bucket). A single-field `after(anchor)` is a complete gate; in a `Sum`
/// window it is the base the duration is added to. This list PARTITIONS
/// `TIME_FIELD_NAMES` with `TIME_DURATION_NAMES` (every name is in exactly one).
const TIME_ANCHOR_NAMES: &[&str] = &[
    "deadline",
    "cliff",
    "unlock_bucket",
    "last_active",
    "last_charged",
    "last_paid",
];

/// DURATION time names — intervals (a committed window length). A duration is only
/// a valid `after(...)` threshold when ADDED to an anchor (`anchor + duration`); on
/// its own it is a tiny relative value, not an absolute deadline. Partition
/// complement of `TIME_ANCHOR_NAMES` within `TIME_FIELD_NAMES`.
const TIME_DURATION_NAMES: &[&str] = &["period", "timeout"];

/// The committed names (role param or state field) that are BOTH int-typed AND
/// carry a conventional time-field name — the only atoms a `temporal_guard` gate
/// may compare `now_bucket` against (L-3).
fn time_committed_atoms(role: &Role) -> HashSet<String> {
    let mut set = HashSet::new();
    for p in &role.params {
        if p.ty == Type::Int && TIME_FIELD_NAMES.contains(&p.name.as_str()) {
            set.insert(p.name.clone());
        }
    }
    for f in &role.state {
        if f.ty == Type::Int && TIME_FIELD_NAMES.contains(&f.name.as_str()) {
            set.insert(f.name.clone());
        }
    }
    set
}

/// True if the entrypoint gates on a committed time: a require of the form
/// `now_bucket >= <committed time expr>`, where the RHS is a committed TIME field
/// (a bare committed time name, e.g. `deadline`, or `prev_states[i].deadline`) or
/// a committed window sum `last_active + timeout` (committed time names, either
/// operand order). `time_atoms` is the int-typed, time-named committed set
/// (L-3). Structural shape match — not a wall-clock proof.
fn asserts_temporal_gate(entry: &portrait_syntax::Entry, time_atoms: &HashSet<String>) -> bool {
    let is_now_bucket = |e: &Expr| matches!(e, Expr::Var(n) if n == "now_bucket");
    // A committed deadline-like atom: a bare committed TIME name (int-typed param
    // / state field on the time allowlist), or a `prev_states[i].<time field>`
    // access. A caller-supplied arg — or a non-time committed field — does not
    // count.
    let is_committed_time_atom = |e: &Expr| -> bool {
        match e {
            Expr::Var(n) => time_atoms.contains(n),
            Expr::Field { base, field } => {
                time_atoms.contains(field)
                    && matches!(
                        base.as_ref(),
                        Expr::Index { base: arr, .. }
                            if matches!(arr.as_ref(), Expr::Var(n) if n == "prev_states")
                    )
            }
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
        // B1 (ADDITIVE): a consensus `after(<committed deadline>)` clause is a
        // strictly STRONGER temporal gate than the caller-asserted `now_bucket`
        // comparison — it lowers to OpCheckLockTimeVerify, which consensus
        // enforces — so it likewise satisfies `temporal_guard`. The `now_bucket`
        // acceptance below is retained unchanged so un-migrated patterns stay green.
        if let Stmt::After { deadline } = stmt {
            let satisfied = match deadline {
                AfterDeadline::Field(f) => time_atoms.contains(f),
                AfterDeadline::Sum(a, b) => time_atoms.contains(a) && time_atoms.contains(b),
            };
            if satisfied {
                return true;
            }
        }
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

/// True if the entrypoint bounds the variable `name` non-negative: a require of
/// the form `name >= <int>=0` or `name > <int>=-1`. The guard must live in THIS
/// entrypoint — a value merely committed at genesis does not qualify, because
/// genesis can commit a negative.
fn asserts_non_negative(entry: &portrait_syntax::Entry, name: &str) -> bool {
    let is_name = |e: &Expr| matches!(e, Expr::Var(n) if n == name);
    for stmt in &entry.body {
        if let Stmt::Require(Expr::Binary { op, lhs, rhs }) = stmt {
            // L1: `-1` parses as `Unary{Neg, Int(1)}`, never `Int(-1)`, so the
            // documented `> -1` form was unreachable until the negation was
            // folded (see `int_literal`).
            // L2: accept the mirrored operand order (`0 <= fee`) too — the same
            // guard, written the other way round.
            match op {
                BinOp::Ge if is_name(lhs) => {
                    if matches!(int_literal(rhs), Some(n) if n >= 0) {
                        return true;
                    }
                }
                BinOp::Gt if is_name(lhs) => {
                    if matches!(int_literal(rhs), Some(n) if n >= -1) {
                        return true;
                    }
                }
                BinOp::Le if is_name(rhs) => {
                    if matches!(int_literal(lhs), Some(n) if n >= 0) {
                        return true;
                    }
                }
                BinOp::Lt if is_name(rhs) => {
                    if matches!(int_literal(lhs), Some(n) if n >= -1) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// The value of `expr` as an int literal, folding a unary negation
/// (`Unary{Neg, Int(n)}` → `-n`) — the shape the parser produces for `-1`.
fn int_literal(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Int(n) => Some(*n),
        Expr::Unary { op: UnOp::Neg, rhs } => match rhs.as_ref() {
            Expr::Int(n) => Some(-*n),
            _ => None,
        },
        _ => None,
    }
}

/// A6-sign — the first `+`-atom of an additive adjustment term that `entry` does NOT
/// establish to be non-negative, or `None` when every atom is established.
///
/// A conservation-preserving `f: f ± e` is only actually conserving when `e` is
/// non-negative. With `e` negative the operator inverts: `f - e` INCREASES `f`
/// (model money-printing) and `f + e` DECREASES it (value destruction, and under
/// `conservation_split` a REVERSE transfer that drains the destination leg). The
/// term is decomposed into its top-level `+`-atoms (so `- (s + m + j)` requires
/// each of `s`, `m`, `j` to be established), and an atom qualifies only if it is
/// a non-negative int literal or a name this same entrypoint guards with
/// `requires <name> >= 0` / `> -1`. A merely COMMITTED field does not qualify —
/// genesis can commit a negative.
fn unguarded_additive_atom<'a>(entry: &portrait_syntax::Entry, term: &'a Expr) -> Option<&'a Expr> {
    let mut atoms: Vec<&Expr> = Vec::new();
    flatten_add_atoms(term, &mut atoms);
    atoms
        .into_iter()
        .find(|atom| !atom_is_established_non_negative(entry, atom))
}

fn atom_is_established_non_negative(entry: &portrait_syntax::Entry, atom: &Expr) -> bool {
    match atom {
        Expr::Int(n) => *n >= 0,
        Expr::Var(name) => asserts_non_negative(entry, name),
        _ => false,
    }
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

/// Non-fatal warnings about a program that PASSES `check` — properties that are
/// not violations but that an author must not be allowed to mistake for a
/// guarantee. Currently: a declared conservation invariant that matches no
/// value-bearing field (M1), which would otherwise report `ok` while checking
/// nothing.
///
/// Deliberately separate from [`check`]'s diagnostics: these do not reject. Some
/// shipped patterns (`SimpleEscrow`, `EvidenceLineage`, `TimeVault`, `Htlc`,
/// `DigitalReit`) declare `value_conserved` with no
/// value-bearing field — partly to opt into the C2 no-auth fail-safe, which keys
/// off the same invariant — so rejecting would silently strip their
/// authorization requirement. The honest move is to say so out loud, not to pass
/// in silence.
pub fn warnings(program: &Program) -> Vec<String> {
    let app = &program.app;
    let value_conserved = app
        .invariants
        .iter()
        .any(|inv| matches!(inv, Invariant::ValueConserved));
    let want_conservation_split = app
        .invariants
        .iter()
        .any(|inv| matches!(inv, Invariant::Custom(s) if s == "conservation_split"));
    let mut out = Vec::new();
    for role in &app.roles {
        vacuous_conservation_warnings(role, value_conserved, want_conservation_split, &mut out);
    }
    out
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

    // ---- B2: pays(...) output-binding validation --------------------------

    /// Build an Escrow-shaped source with a single `release` transition whose
    /// body contains `pays({index}, {payee}, {amount});`.
    fn pays_src(index: &str, payee: &str, amount: &str) -> String {
        format!(
            r#"
pragma portrait ^0.1.0;
app Escrow {{
  role escrow {{
    param pubkey seller;
    param coin   amount;
    state {{ pubkey seller; coin amount; }}
    #[covenant(mode = transition)]
    entrypoint function release(sig auth, pubkey to) : (pubkey seller, coin amount) {{
      requires checkSig(auth, seller);
      pays({index}, {payee}, {amount});
      return Escrow {{ seller: seller, amount: amount }};
    }}
  }}
  lifecycle {{ live -> live via escrow.release; }}
  invariant value_conserved;
}}
"#
        )
    }

    #[test]
    fn pays_accepts_committed_payee_and_amount() {
        let program = parse(&pays_src("0", "seller", "amount")).expect("parse");
        assert!(
            check(&program).is_ok(),
            "committed payee + coin amount should pass: {:?}",
            check(&program).err()
        );
    }

    #[test]
    fn pays_rejects_spender_supplied_payee() {
        // `to` is an entrypoint arg (spender-supplied), not committed state.
        let program = parse(&pays_src("0", "to", "amount")).expect("parse");
        let errs = check(&program).expect_err("spender-supplied payee must reject");
        assert!(
            errs.iter().any(|d| d.message.contains("payee `to`")
                && d.message.contains("spender-supplied argument")),
            "diagnostic must name the spender-supplied payee: {errs:?}"
        );
    }

    #[test]
    fn pays_rejects_non_value_bearing_amount() {
        // `seller` is committed but a pubkey, not value-bearing.
        let program = parse(&pays_src("0", "seller", "seller")).expect("parse");
        let errs = check(&program).expect_err("non-value-bearing amount must reject");
        assert!(
            errs.iter()
                .any(|d| d.message.contains("amount `seller`")
                    && d.message.contains("value-bearing")),
            "diagnostic must name the non-value-bearing amount: {errs:?}"
        );
    }

    /// A Subscription-shaped source: a committed `int fee` paid out by
    /// `pays(1, provider, fee)`, with the entrypoint's guards and the successor's
    /// `balance` expression left open so a test can vary the drawdown link.
    fn drawdown_pays_src(guards: &str, balance_expr: &str) -> String {
        format!(
            r#"
pragma portrait ^0.1.0;
app Sub {{
  role sub {{
    param pubkey provider;
    param int    fee;
    param int    balance;
    state {{ pubkey provider; int fee; int balance; }}
    #[covenant(mode = transition)]
    entrypoint function charge(sig auth) : (pubkey provider, int fee, int balance) {{
      requires checkSig(auth, provider);
      {guards}
      pays(1, provider, fee);
      return Sub {{ provider: provider, fee: fee, balance: {balance_expr} }};
    }}
  }}
  lifecycle {{ live -> live via sub.charge; }}
  invariant authorized;
}}
"#
        )
    }

    #[test]
    fn accepts_pays_amount_bound_to_a_guarded_drawdown() {
        // `fee` is a committed INT and NOT value-bearing by type or by name, but
        // the same entrypoint's return decreases the value-bearing `balance` by
        // exactly it, under a `requires fee >= 0;` sign guard. That structural
        // drawdown link is what makes it the quantity leaving the model.
        let program =
            parse(&drawdown_pays_src("requires fee >= 0;", "balance - fee")).expect("parse");
        assert!(
            check(&program).is_ok(),
            "a guarded drawdown must license the pays amount: {:?}",
            check(&program).err()
        );
    }

    #[test]
    fn rejects_pays_amount_that_is_an_int_field_with_no_drawdown_link() {
        // Committed + int + sign-guarded, but the return never DECREASES anything
        // by `fee` — nothing establishes that the paid quantity leaves the model,
        // so this must stay rejected (the widening is a link, not a type waiver).
        let program = parse(&drawdown_pays_src("requires fee >= 0;", "balance")).expect("parse");
        let errs = check(&program).expect_err("an int amount with no drawdown must reject");
        assert!(
            errs.iter()
                .any(|d| d.message.contains("amount `fee`") && d.message.contains("drawn down")),
            "diagnostic must name the missing drawdown link: {errs:?}"
        );
    }

    #[test]
    fn rejects_pays_amount_whose_drawdown_term_is_unguarded() {
        // A6 interlock: the return DOES subtract `fee` from `balance`, but no
        // guard establishes `fee >= 0`. A negative `fee` inverts the subtraction
        // into a top-up, so the "drawdown" proves nothing and must not license
        // the pays amount.
        let program = parse(&drawdown_pays_src("", "balance - fee")).expect("parse");
        let errs = check(&program).expect_err("an unguarded drawdown term must reject");
        assert!(
            errs.iter()
                .any(|d| d.message.contains("amount `fee`") && d.message.contains("drawn down")),
            "diagnostic must reject the unguarded drawdown as a pays amount: {errs:?}"
        );
    }

    #[test]
    fn accepts_pays_with_a_committed_bytes32_payee() {
        // Item 3: a committed `bytes32` payee (silverscript `byte[32]`) is a SCRIPT
        // HASH, which the emitter lowers via `ScriptPubKeyP2SH` instead of
        // `ScriptPubKeyP2PK`. The checker's payee rule is about COMMITMENT, not
        // address form, so it accepts it — pinning that the P2SH payee route is
        // not gated behind a sema rejection.
        let src = r#"
pragma portrait ^0.1.0;
app Escrow {
  role escrow {
    param pubkey  buyer;
    param bytes32 seller_script;
    param coin    amount;
    state { pubkey buyer; bytes32 seller_script; coin amount; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey buyer, bytes32 seller_script, coin amount) {
      requires checkSig(auth, buyer);
      pays(0, seller_script, amount);
      return Escrow { buyer: buyer, seller_script: seller_script, amount: amount };
    }
  }
  lifecycle { live -> live via escrow.release; }
  invariant value_conserved;
}
"#;
        let program = parse(src).expect("parse");
        assert!(
            check(&program).is_ok(),
            "a committed byte[32] script-hash payee must pass check_pays: {:?}",
            check(&program).err()
        );
    }

    #[test]
    fn pays_rejects_outside_a_transition_entrypoint() {
        // A `pays(...)` in a NON-transition (here a NonCovenant/vProg) entrypoint
        // is not lowered to a covenant, so the binding would be silently dropped —
        // `check_pays` must reject it, naming the transition-only requirement.
        let src = r#"
pragma portrait ^0.1.0;
app Escrow {
  role escrow {
    param pubkey seller;
    param coin   amount;
    state { pubkey seller; coin amount; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey seller, coin amount) {
      requires checkSig(auth, seller);
      return Escrow { seller: seller, amount: amount };
    }
    entrypoint function compute(sig auth) {
      pays(0, seller, amount);
    }
  }
  lifecycle { live -> live via escrow.release; }
  invariant authorized;
}
"#;
        let program = parse(src).expect("parse");
        let errs = check(&program).expect_err("pays outside a transition must reject");
        assert!(
            errs.iter()
                .any(|d| d.message.contains("only valid in a `mode = transition`")),
            "diagnostic must name the transition-only requirement: {errs:?}"
        );
    }

    /// D2 helper: a two-`pays` role whose two output indices are `i0`/`i1`.
    fn dual_pays_src(i0: &str, i1: &str) -> String {
        format!(
            r#"
pragma portrait ^0.1.0;
app Split {{
  role split {{
    param pubkey a;
    param pubkey b;
    param coin   amt_a;
    param coin   amt_b;
    state {{ pubkey a; pubkey b; coin amt_a; coin amt_b; }}
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey a, pubkey b, coin amt_a, coin amt_b) {{
      requires checkSig(auth, a);
      pays({i0}, a, amt_a);
      pays({i1}, b, amt_b);
      return Split {{ a: a, b: b, amt_a: amt_a, amt_b: amt_b }};
    }}
  }}
  lifecycle {{ live -> live via split.release; }}
  invariant no_undeclared_state;
}}
"#
        )
    }

    /// D2 ACCEPT: two `pays(...)` clauses at DISTINCT output indices are fine.
    #[test]
    fn pays_accepts_distinct_output_indices() {
        let program = parse(&dual_pays_src("0", "1")).expect("parse");
        assert!(
            check(&program).is_ok(),
            "distinct pays indices should pass: {:?}",
            check(&program).err()
        );
    }

    /// D2 REJECT: two `pays(...)` clauses at the SAME output index collide — the
    /// second binding would silently overwrite the first.
    #[test]
    fn pays_rejects_duplicate_output_index() {
        let program = parse(&dual_pays_src("0", "0")).expect("parse");
        let errs = check(&program).expect_err("duplicate pays index must reject");
        assert!(
            errs.iter()
                .any(|d| d.message.contains("bind output index 0")
                    && d.message.contains("at most once")),
            "diagnostic must name the duplicate output index: {errs:?}"
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

    // C1/A2-full: the explicit `supply_change = issuer` capability exempts the
    // entry from the CONSERVATION shape (a real supply change does not conserve
    // supply). A2-full still requires the named authority to be committed AND
    // guaranteed to sign, so `mint_more` carries `checkSig(auth, issuer)`. This
    // entry is C1-exempt AND its authority signs → accepted.
    #[test]
    fn c1_accepts_mint_exemption() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey issuer;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, issuer);
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    // A2-full: an explicit `supply_change = owner` capability whose authority is
    // committed AND mandatorily signs. The burn return drops `supply` to a
    // constant (non-conserving) — accepted ONLY because the annotation waives C1.
    #[test]
    fn supply_change_signed_authority_accepted_and_exempt() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = owner)]
    entrypoint function burn_all(int amount) : (int supply) {
      requires checkSig(auth, owner);
      return A { supply: 0 };
    }
  }
  lifecycle { live -> live via r.burn_all; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    // A2-full reject: `supply_change = issuer` but the entry only signs a
    // DIFFERENT committed key (`owner`) — the named authority is not guaranteed
    // to sign, so the capability is invalid (check_supply_change fires).
    #[test]
    fn supply_change_unsigned_authority_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey issuer;
    param pubkey owner;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, owner);
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
}
"#;
        assert_rejects_with(src, "authority `issuer` is not guaranteed to sign");
    }

    // A2-full reject: the authority signs ONLY in a disjunctive `||` arm — a
    // satisfying path exists on which `issuer` does not sign, so it cannot
    // guarantee the supply change.
    #[test]
    fn supply_change_disjunctive_authority_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey issuer;
    param pubkey backup;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, issuer) || checkSig(auth, backup);
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
}
"#;
        assert_rejects_with(src, "authority `issuer` is not guaranteed to sign");
    }

    // A2-full RT-1 (soundness): the authority sits in a NESTED `||` under an `&&`
    // (`gate && (issuer || backup)`). A satisfying spend can sign `gate` + `backup`
    // and mint WITHOUT ever signing `issuer` — so the capability must be REJECTED.
    // The old mandatory-keyset membership WRONGLY accepted this (the `Or` case's
    // smaller-arm SET leaked `issuer` into `mandatory`).
    #[test]
    fn supply_change_nested_or_bypass_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey gate;
    param pubkey issuer;
    param pubkey backup;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, gate) && (checkSig(auth, issuer) || checkSig(auth, backup));
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
}
"#;
        assert_rejects_with(src, "authority `issuer` is not guaranteed to sign");
    }

    // A2-full RT-1 (COMMUTATIVITY): the same guard with the `||` arms SWAPPED
    // (`gate && (backup || issuer)`) must ALSO reject — the verdict may not depend
    // on arm order. The old code flipped to ACCEPT here (definitive unsoundness).
    #[test]
    fn supply_change_nested_or_bypass_arms_swapped_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey gate;
    param pubkey issuer;
    param pubkey backup;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, gate) && (checkSig(auth, backup) || checkSig(auth, issuer));
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
}
"#;
        assert_rejects_with(src, "authority `issuer` is not guaranteed to sign");
    }

    // A2-full RT-1 (asymmetric arms): `gate && (issuer || (x && y))` — the
    // authority is in the SMALLER `||` arm. A spender can take the `x && y` arm and
    // never sign `issuer`, so it must REJECT (the old smaller-arm SET admitted it).
    #[test]
    fn supply_change_asymmetric_or_arm_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey gate;
    param pubkey issuer;
    param pubkey x;
    param pubkey y;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, gate) && (checkSig(auth, issuer) || (checkSig(auth, x) && checkSig(auth, y)));
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
}
"#;
        assert_rejects_with(src, "authority `issuer` is not guaranteed to sign");
    }

    // A2-full RT-1 (accept): the authority IS forced under an `&&`
    // (`gate && issuer`) — every satisfying path signs `issuer`. Accepted.
    #[test]
    fn supply_change_and_forced_authority_accepted() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey gate;
    param pubkey issuer;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, gate) && checkSig(auth, issuer);
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
}
"#;
        assert_accepts(src);
    }

    // A2-full RT-2: a supply-change entry must release NO coin. One that ALSO
    // carries a `pays(...)` clause is a coin-releasing settlement masquerading as a
    // supply change — rejected (makes the payout_bound exclusion premise a fact).
    #[test]
    fn supply_change_with_pays_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey issuer;
    param pubkey payee;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_and_pay(int amount) : (int supply) {
      requires checkSig(auth, issuer);
      pays(0, payee, supply);
      return A { supply: supply + amount };
    }
  }
  lifecycle { live -> live via r.mint_and_pay; }
}
"#;
        assert_rejects_with(src, "must not release coin");
    }

    // A2-full RT-2: a supply-change entry named by a TERMINAL edge releases the
    // coin and ends the lifecycle — rejected for the same reason.
    #[test]
    fn supply_change_terminal_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey issuer;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, issuer);
      return A { supply: supply + amount };
    }
  }
  lifecycle { live -> done via r.mint_more terminal; }
}
"#;
        assert_rejects_with(src, "must not release coin");
    }

    // A2-full reject: `supply_change = amount` names a spender ARG, not a
    // committed key — a caller-supplied authority cannot gate a supply change.
    #[test]
    fn supply_change_non_committed_authority_rejected() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition, supply_change = amount)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, owner);
      return A { supply: amount };
    }
  }
  lifecycle { live -> live via r.mint_more; }
}
"#;
        assert_rejects_with(src, "authority `amount` is not a committed key");
    }

    // A2-full: the name no longer buys the exemption. A `mint`-named entry with
    // NO `supply_change` annotation and a non-conserving return under
    // `value_conserved` is now conservation-CHECKED and rejected.
    #[test]
    fn unannotated_mint_name_now_conservation_checked() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int supply; }
    #[covenant(mode = transition)]
    entrypoint function mint_more(int amount) : (int supply) {
      requires checkSig(auth, owner);
      return A { supply: 999999999 };
    }
  }
  lifecycle { live -> live via r.mint_more; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "value-bearing field `supply`");
    }

    // A2: a mint with NO authorization under `value_conserved` — C1-exempt from
    // conservation, but the no-auth fail-safe now fires (mint no longer opts out).
    #[test]
    fn c2_rejects_mint_without_checksig() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param int start;
    state { int supply; }
    #[covenant(mode = transition)]
    entrypoint function mintDrain(int amount) : (int supply) {
      return A { supply: 999999999 };
    }
  }
  lifecycle { live -> live via r.mintDrain; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "state-mutating transition has NO");
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
      requires amount >= 0;
      return A { balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.spend; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    // ── A6-sign: the decrease/increase guard on the adjustment term ───────────────
    //
    // `f: f - e` conserves only when `e >= 0`; with `e` negative the operator
    // inverts and the field GROWS (model money-printing). The mirror hole is
    // `f: f + e`, which with a negative `e` destroys value. Both are folded into
    // C1 unconditionally — the term's sign must be established by THIS entrypoint.

    /// A6-sign REJECT: `balance - fee` where `fee` is never established non-negative.
    /// The diagnostic must NAME the term.
    #[test]
    fn rejects_value_conserved_decrease_by_an_unguarded_term() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function settle(int fee) : (int balance) {
      requires checkSig(auth, owner);
      return A { balance: balance - fee };
    }
  }
  lifecycle { live -> live via r.settle; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "term containing `fee`");
    }

    /// A6-sign ACCEPT: the same decrease with the term's sign established here.
    #[test]
    fn accepts_value_conserved_decrease_when_the_term_is_guarded() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function settle(int fee) : (int balance) {
      requires checkSig(auth, owner);
      requires fee >= 0;
      return A { balance: balance - fee };
    }
  }
  lifecycle { live -> live via r.settle; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    /// A6-sign REJECT (the symmetric hole): `balance + credit` with an unguarded
    /// `credit` DESTROYS value when `credit` is negative. Covering only the
    /// subtraction direction would be an honesty gap.
    #[test]
    fn rejects_value_conserved_increase_by_an_unguarded_term() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function credit_in(int credit) : (int balance) {
      requires checkSig(auth, owner);
      return A { balance: balance + credit };
    }
  }
  lifecycle { live -> live via r.credit_in; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "term containing `credit`");
    }

    /// A6-sign ACCEPT: a compound subtrahend `- (s + m + j)` (the TrancheWaterfall
    /// shape) passes when EVERY `+`-atom is established non-negative.
    #[test]
    fn accepts_compound_subtrahend_when_every_atom_is_guarded() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function distribute(int s, int m, int j) : (int balance) {
      requires checkSig(auth, owner);
      requires s >= 0;
      requires m >= 0;
      requires j >= 0;
      return A { balance: balance - (s + m + j) };
    }
  }
  lifecycle { live -> live via r.distribute; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    /// A6-sign REJECT: one unguarded atom is enough — a negative `j` makes the whole
    /// subtrahend able to go negative, inflating the field.
    #[test]
    fn rejects_compound_subtrahend_when_one_atom_is_unguarded() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function distribute(int s, int m, int j) : (int balance) {
      requires checkSig(auth, owner);
      requires s >= 0;
      requires m >= 0;
      return A { balance: balance - (s + m + j) };
    }
  }
  lifecycle { live -> live via r.distribute; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "term containing `j`");
    }

    /// L1 — `requires fee > -1;` is documented and CHANGELOG'd as an accepted
    /// form. It was REJECTED in practice: `-1` parses as `Unary{Neg, Int(1)}`,
    /// never `Int(-1)`, so the `>= -1` arm was unreachable dead code.
    #[test]
    fn accepts_strict_gt_negative_one_as_a_non_negativity_guard() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function settle(int fee) : (int balance) {
      requires checkSig(auth, owner);
      requires fee > -1;
      return A { balance: balance - fee };
    }
  }
  lifecycle { live -> live via r.settle; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    /// L2 — the mirrored operand order `0 <= fee` is the same guard written the
    /// other way round and must not false-reject.
    #[test]
    fn accepts_mirrored_operand_order_as_a_non_negativity_guard() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    state { int balance; }
    #[covenant(mode = transition)]
    entrypoint function settle(int fee) : (int balance) {
      requires checkSig(auth, owner);
      requires 0 <= fee;
      return A { balance: balance - fee };
    }
  }
  lifecycle { live -> live via r.settle; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    /// M2 — `supply_change` authorises a SUPPLY CHANGE, not a sign inversion.
    /// The capability waives the conservation SHAPE; A6-sign still applies, so a
    /// mint authority cannot covertly inflate via an unguarded subtrahend.
    #[test]
    fn supply_change_capability_does_not_waive_the_a6_sign_check() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey issuer;
    param int supply;
    state { pubkey issuer; int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function issue(sig auth, int fee) : (pubkey issuer, int supply) {
      requires checkSig(auth, issuer);
      return A { issuer: issuer, supply: supply - fee };
    }
  }
  lifecycle { live -> live via r.issue; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "term containing `fee`");
        assert_rejects_with(src, "does NOT waive this check");
    }

    /// M2 control: the capability DOES still waive the conservation shape, so a
    /// genuine non-conserving mint is accepted once the sign is established.
    #[test]
    fn supply_change_capability_still_waives_the_conservation_shape() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey issuer;
    param int supply;
    state { pubkey issuer; int supply; }
    #[covenant(mode = transition, supply_change = issuer)]
    entrypoint function issue(sig auth, int minted) : (pubkey issuer, int supply) {
      requires checkSig(auth, issuer);
      requires minted >= 0;
      return A { issuer: issuer, supply: minted };
    }
  }
  lifecycle { live -> live via r.issue; }
  invariant value_conserved;
}
"#;
        assert_accepts(src);
    }

    /// M3 — a param named `max_ins`/`max_outs` is emitted alongside the bound the
    /// Engraver injects under the same name. silverc accepts the duplicate and
    /// the USER's param wins, handing the covenant's output-count bound to the
    /// deployer. Reject at the source level.
    #[test]
    fn rejects_a_role_param_shadowing_an_injected_emitter_name() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    param int max_outs;
    state { pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function touch() : (pubkey owner) {
      requires checkSig(auth, owner);
      return A { owner: owner };
    }
  }
  lifecycle { live -> live via r.touch; }
  invariant authorized;
}
"#;
        assert_rejects_with(src, "RESERVED emitter identifier");
        assert_rejects_with(src, "max_outs");
    }

    /// M3 — the same reservation applies to a state field, which would force a
    /// duplicate ctor param of the reserved name.
    #[test]
    fn rejects_a_state_field_shadowing_an_injected_emitter_name() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    param int max_ins;
    state { pubkey owner; int max_ins; }
    #[covenant(mode = transition)]
    entrypoint function touch() : (pubkey owner, int max_ins) {
      requires checkSig(auth, owner);
      return A { owner: owner, max_ins: max_ins };
    }
  }
  lifecycle { live -> live via r.touch; }
  invariant authorized;
}
"#;
        assert_rejects_with(src, "RESERVED emitter identifier");
    }

    /// M4 — genesis binding is by name, so a duplicate param makes the
    /// initialiser ambiguous and the second declaration unreachable. Reject it
    /// here rather than letting emit report a misleading TYPE mismatch.
    #[test]
    fn rejects_duplicate_role_param_names() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param sig auth;
    param pubkey balance;
    param int balance;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(int amount) : (pubkey owner, int balance) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      return A { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.spend; }
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "declared more than once");
    }

    /// M1 — a conservation invariant on a role with no value-bearing field
    /// checks nothing yet reports ok. It must WARN. (Not a rejection: `Htlc` and
    /// `EvidenceLineage` ship in exactly this shape, and `value_conserved` is
    /// also what opts them into the C2 no-auth fail-safe, so rejecting would
    /// silently strip their authorization requirement.)
    #[test]
    fn warns_when_a_declared_conservation_invariant_matches_no_field() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role vault {
    param pubkey owner;
    param sig auth;
    param int funds;
    state { pubkey owner; int funds; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(int take) : (pubkey owner, int funds) {
      requires checkSig(auth, owner);
      return A { owner: owner, funds: funds - take };
    }
  }
  lifecycle { live -> live via vault.withdraw; }
  invariant value_conserved;
}
"#;
        let program = parse(src).expect("parses");
        // It PASSES check — that is exactly the hazard.
        assert!(
            check(&program).is_ok(),
            "the vacuous case still type-checks"
        );
        let ws = warnings(&program);
        assert!(
            ws.iter().any(
                |w| w.contains("`value_conserved` is declared but NO state field")
                    && w.contains("`vault`")
            ),
            "a vacuous conservation invariant must warn, got: {ws:?}"
        );
    }

    /// M1 control: a role WITH a value-bearing field warns about nothing.
    #[test]
    fn no_vacuity_warning_when_a_value_bearing_field_exists() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role vault {
    param pubkey owner;
    param sig auth;
    param int balance;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(int amount) : (pubkey owner, int balance) {
      requires checkSig(auth, owner);
      requires amount >= 0;
      return A { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via vault.withdraw; }
  invariant value_conserved;
}
"#;
        let program = parse(src).expect("parses");
        assert!(warnings(&program).is_empty(), "no warning is owed here");
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

    // A1: a disjunctive authorization `checkSig(owner) || checkSig(attacker)`
    // where the `attacker` arm binds a CALLER-SUPPLIED pubkey leaves a satisfying
    // path with no committed signature → guaranteed committed signers == 0 →
    // rejected under `authorized`.
    #[test]
    fn c2_rejects_disjunctive_authorization() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig owner_auth, sig attacker_auth, pubkey attacker, int amount) : (pubkey owner, int balance) {
      requires checkSig(owner_auth, owner) || checkSig(attacker_auth, attacker);
      return A { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
  invariant authorized;
}
"#;
        assert_rejects_with(src, "capability check failed");
    }

    // A1: a negated signature check `!checkSig(auth, owner)` authorizes nothing —
    // no committed key is guaranteed to sign → rejected under `authorized`.
    #[test]
    fn c2_rejects_negated_authorization() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, int amount) : (pubkey owner, int balance) {
      requires !checkSig(auth, owner);
      return A { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
  invariant authorized;
}
"#;
        assert_rejects_with(src, "capability check failed");
    }

    // A4: a role declaring `temporal_guard` whose only mutating transition has no
    // committed-time gate at all → rejected (existence tightening).
    #[test]
    fn d3_temporal_guard_rejects_missing_gate() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    state { pubkey owner; int deadline; int settled; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth) : (pubkey owner, int deadline, int settled) {
      requires checkSig(auth, owner);
      return A { owner: owner, deadline: deadline, settled: 1 };
    }
  }
  lifecycle { live -> live via r.refund; }
  invariant temporal_guard;
}
"#;
        assert_rejects_with(src, "invariant `temporal_guard` violated");
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

    // M-1: key-aliasing multisig — `checkSig(a, signer) && checkSig(b,
    // prev_states[0].signer)` names the SAME committed key twice (a self-loop
    // alias), so only 1 distinct committed key is guaranteed → `multisig_threshold`
    // must reject it (was a deployable 1-of-1 masquerading as 2-of-2).
    #[test]
    fn d3_multisig_threshold_rejects_key_aliasing() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role treasury {
    param pubkey signer;
    param int balance;
    state { pubkey signer; int balance; }
    #[covenant(mode = transition)]
    entrypoint function spend(sig auth_a, sig auth_b, int amount) : (pubkey signer, int balance) {
      requires checkSig(auth_a, signer) && checkSig(auth_b, prev_states[0].signer);
      requires amount >= 0;
      requires amount <= balance;
      return A { signer: signer, balance: balance - amount };
    }
  }
  lifecycle { live -> live via treasury.spend; }
  invariant value_conserved;
  invariant authorized;
  invariant multisig_threshold;
}
"#;
        assert_rejects_with(src, "invariant `multisig_threshold` violated");
    }

    // L-3: `temporal_guard` declared but the only `now_bucket` comparison gates on
    // a NON-time committed field (`balance`), which is not a deadline → the role
    // has no committed-time gate → rejected.
    #[test]
    fn d3_temporal_guard_rejects_non_time_field_gate() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param int balance;
    state { pubkey owner; int balance; int settled; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth, int now_bucket) : (pubkey owner, int balance, int settled) {
      requires checkSig(auth, owner);
      requires now_bucket >= balance;
      return A { owner: owner, balance: balance, settled: 1 };
    }
  }
  lifecycle { live -> live via r.refund; }
  invariant temporal_guard;
}
"#;
        assert_rejects_with(src, "invariant `temporal_guard` violated");
    }

    // L-1: an entrypoint argument shadowing a committed name (`owner` is both a
    // committed param/state field and a caller-supplied arg) is rejected by the
    // checker with a clean diagnostic — not left to a downstream silverc panic.
    #[test]
    fn rejects_arg_shadowing_committed_name() {
        let src = r#"
pragma portrait ^0.1.0;
app A {
  role r {
    param pubkey owner;
    param int balance;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function withdraw(sig auth, pubkey owner, int amount) : (pubkey owner, int balance) {
      requires checkSig(auth, owner);
      return A { owner: owner, balance: balance - amount };
    }
  }
  lifecycle { live -> live via r.withdraw; }
  invariant authorized;
  invariant value_conserved;
}
"#;
        assert_rejects_with(src, "shadows a committed name");
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

    // ---- B1: after(deadline) consensus time-gate ------------------------------

    /// B1 ACCEPT: an `after(unlock_bucket)` clause naming a committed int-typed
    /// time field is well-formed.
    #[test]
    fn after_accepts_committed_time_deadline() {
        let src = r#"
pragma portrait ^0.1.0;
app Vault {
  role vault {
    param pubkey owner;
    param int    unlock_bucket;
    state { pubkey owner; int unlock_bucket; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey owner, int unlock_bucket) {
      requires checkSig(auth, owner);
      after(unlock_bucket);
      return Vault { owner: owner, unlock_bucket: unlock_bucket };
    }
  }
  lifecycle { live -> live via vault.release; }
  invariant no_undeclared_state;
}
"#;
        assert_accepts(src);
    }

    /// B1 REJECT: an `after(now)` deadline that is a spender-supplied argument is
    /// no gate at all.
    #[test]
    fn after_rejects_spender_arg_deadline() {
        let src = r#"
pragma portrait ^0.1.0;
app Vault {
  role vault {
    param pubkey owner;
    state { pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth, int deadline) : (pubkey owner) {
      requires checkSig(auth, owner);
      after(deadline);
      return Vault { owner: owner };
    }
  }
  lifecycle { live -> live via vault.release; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "spender-supplied argument");
    }

    /// B1 REJECT: a committed but non-time field (`balance`) cannot masquerade as
    /// a deadline.
    #[test]
    fn after_rejects_non_time_field_deadline() {
        let src = r#"
pragma portrait ^0.1.0;
app Vault {
  role vault {
    param pubkey owner;
    param int    balance;
    state { pubkey owner; int balance; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey owner, int balance) {
      requires checkSig(auth, owner);
      after(balance);
      return Vault { owner: owner, balance: balance };
    }
  }
  lifecycle { live -> live via vault.release; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "must be a committed TIME field");
    }

    /// B1 (D1) ACCEPT: `after(last_charged + period)` — the two-atom window form —
    /// is well-formed when BOTH operands are committed int-typed time fields.
    #[test]
    fn after_sum_accepts_two_committed_time_atoms() {
        let src = r#"
pragma portrait ^0.1.0;
app Sub {
  role sub {
    param pubkey owner;
    param int    last_charged;
    param int    period;
    state { pubkey owner; int last_charged; int period; }
    #[covenant(mode = transition)]
    entrypoint function charge(sig auth) : (pubkey owner, int last_charged, int period) {
      requires checkSig(auth, owner);
      after(last_charged + period);
      return Sub { owner: owner, last_charged: last_charged, period: period };
    }
  }
  lifecycle { live -> live via sub.charge; }
  invariant no_undeclared_state;
}
"#;
        assert_accepts(src);
    }

    /// B1 (D1) REJECT: a sum whose operand is a spender-supplied argument is no
    /// gate — the caller could choose the window.
    #[test]
    fn after_sum_rejects_spender_arg_operand() {
        let src = r#"
pragma portrait ^0.1.0;
app Sub {
  role sub {
    param pubkey owner;
    param int    last_charged;
    state { pubkey owner; int last_charged; }
    #[covenant(mode = transition)]
    entrypoint function charge(sig auth, int period) : (pubkey owner, int last_charged) {
      requires checkSig(auth, owner);
      after(last_charged + period);
      return Sub { owner: owner, last_charged: last_charged };
    }
  }
  lifecycle { live -> live via sub.charge; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "spender-supplied argument");
    }

    /// RT-1 REJECT: `after(period + timeout)` is duration+duration — a tiny
    /// threshold that is no real gate, even though both names are on the time
    /// allowlist. Must be rejected with the anchor+duration diagnostic.
    #[test]
    fn after_sum_rejects_duration_plus_duration() {
        let src = r#"
pragma portrait ^0.1.0;
app Sub {
  role sub {
    param pubkey owner;
    param int    period;
    param int    timeout;
    state { pubkey owner; int period; int timeout; }
    #[covenant(mode = transition)]
    entrypoint function charge(sig auth) : (pubkey owner, int period, int timeout) {
      requires checkSig(auth, owner);
      after(period + timeout);
      return Sub { owner: owner, period: period, timeout: timeout };
    }
  }
  lifecycle { live -> live via sub.charge; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(
            src,
            "anchor+anchor overshoots and duration+duration is no real gate",
        );
    }

    /// RT-1 REJECT: `after(deadline + cliff)` is anchor+anchor — the threshold
    /// overshoots (sum of two absolute points), which can lock the UTXO past any
    /// real time. Must be rejected with the anchor+duration diagnostic.
    #[test]
    fn after_sum_rejects_anchor_plus_anchor() {
        let src = r#"
pragma portrait ^0.1.0;
app Grant {
  role grant {
    param pubkey owner;
    param int    deadline;
    param int    cliff;
    state { pubkey owner; int deadline; int cliff; }
    #[covenant(mode = transition)]
    entrypoint function vest(sig auth) : (pubkey owner, int deadline, int cliff) {
      requires checkSig(auth, owner);
      after(deadline + cliff);
      return Grant { owner: owner, deadline: deadline, cliff: cliff };
    }
  }
  lifecycle { live -> live via grant.vest; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "must be a committed ANCHOR");
    }

    /// B1 (D1) REJECT: a sum whose operand is a committed but NON-time field cannot
    /// masquerade as a window bound.
    #[test]
    fn after_sum_rejects_non_time_operand() {
        let src = r#"
pragma portrait ^0.1.0;
app Sub {
  role sub {
    param pubkey owner;
    param int    last_charged;
    param int    balance;
    state { pubkey owner; int last_charged; int balance; }
    #[covenant(mode = transition)]
    entrypoint function charge(sig auth) : (pubkey owner, int last_charged, int balance) {
      requires checkSig(auth, owner);
      after(last_charged + balance);
      return Sub { owner: owner, last_charged: last_charged, balance: balance };
    }
  }
  lifecycle { live -> live via sub.charge; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "must be a committed TIME field");
    }

    /// B1 REJECT: `after(...)` outside a `mode = transition` entrypoint would be
    /// silently dropped, so it is rejected.
    #[test]
    fn after_rejects_non_transition_entrypoint() {
        let src = r#"
pragma portrait ^0.1.0;
app Vault {
  role vault {
    param pubkey owner;
    param int    unlock_bucket;
    state { pubkey owner; int unlock_bucket; }
    #[covenant(mode = verification)]
    entrypoint function release(sig auth) : (pubkey owner, int unlock_bucket) {
      requires checkSig(auth, owner);
      after(unlock_bucket);
      return Vault { owner: owner, unlock_bucket: unlock_bucket };
    }
  }
  lifecycle { live -> live via vault.release; }
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "only valid in a `mode = transition`");
    }

    /// B1 ACCEPT: an `after(unlock_bucket)` clause satisfies `invariant
    /// temporal_guard` — the consensus gate is a strictly stronger temporal gate
    /// than the caller-asserted `now_bucket` comparison.
    #[test]
    fn after_satisfies_temporal_guard_invariant() {
        let src = r#"
pragma portrait ^0.1.0;
app Vault {
  role vault {
    param pubkey owner;
    param int    unlock_bucket;
    param int    released;
    state { pubkey owner; int unlock_bucket; int released; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey owner, int unlock_bucket, int released) {
      requires checkSig(auth, owner);
      requires released == 0;
      after(unlock_bucket);
      return Vault { owner: owner, unlock_bucket: unlock_bucket, released: 1 };
    }
  }
  lifecycle { live -> live via vault.release; }
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
      requires amount >= 0;
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
      requires x >= 0;
      requires y >= 0;
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
      requires x >= 0;
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

    /// A6-sign under `conservation_split`: the two legs cancel structurally whatever
    /// the sign of `x`, so cancellation alone cannot see that a negative `x`
    /// REVERSES the transfer — draining the destination into the source. The
    /// leg-level sign guard catches it and names the term.
    #[test]
    fn rejects_conservation_split_leg_with_an_unguarded_term() {
        let src = r#"
pragma portrait ^0.1.0;
app S {
  role pool {
    param int a_balance;
    param int b_balance;
    param pubkey owner;
    state { int a_balance; int b_balance; pubkey owner; }
    #[covenant(mode = transition)]
    entrypoint function move_ab(sig auth, int x) : (int a_balance, int b_balance, pubkey owner) {
      requires checkSig(auth, owner);
      return S {
        a_balance: a_balance - x,
        b_balance: b_balance + x,
        owner:     owner
      };
    }
  }
  lifecycle { live -> live via pool.move_ab; }
  invariant conservation_split;
  invariant authorized;
}
"#;
        assert_rejects_with(src, "term containing `x`");
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
                        supply_change: None,
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

    // ---- A4-full: formula-bearing temporal invariants ---------------------

    /// A `refund`-shaped app whose `after(...)` clause is `after_clause` (pass ""
    /// to DELETE it) and whose app-level temporal-path `invariant` is `invariant`.
    fn temporal_path_src(after_clause: &str, invariant: &str) -> String {
        format!(
            r#"
pragma portrait ^0.1.0;
app Vault {{
  role vault {{
    param pubkey owner;
    param int    deadline;
    param int    cliff;
    state {{ pubkey owner; int deadline; int cliff; }}
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth) : (pubkey owner, int deadline, int cliff) {{
      requires checkSig(auth, owner);
      {after_clause}
      return Vault {{ owner: owner, deadline: deadline, cliff: cliff }};
    }}
  }}
  lifecycle {{ live -> live via vault.refund; }}
  {invariant}
  invariant no_undeclared_state;
}}
"#
        )
    }

    #[test]
    fn temporal_path_requires_matching_after_clause() {
        // Passes when `refund` carries the matching `after(deadline)` clause.
        assert_accepts(&temporal_path_src(
            "after(deadline);",
            "invariant refund_after_deadline: refund => after(deadline);",
        ));
        // Survives-deletion: DELETE the `after(deadline)` clause and the same
        // program now FAILS — the invariant pins the clause to the entrypoint.
        assert_rejects_with(
            &temporal_path_src(
                "",
                "invariant refund_after_deadline: refund => after(deadline);",
            ),
            "must carry the matching `after(deadline)`",
        );
    }

    #[test]
    fn temporal_path_unknown_entrypoint_rejected() {
        assert_rejects_with(
            &temporal_path_src(
                "after(deadline);",
                "invariant refund_after_deadline: settle => after(deadline);",
            ),
            "entrypoint `settle` is not declared in any role",
        );
    }

    #[test]
    fn temporal_path_deadline_mismatch_rejected() {
        // `refund` carries `after(deadline)` but the invariant names `after(cliff)`:
        // the deadline shape does not match, so the structural check rejects it.
        assert_rejects_with(
            &temporal_path_src(
                "after(deadline);",
                "invariant refund_after_deadline: refund => after(cliff);",
            ),
            "must carry the matching `after(cliff)`",
        );
    }

    // ---- A6: payout_bound -------------------------------------------------

    /// An Escrow-shaped app whose settling `release` transition (flips `settled`
    /// 0 → 1) carries `release_pays` (pass "" to OMIT the payout binding), under
    /// `invariant payout_bound;`.
    fn payout_bound_src(release_pays: &str) -> String {
        format!(
            r#"
pragma portrait ^0.1.0;
app Escrow {{
  role escrow {{
    param pubkey seller;
    param coin   amount;
    param int    settled;
    state {{ pubkey seller; coin amount; int settled; }}
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey seller, coin amount, int settled) {{
      requires checkSig(auth, seller);
      requires settled == 0;
      {release_pays}
      return Escrow {{ seller: seller, amount: amount, settled: 1 }};
    }}
  }}
  lifecycle {{ live -> live via escrow.release; }}
  invariant payout_bound;
  invariant no_undeclared_state;
}}
"#
        )
    }

    #[test]
    fn payout_bound_passes_when_settlement_pays() {
        assert_accepts(&payout_bound_src("pays(0, seller, amount);"));
    }

    #[test]
    fn payout_bound_fails_when_settlement_omits_pays() {
        // Survives-deletion: DELETE the `pays(...)` clause and the settling path
        // now FAILS — payout_bound makes the payout binding a mandatory obligation.
        assert_rejects_with(&payout_bound_src(""), "invariant `payout_bound` violated");
    }

    #[test]
    fn payout_bound_ignores_non_settling_transitions() {
        // `release` settles and pays (so payout_bound is not vacuous); `poke` does
        // NOT flip a one-shot flag, so it is not a settlement and payout_bound does
        // not require it to pay — the app is accepted.
        let src = r#"
pragma portrait ^0.1.0;
app Escrow {
  role escrow {
    param pubkey seller;
    param coin   amount;
    param int    settled;
    state { pubkey seller; coin amount; int settled; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey seller, coin amount, int settled) {
      requires checkSig(auth, seller);
      requires settled == 0;
      pays(0, seller, amount);
      return Escrow { seller: seller, amount: amount, settled: 1 };
    }
    #[covenant(mode = transition)]
    entrypoint function poke(sig auth) : (pubkey seller, coin amount, int settled) {
      requires checkSig(auth, seller);
      return Escrow { seller: seller, amount: amount, settled: settled };
    }
  }
  lifecycle {
    live -> live via escrow.release;
    live -> live via escrow.poke;
  }
  invariant payout_bound;
  invariant no_undeclared_state;
}
"#;
        assert_accepts(src);
    }

    // H-1: the recognizer catches the bool and computed one-shot flips too, so a
    // real settlement written in those shapes cannot escape payout_bound.

    #[test]
    fn payout_bound_fails_when_bool_flip_settlement_omits_pays() {
        // A bool one-shot flip (`require settled == false;` + `settled: true`) with
        // NO pays is a settlement that must be rejected (previously escaped).
        let src = r#"
pragma portrait ^0.1.0;
app Escrow {
  role escrow {
    param pubkey seller;
    param coin   amount;
    param bool   settled;
    state { pubkey seller; coin amount; bool settled; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey seller, coin amount, bool settled) {
      requires checkSig(auth, seller);
      requires settled == false;
      return Escrow { seller: seller, amount: amount, settled: true };
    }
  }
  lifecycle { live -> live via escrow.release; }
  invariant payout_bound;
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "invariant `payout_bound` violated");
    }

    #[test]
    fn payout_bound_fails_when_computed_flip_settlement_omits_pays() {
        // A computed one-shot flip (`require settled == 0;` + `settled: settled + 1`)
        // with NO pays is a settlement that must be rejected (previously escaped).
        let src = r#"
pragma portrait ^0.1.0;
app Escrow {
  role escrow {
    param pubkey seller;
    param coin   amount;
    param int    settled;
    state { pubkey seller; coin amount; int settled; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey seller, coin amount, int settled) {
      requires checkSig(auth, seller);
      requires settled == 0;
      return Escrow { seller: seller, amount: amount, settled: settled + 1 };
    }
  }
  lifecycle { live -> live via escrow.release; }
  invariant payout_bound;
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "invariant `payout_bound` violated");
    }

    #[test]
    fn payout_bound_vacuous_declaration_rejected() {
        // FAIL-LOUD ON VACUITY: payout_bound declared but NO settling transition is
        // recognized — the invariant is rejected, so a 0-match pass cannot pose as
        // enforcement.
        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param pubkey owner;
    param int    n;
    state { pubkey owner; int n; }
    #[covenant(mode = transition)]
    entrypoint function poke(sig auth) : (pubkey owner, int n) {
      requires checkSig(auth, owner);
      return Counter { owner: owner, n: n };
    }
  }
  lifecycle { live -> live via counter.poke; }
  invariant payout_bound;
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "no settling transition was recognized");
    }

    #[test]
    fn settling_transition_count_reports_recognized_settlements() {
        // The coverage signal counts recognized settling transitions across roles.
        let program = parse(&payout_bound_src("pays(0, seller, amount);")).expect("parse");
        assert_eq!(settling_transition_count(&program.app), 1);
    }

    // ---- B3: terminal settling transitions --------------------------------

    /// An Escrow-shaped app whose `release` is a TERMINAL transition (a lifecycle
    /// edge marked `terminal`, no successor return). `release_pays` is spliced into
    /// the body (pass "" to OMIT the payout binding), under `invariant payout_bound;`.
    fn terminal_payout_src(release_pays: &str) -> String {
        format!(
            r#"
pragma portrait ^0.1.0;
app Escrow {{
  role escrow {{
    param pubkey seller;
    param coin   amount;
    state {{ pubkey seller; coin amount; }}
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) {{
      requires checkSig(auth, seller);
      {release_pays}
    }}
  }}
  lifecycle {{ live -> released via escrow.release terminal; }}
  invariant payout_bound;
  invariant no_undeclared_state;
}}
"#
        )
    }

    #[test]
    fn payout_bound_recognizes_terminal_settling_transition() {
        // A TERMINAL transition (releases the coin, ends the lifecycle) is a
        // recognized settling transition; with its `pays(...)` binding present,
        // payout_bound is satisfied (and non-vacuous → 1 recognized settlement).
        let src = terminal_payout_src("pays(0, seller, amount);");
        assert_accepts(&src);
        let program = parse(&src).expect("parse");
        assert_eq!(settling_transition_count(&program.app), 1);
    }

    #[test]
    fn terminal_settle_without_pays_fails_loud() {
        // Survives-deletion: DELETE the `pays(...)` on a terminal settle and
        // payout_bound rejects it — a terminal spend must bind its payout.
        assert_rejects_with(
            &terminal_payout_src(""),
            "invariant `payout_bound` violated",
        );
    }

    #[test]
    fn terminal_transition_with_return_is_rejected() {
        // A terminal transition must not return a successor: the coin is released
        // via pays and the UTXO is consumed, so a `return` is a contradiction.
        let src = r#"
pragma portrait ^0.1.0;
app Escrow {
  role escrow {
    param pubkey seller;
    param coin   amount;
    state { pubkey seller; coin amount; }
    #[covenant(mode = transition)]
    entrypoint function release(sig auth) : (pubkey seller, coin amount) {
      requires checkSig(auth, seller);
      pays(0, seller, amount);
      return Escrow { seller: seller, amount: amount };
    }
  }
  lifecycle { live -> released via escrow.release terminal; }
  invariant payout_bound;
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "must not return a successor");
    }

    #[test]
    fn terminal_burn_named_settle_without_pays_fails_loud() {
        // R2: a TERMINAL transition named `burn_out` still RELEASES coin, so the
        // mint/burn exemption must NOT apply to it — payout_bound must recognize it
        // as settling and reject the missing `pays(...)`. Survives-deletion: with a
        // `pays(...)` present it would be accepted; without one it is rejected.
        let with_pays = r#"
pragma portrait ^0.1.0;
app Vault {
  role vault {
    param pubkey owner;
    param coin   amount;
    state { pubkey owner; coin amount; }
    #[covenant(mode = transition)]
    entrypoint function burn_out(sig auth) {
      requires checkSig(auth, owner);
      pays(0, owner, amount);
    }
  }
  lifecycle { live -> burned via vault.burn_out terminal; }
  invariant payout_bound;
  invariant no_undeclared_state;
}
"#;
        assert_accepts(with_pays);
        let no_pays = with_pays.replace("      pays(0, owner, amount);\n", "");
        assert_rejects_with(&no_pays, "invariant `payout_bound` violated");
    }

    #[test]
    fn terminal_recognition_is_role_qualified() {
        // R4: a terminal `refund` in role `a` must NOT force a NON-terminal `refund`
        // in role `b` to carry `pays(...)`. Role `b.refund` continues to a successor
        // and settles nothing, so payout_bound must accept it unbound while still
        // obligating the terminal `a.refund`.
        let src = r#"
pragma portrait ^0.1.0;
app Two {
  role a {
    param pubkey owner;
    param coin   amount;
    state { pubkey owner; coin amount; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth) {
      requires checkSig(auth, owner);
      pays(0, owner, amount);
    }
  }
  role b {
    param pubkey owner;
    param int    n;
    state { pubkey owner; int n; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth) : (pubkey owner, int n) {
      requires checkSig(auth, owner);
      return B { owner: owner, n: n };
    }
  }
  lifecycle {
    live -> refunded via a.refund terminal;
    live -> live via b.refund;
  }
  invariant payout_bound;
  invariant no_undeclared_state;
}
"#;
        assert_accepts(src);
        // Only the terminal `a.refund` is recognized as settling (role-qualified).
        let program = parse(src).expect("parse");
        assert_eq!(settling_transition_count(&program.app), 1);
    }

    // M-1: the bare entry name binds by entrypoint NAME across every role.

    #[test]
    fn temporal_path_rejects_when_a_second_role_occurrence_lacks_the_gate() {
        // Two roles both declare `refund`: `a.refund` carries `after(deadline)` but
        // `b.refund` does not. The invariant must reject (it would otherwise read as
        // "refund is time-gated" while one occurrence is not).
        let src = r#"
pragma portrait ^0.1.0;
app Vault {
  role a {
    param pubkey owner;
    param int    deadline;
    state { pubkey owner; int deadline; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth) : (pubkey owner, int deadline) {
      requires checkSig(auth, owner);
      after(deadline);
      return Vault { owner: owner, deadline: deadline };
    }
  }
  role b {
    param pubkey owner;
    param int    deadline;
    state { pubkey owner; int deadline; }
    #[covenant(mode = transition)]
    entrypoint function refund(sig auth) : (pubkey owner, int deadline) {
      requires checkSig(auth, owner);
      return Vault { owner: owner, deadline: deadline };
    }
  }
  lifecycle {
    live -> live via a.refund;
    live -> live via b.refund;
  }
  invariant refund_after_deadline: refund => after(deadline);
  invariant no_undeclared_state;
}
"#;
        assert_rejects_with(src, "`b.refund` must carry the matching `after(deadline)`");
    }

    #[test]
    fn temporal_path_sum_deadline_matches_either_operand_order() {
        // L-2: `after(a + b)` in the body satisfies an invariant naming
        // `after(b + a)` — a Sum window is compared unordered.
        let src = r#"
pragma portrait ^0.1.0;
app Sub {
  role sub {
    param pubkey owner;
    param int    last_charged;
    param int    period;
    state { pubkey owner; int last_charged; int period; }
    #[covenant(mode = transition)]
    entrypoint function charge(sig auth) : (pubkey owner, int last_charged, int period) {
      requires checkSig(auth, owner);
      after(last_charged + period);
      return Sub { owner: owner, last_charged: last_charged, period: period };
    }
  }
  lifecycle { live -> live via sub.charge; }
  invariant charge_after_window: charge => after(period + last_charged);
  invariant no_undeclared_state;
}
"#;
        assert_accepts(src);
    }
}
