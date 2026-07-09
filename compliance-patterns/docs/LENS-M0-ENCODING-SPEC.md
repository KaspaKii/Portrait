# LENS — M0 ENCODING SPEC (AST → SMT-LIB)

> **STATUS UPDATE (2026-06-29) — M1+M2+M3: FOUR VC classes implemented.** The
> in-tree `portrait-lens` crate (Portrait repo) now builds **four** VC classes,
> each grounded in the *real* surface AST and each routed through the *same*
> SAT(T) vacuity-safe discharge (only a satisfiable `T` plus an unsat negated-VC
> yields `PROVED`):
>
> 1. **Total value conservation** (§4(a), internal-flow form) — M1/M2.
> 2. **Range / overflow** (§4(c)) — for every value-bearing field whose return
>    performs arithmetic, the obligation `G ⟹ 0 <= f' < 2^64` (the on-chain
>    `u64` sompi window; a documented refinement of A2). `unsat`→PROVED (no
>    overflow/underflow), `sat`→REFUTED + counter-model. This turns the prior
>    honest-scope bounded-int gap (`a*2` PROVED over ℤ but wraps on-chain) into a
>    CHECKED obligation — and on the real `InternalSplit.rebalance` it **REFUTES**
>    (the int legs carry no `>= 0` lower bound, so the model admits a negative
>    leg that would underflow on-chain — a genuine finding conservation alone
>    misses).
> 3. **Refinement** (§4(b)) — `G ⟹ φ` for a **declared** named refinement
>    invariant: `non_negative_amount` (`G ⟹ amount >= 0`), `spending_cap`
>    (`G ⟹ amount <= limit`). Generated only when the app declares the tag.
> 4. **Invariant preservation** (§4(d)) — `I(s) ∧ G ⟹ I(s')` for a **declared**
>    stateful invariant: `bounded_supply` (`supply <= total`), `monotonic_seq`
>    (`seq' = seq + 1`). The `I(pre)` hypothesis is asserted on the query but NOT
>    on the vacuity probe, so a contradictory `I(pre)` cannot vacuously discharge.
>
> **Honest scope of §4(b)/(d).** The surface AST has **no syntax** for a
> *user-written arbitrary arithmetic invariant* `I` (e.g. a bespoke ceiling
> `balance <= cap`): `portrait_syntax::Invariant` is a *named tag*
> (`ValueConserved` / `NoUndeclaredState` / `Custom(String)`) whose arithmetic
> meaning is fixed in `portrait-sema`, never an author-supplied `Expr`. So §4(b)
> and §4(d) are grounded ONLY in the named invariants with sema-defined
> arithmetic semantics; a VC for a predicate the language cannot state is not
> generatable and is NOT faked. The `temporal_guard` named invariant is likewise
> not generated (its meaning is a structural time/capability gate, not a closed
> arithmetic obligation). The **spend VC** (`spent_out` binding, Q3) remains
> deferred. It proves the Portrait **MODEL**, not the emitted `.sil`. The
> value-bearing set `V` uses the wide `is_value_bearing_split` rule (§4(a));
> `UNKNOWN` is first-class; only the literal solver `unsat` (over a satisfiable
> `T`) maps to `PROVED`. The original M0 review-gate text below is retained
> verbatim for the encoding it pins; treat its "Nothing here is built" language
> as superseded **for the four implemented VC classes**.
>
> **Adversarially hardened (2026-06-29).** A soundness fix was applied for a
> **vacuous false-`PROVED`**: an *unsatisfiable* guard on a
> value-creating body returned `PROVED`, because `T ∧ ¬VC` is trivially `unsat`
> when `T` itself is `unsat`. `discharge()` now probes `T` alone for
> satisfiability first; an unsatisfiable (vacuous / unreachable) transition maps
> to `UNKNOWN`, never `PROVED`. Assumptions A1–A4 are now inlined into the crate
> docs. Portrait commits `4575124` (build) + `6ffde23` (hardening).
>
> **M3 hardening (2026-06-29).** The real tool (z3 4.15.4) was re-run
> against the three *new* classes to check for a false-`PROVED`. **None
> found.** The SAT(T) vacuity guard holds UNIFORMLY across all four classes,
> incl. the new range and refinement classes (a contradictory guard yields
> `UNKNOWN`, never `PROVED`). Each new class is confirmed **non-vacuous** (it
> REFUTES a genuine violation, over a *satisfiable* `T`): range REFUTES an
> overflow/underflow, invariant-preservation REFUTES a `bounded_supply`
> overshoot, and refinement REFUTES an unbounded `spending_cap`. One honest
> coverage nuance closed: the refinement-class REFUTED direction for the two
> named invariants (`non_negative_amount`, `spending_cap`) is **pre-empted
> end-to-end by `portrait-sema`'s structural checks** (sema rejects the unguarded
> covenant before Lens — defence in depth), so it is exercised at the
> VC-*builder* level (parse → build the refinement VC directly, sema-bypassed)
> rather than through `prove_program`; a mutation test (assert φ instead of ¬φ)
> confirms the demonstration is non-vacuous. Real-covenant headline preserved:
> on `InternalSplit.rebalance`, value-conservation **PROVES** and range-overflow
> **REFUTES** as distinct per-class verdict lines.
>
> **STATUS UPDATE (2026-06-29) — M4: counter-model validation + unsat-core.**
> Two additions, both strictly on the *non-PROVED-soundness* side (the
> PROVED/UNKNOWN soundness of M1–M3 is **unchanged**; M4 only adds trust to the
> REFUTED side and explainability to the PROVED side). This directly closes the
> two open asks the M0 review-gate text below records — §6 R3 ("filter for
> over-approximation-spurious counter-models") and the §7 ask "how should a
> not-yet-replayed `REFUTED` be presented so it is not mistaken for a confirmed
> one".
>
> 1. **Counter-model validation (REFUTED trust).** When z3 returns `sat`
>    (REFUTED), Lens no longer blindly trusts it. It parses the model's concrete
>    integer/boolean assignment and **independently replays** every asserted term
>    (guards, next-state body, negated VC) in Rust over interpreted integer
>    arithmetic. If all guards hold and the VC is genuinely violated using ONLY
>    interpreted values ⇒ the REFUTED is marked **CONFIRMED** (a validated
>    witness). If the witness relies on an UNINTERPRETED-function value
>    (`checkSig` / `blake2b` / opaque `acc_*` selector — values that need not
>    correspond to a real on-chain execution) ⇒ it is marked **CANDIDATE**
>    (unvalidated, a possible over-approximation artifact), reported but flagged,
>    never silently confirmed. The validator is **fail-closed toward CANDIDATE**:
>    any missing model value or non-integer evaluation degrades to CANDIDATE; a
>    CONFIRMED is earned only by a complete interpreted replay. `portrait prove`
>    prints `[refuted]` + "(CONFIRMED…)" vs `[refuted?]` + "(CANDIDATE,
>    unvalidated: …)". Verified live (z3 4.15.4): a pure-integer value-creating
>    flow → CONFIRMED; the same flow behind a `checkSig` guard → CANDIDATE naming
>    `checkSig`.
> 2. **Unsat-core (PROVED explainability).** On PROVED, Lens re-asks z3 with named
>    assertions + `:produce-unsat-cores` and reports the `(get-unsat-core)` — WHICH
>    assertions (named guards / domain axioms / negated VC) the proof relied on.
>    This is **explainability ONLY**, best-effort: it never changes what PROVED
>    means and is omitted gracefully when z3 cannot produce a core. Verified live:
>    `InternalSplit.rebalance` value-conservation PROVED reports a non-empty core
>    naming the next-state equalities + the negated VC.
>
> Both are dep-free (a hand-rolled S-expression reader + integer evaluator in the
> crate; z3 stays an external runtime binary). It still proves the Portrait
> **MODEL**, not the emitted `.sil`; CONFIRMED strengthens trust in a REFUTED and
> flags over-approximation, but does **not** change the model-vs-`.sil` boundary.
>
> **M4 verification (2026-06-29).** M4 was exercised via
> the real `portrait prove` over 10+ crafted covenants. The dangerous direction
> (a CONFIRMED that is not a genuine violation) did **not** reproduce: every
> CONFIRMED witness produced was hand-verified to violate the property over
> integers (value-creation, `*2` doubling, `!=`-guarded, multi-leg flows; negative
> and `(- n)` literals parse correctly; the replay re-checks the negated VC, so a
> CONFIRMED is a real model-level violation). The vacuity guard, z3-absent⇒UNKNOWN,
> and PROVED + honest unsat-core all held unchanged. One **honesty caveat** is
> documented, not a soundness bug: any signature-gated covenant (almost all real
> ones) surfaces a real, integer-replayable REFUTED as **CANDIDATE** solely
> because a `checkSig` guard assert applies an uninterpreted function — even when
> `checkSig = true` is a realizable on-chain value and the violation does not
> actually depend on it (e.g. `MultisigTreasury.spend` range-overflow with a
> pre-state balance at `2^64`). CANDIDATE under-claims trust (never over-claims)
> and still flags failure (`any_refuted = true`, exit 1), so this is a usability /
> wording trade-off, not unsoundness.
>
> **Re-verification (2026-06-29).** Testing surfaced **no** unsoundness
> (the one remaining caveat is the CANDIDATE over-caution
> above — sound, under-claims), so no soundness fix was required. The four
> invariants were independently re-checked with the real
> `portrait prove` + z3 4.15.4: correct `InternalSplit` ⇒ **PROVED** with an
> honest unsat-core (`a4/a5/a6_eq_*` next-state equalities + `a8_not_*` negated
> VC, guards omitted); a sig-free pure-integer leak ⇒ **REFUTED + CONFIRMED**,
> with the conservation witness hand-verified as a genuine violation
> (`pool_a'+pool_b' = 1 ≠ 0 = pool_a+pool_b`); contradictory guards
> (`x>=5 ∧ x<=3`) ⇒ **UNKNOWN** (vacuous, never PROVED/CONFIRMED); the
> sig-gated REFUTED ⇒ **CANDIDATE** naming `checkSig`; the model-vs-`.sil`
> boundary footer prints on every report. Gates green: `cargo fmt --check`
> clean, `cargo clippy --workspace --all-targets -D warnings` clean, full
> `cargo test --workspace` = **290 passed / 0 failed**; portrait-lens stays
> dep-free (only `portrait-syntax` + `portrait-sema`).
>
> **M0 design artifact — encoding spec for external PL/FM review.**
>
> This is the M0 reviewable encoding spec. It specifies *precisely* how the
> Portrait surface AST would be
> encoded into SMT-LIB, what verification conditions (VCs) would be discharged,
> and — most importantly — **why an `unsat` answer is sound** (i.e. why a Lens
> `PROVED` means what it says). It exists to be **audited by a programming-
> languages / formal-methods professional before any solver code is written**
> (Proposal §5, "M0 — review gate #1"; §7 review ask items 1, 3, 4).
>
> **Status update (2026-06-29): the value-conservation VC class IS now built**
> (M1/M2) in the `portrait-lens` crate — `portrait prove FILE` generates the
> total-value-conservation VC described in §4(a), emits this SMT-LIB, and
> discharges it via the real `z3` binary (dep-free shell-out): `unsat`→PROVED,
> `sat`→REFUTED+counter-model, anything else→UNKNOWN. Verified live (z3 4.15.4):
> correct InternalSplit.rebalance → PROVED; the §6.2 broken variant → REFUTED with
> the `x=1,y=0` counter-model; `x*2 == x+x` → PROVED (beyond structural D4); no
> false-PROVED. It proves the Portrait **model** (T = G ∧ s'=⟦return⟧), **not** the
> emitted `.sil`, sound only under assumptions A1–A4. **Still proposed / NOT built:**
> the other three VC classes — refinement (§4b), range/overflow (§4c), invariant
> preservation (§4d) — and the spend VC (§Q3, needs `spent_out`); those remain
> M3+/future. So below, "would"/"Lens encodes" is **shipped** for §4(a) value
> conservation and **proposed** for everything else. The grammar, structural checks
> (C1/C2/C3/D4), and the worked-example covenants exist in the tree (file+line cited).
>
> **Status update (2026-06-29, M5): the SPEND VC class is now built (Q3 closed,
> honestly scoped), and a SOLVER CROSS-CHECK now guards every PROVED.** A clean
> value-out spend (e.g. `MultisigTreasury.spend`) is no longer deferred: Lens
> introduces a fresh SMT var `spent_out` bound to the drop `Σf − Σf'` and proves
> the OBLIGATION `spent_out >= 0` — i.e. the model spend creates NO value (the
> state total does not increase). Negate-and-check: assert `T ∧ (< spent_out 0)`
> [value created]; `unsat` ⇒ PROVED, `sat` ⇒ REFUTED + counter-model, routed
> through the SAME SAT(T) vacuity guard + M4 counter-model validation. Verified
> live (z3 4.15.4): `MultisigTreasury.spend` ⇒ PROVED; a spend whose body can
> mint value (free, unbounded `amount`) ⇒ REFUTED; a self-contradictory-guard
> spend ⇒ UNKNOWN (vacuous). HONEST SCOPE: this proves the MODEL spend mints
> nothing; it does **NOT** bind `spent_out` to the actual on-chain output amount
> (the covenant model does not read UTXO coin values) — that binding remains
> translation-validation (the model/`.sil` boundary, §7 / M6). So Q3 is answered
> "meaningful at the model level as a no-value-created obligation; the output
> binding stays translation-validation," not by reading coin values.
>
> The CROSS-CHECK reduces (does not discharge) assumption **A3** ("trust z3
> once") in a LIMITED sense: on a candidate PROVED (negated VC `unsat` + `T` alone
> `sat`), Lens re-runs the SAME negated-VC query through the SAME z3 binary under a
> PERTURBED configuration — non-default `sat.random_seed`/`smt.random_seed` AND a
> reordered assertion set — and reports PROVED ONLY if that re-run also returns
> `unsat`; a disagreement DOWNGRADES to UNKNOWN (never a single-run PROVED).
> HONEST SCOPE OF THE CROSS-CHECK (2026-06-29): this is NOT a second
> *independent* solver and NOT a proof-certificate check. Both runs share the same
> binary, version and decision procedures, so a *deterministic* z3 soundness bug (a
> wrong `unsat`) would reproduce IDENTICALLY in both and pass the cross-check. What
> it actually catches is search-order *instability* — a wrong answer that depends
> on seed or assertion order. So A3 is reduced (one narrow failure class is
> screened), not eliminated; a verified kernel / proof certificate remains future
> work. The CLI no longer describes the second run as "independently-configured";
> it states the same-binary/perturbed-config scope inline on every PROVED line.
>
> Maturity discipline: pre-production, unaudited, testnet-only posture.

> **STATUS UPDATE (2026-06-29) — M6: STRUCTURAL translation validation across the
> model-vs-`.sil` gap (first bridge of the honest #1 gap).** The boundary the rest
> of this spec defers ("Lens proves the model, NOT the emitted `.sil`"; §7) now has
> a first, deliberately STRUCTURAL bridge: `portrait_lens::validate_translation`
> (also `portrait validate-translation FILE`). For each transition entrypoint it
> derives the model fact set from the AST (one guard predicate per `requires`; the
> value-bearing field writes of the next-state return, skipping pure carries) and
> parses the emitted `.sil`'s matching `function` block (`require(...)` clauses +
> `return({...})` writes), then reports **CORRESPONDS** iff (1) every model guard
> maps to a `.sil` `require` — no guard silently dropped in emission — AND (2) the
> model and the `.sil` agree on every value-bearing write **in both directions**:
> the `.sil` adds no value-bearing write the model did not account for (no
> unaccounted mint / extra output) AND every model value-bearing write appears in
> the `.sil` and is not dropped or replaced by a pure carry (no value lost / source
> leg never moved) — else **DIVERGES** naming each divergence. A duplicate
> (shadowing) same-name `function` block is itself reported, since a single-block
> extractor would otherwise miss the shadow. Comparison is modulo a FIXED syntactic
> normalization (strip the engraver's `prev_states[0].` prefix; re-parse + re-render
> through the same canonical `Expr::to_silverscript`, robust to grouping parens /
> whitespace). Verified live: `CsciInstrument` ⇒ CORRESPONDS (the engraver-injected
> `require(proof_cov_id == OpInputCovenantId(0))` is an *added* guard, correctly NOT
> a divergence). Non-vacuity pinned by mutation tests — a `.sil` that drops a guard,
> adds/alters a value-bearing write, **drops a model value-bearing write or
> substitutes it with a pure carry** (money-printing shape), or duplicates an
> entrypoint block ⇒ DIVERGES naming it. HONEST SCOPE: this is a STRUCTURAL
> correspondence, **NOT** a semantic refinement proof — it catches emitter drift of
> those decidable shapes; it does NOT give the `.sil` a formal semantics, does NOT
> relate the two as a simulation/refinement, and does NOT prove behavioural
> equivalence. A genuine refinement still needs a `.sil` operational semantics + an
> SMT refinement obligation — the remaining deeper step, still future work. dep-free
> (parses `.sil` text + reuses the AST); pre-production, unaudited, testnet-only.

---

## 0. Scope, non-goals, and how to read this document

**In scope (what this spec pins down):**

1. A total, mechanical map from every `Expr` / `Stmt` / `ReturnExpr` AST node
   (as defined in `portrait-syntax`) to an SMT-LIB term or assertion, with the
   SMT theory each lands in (§2).
2. The transition-relation encoding `T(s, p, s')` for a covenant entrypoint (§3).
3. The verification-condition catalogue Lens discharges, each via the
   negate-and-check protocol (§4).
4. The over-approximation soundness argument: why `unsat ⇒ property holds`, and
   why every conservative choice fails toward `UNKNOWN`, never toward a false
   `PROVED` (§5).
5. Two end-to-end worked examples on real library covenants (§6).
6. The model-vs-emitted-silverscript soundness boundary, and the explicit
   open questions a reviewer must close before M1 (§7, §8).

**Out of scope / non-goals (stated so the reviewer knows the edges):**

- **No general program verification.** Lens targets value-conservation,
  refinement, and range VCs over the *small, closed* covenant grammar — not
  arbitrary safety properties (Proposal §6 R6).
- **No proof that the emitted `.sil` matches the model.** Lens proves a property
  of the **Portrait model**; translation validation between model and emitted
  silverscript is future / out of scope (§7; Proposal §3.3, R5).
- **No claim about the chain.** A Lens `PROVED` is not "the deployed covenant is
  safe." See §7.
- **No floating point, no strings, no general recursion.** The grammar has none.

**Notation.** `⟦e⟧` is the SMT term denoting expression `e`. SMT-LIB logic names
(`QF_LIA`, `QF_NIA`, `QF_BV`, `QF_AUFLIA`) follow SMT-LIB 2.6. `s` is the
pre-state tuple, `p` the parameter tuple, `s'` the post-state tuple. "Sort" is
the SMT-LIB word for type.

---

## 1. Grounding: the exact AST and the checks Lens supersedes

Everything below is grounded in source read read-only on 2026-06-29. The
encoding targets exactly these definitions; if they change, the encoding must be
revisited.

### 1.1 The surface AST (`portrait-syntax/src/lib.rs`)

- `Expr` (`lib.rs:138-165`):
  `Int(i64)`, `Bool(bool)`, `Bytes(Vec<u8>)`, `Var(String)`,
  `Field { base, field }`, `Index { base, index }`, `Unary { op, rhs }`,
  `Binary { op, lhs, rhs }`, `Call { name, args }`.
- `BinOp` (`lib.rs:84-97`): `Add, Sub, Mul, Eq, Ne, Ge, Le, Gt, Lt, And, Or`.
- `UnOp` (`lib.rs:119-123`): `Neg, Not`.
- `Stmt` (`lib.rs:76-81`): `Require(Expr)`, `Return(ReturnExpr)`, `Raw(String)`.
- `ReturnExpr` (`lib.rs:208-217`): `Scalar(Expr)` or
  `Object { name: Option<String>, fields: Vec<(String, Expr)> }`.
- `Type` (`lib.rs:42-53`): `Int, Bool, PubKey, Sig, Bytes32, Coin, Set(_),
  Map(_, _), Named(_)`. (`Map` cannot reach a covenant: it is rejected at parse
  in `parse_state_block`, `lib.rs:624-631`.)

This is a **small, closed grammar**. The genuinely hard cases for an SMT
encoding are exactly three nodes — `Mul`, `Call`, `Index`/`Field` — and each is a
*bounded, enumerable* situation, which is the feasibility lever (Proposal §1.4).

### 1.2 The structural checks Lens escalates (`portrait-sema/src/lib.rs`)

Lens does not replace `portrait-sema`; it slots **behind** it as an opt-in
`--prove` stage (Proposal §2.5). The two checks Lens supersedes — and whose
documented limitations motivate it — are:

- **C1 `value_conserved`** — `check_c1_value_conservation` (`lib.rs:558`),
  `is_conservation_preserving` (`lib.rs:533`). A **per-field structural shape**
  guard: each value-bearing field's new value must be a bare carry `f: f` or a
  single additive `f: f ± e`. The module header is explicit that C1 **checks each
  field in isolation and never sums deltas across fields** (`lib.rs:25-46`) and
  does **no arithmetic reasoning** — it cannot see underflow/overflow, and it
  does not know `x * 2 == x + x` (`lib.rs:47-49`).
- **D4 `conservation_split`** — `check_conservation_split` (declared at
  `lib.rs:213, 231`; semantics documented `lib.rs:990-1033`). The opt-in invariant
  that closes C1's cross-field gap **structurally**: it flattens each field's
  additive delta into `+`-separated atoms and requires the multiset of added
  atoms to equal the multiset of subtracted atoms **by `Expr` structural
  equality**. Its own header states it is **"STRUCTURAL N-field additive-delta
  arithmetic, NOT an SMT proof"** (`lib.rs:39-46, 1020-1033`): no numeric
  reasoning, no conditionals, no arithmetic identities, internal flows only.

The C2 capability check (`check_c2_authorization`, `lib.rs:659`) and the C3/D3
refinements (`check_c3_refinements`, `lib.rs:871`) are also structural pattern
matches; Lens turns the *refinement* ones into real implications (§4(b)). C2's
"committed key" notion (`pubkey_is_committed`, `lib.rs:782`) is reused by Lens
only as metadata, not re-proved — capability is a syntactic, not arithmetic,
property and stays in sema.

---

## 2. AST → SMT-LIB term mapping

### 2.1 Sorts (how a `Type` becomes an SMT sort)

| Portrait `Type` | SMT sort | Notes |
|---|---|---|
| `Int` | `Int` (or `(_ BitVec w)` in bounded mode, §2.4) | Mathematical integer by default. |
| `Coin` | `Int` **with a non-negativity side constraint** `v >= 0` asserted for every coin-sorted variable | Coin amounts are non-negative by domain; the constraint is part of the pre-state axioms (§3.3). |
| `Bool` | `Bool` | |
| `Bytes32`, `PubKey`, `Sig` | an **uninterpreted sort** `Bytes32` / `PubKey` / `Sig` | Opaque: only `=` / `distinct` are available. No structure is assumed. |
| `Set(T)` | uninterpreted sort `Set_T` (M0: opaque; no membership theory) | Sets do not appear in any value-conservation VC; left opaque, fail toward `UNKNOWN`. |
| `Map(K,V)` | — | **Unreachable in a covenant** (rejected at parse, `lib.rs:624`). Lens never sees one. |
| `Named(_)` | uninterpreted sort named after the identifier | Opaque user type; opaque sort. |

The implicit `prev_states: State[]` binding (introduced by sema's `TyEnv`,
`lib.rs:334`) is modelled as an SMT array `(Array Int State)` where `State` is a
record sort whose selectors are the role's declared state fields (§2.3, `Index` /
`Field`).

### 2.2 Expression encoding `⟦·⟧` (the core table)

`⟦e⟧` is defined by structural recursion over `Expr`. Each row names the SMT-LIB
construct and the **theory** the row commits to. The *whole-entrypoint* logic is
the join (least upper bound) of the theories of all rows it uses (§2.5).

| `Expr` node | `⟦node⟧` (SMT-LIB) | Theory contributed |
|---|---|---|
| `Int(n)` | the numeral `n` | QF_LIA (free) |
| `Bool(b)` | `true` / `false` | core |
| `Bytes(bs)` | a fresh constant of sort `Bytes32`, **interned by byte-value** (equal literals ⇒ same constant) | EUF (uninterpreted) |
| `Var(x)` | the SMT constant declared for `x` (pre-state field, param, or arg — §3.1) | — |
| `Binary{Add, a, b}` | `(+ ⟦a⟧ ⟦b⟧)` | **QF_LIA** |
| `Binary{Sub, a, b}` | `(- ⟦a⟧ ⟦b⟧)` | **QF_LIA** |
| `Binary{Mul, a, b}` | if one of `a,b` is `Int(k)`: `(* k ⟦other⟧)` (**stays QF_LIA** — linear) · else: `(* ⟦a⟧ ⟦b⟧)` (**QF_NIA**), or bounded `bvmul` in `QF_BV` mode (§2.4); else `UNKNOWN` (§2.5, §5.4) | QF_LIA / QF_NIA / QF_BV |
| `Binary{Eq, a, b}` | `(= ⟦a⟧ ⟦b⟧)` | core + operand theory |
| `Binary{Ne, a, b}` | `(distinct ⟦a⟧ ⟦b⟧)` | core + operand theory |
| `Binary{Lt, a, b}` | `(< ⟦a⟧ ⟦b⟧)` | QF_LIA (operands int) |
| `Binary{Le, a, b}` | `(<= ⟦a⟧ ⟦b⟧)` | QF_LIA |
| `Binary{Gt, a, b}` | `(> ⟦a⟧ ⟦b⟧)` | QF_LIA |
| `Binary{Ge, a, b}` | `(>= ⟦a⟧ ⟦b⟧)` | QF_LIA |
| `Binary{And, a, b}` | `(and ⟦a⟧ ⟦b⟧)` | core |
| `Binary{Or, a, b}` | `(or ⟦a⟧ ⟦b⟧)` | core |
| `Unary{Neg, e}` | `(- ⟦e⟧)` | QF_LIA |
| `Unary{Not, e}` | `(not ⟦e⟧)` | core |
| `Field{base, field}` | **uninterpreted select**: if `base` is `prev_states[i]`, `(field_sel (select prev_states i))` where `field_sel` is the record selector for `field`; otherwise a fresh uninterpreted function `acc_<field>(⟦base⟧)` interned by `(base, field)` syntactic key (§2.3) | EUF / arrays (QF_AUFLIA) |
| `Index{base, index}` | `(select ⟦base⟧ ⟦index⟧)` (theory of arrays) | arrays |
| `Call{name, args}` | if `name ∈ ModelledBuiltins` (§2.3): the modelled term/axiom; else a **fresh uninterpreted function** `name(⟦args⟧)`, same name+arity+args ⇒ same term | EUF (+ axioms only for allow-listed builtins) |

Note `Set` literals, `Map`, and `Named` values never appear as the operand of an
arithmetic or comparison node in any value-conservation VC; if one does appear
(e.g. an exotic refinement), it is opaque and the VC degrades to `UNKNOWN`.

### 2.3 The three soundness-critical corners: `Field`, `Index`, `Call`

These are where a naive encoding silently lies, so each is **conservative by
construction** (Proposal §2.3):

- **`Field`** — encoded as an **uninterpreted selector**. The only fact the
  solver has about `acc_balance(x)` is *functional consistency*: the same
  syntactic access yields the same term (`acc_balance(x) = acc_balance(x)`). It
  may **not** assume any relationship between distinct accesses. The one
  exception is the modelled `prev_states[i].field` shape, which is a genuine
  record selector over the array element (this is sound because `prev_states` is
  the role's own committed state with known fields — sema already resolves these
  in `TyEnv.state_fields`, `lib.rs:306, 446`).
- **`Index`** — `(select arr idx)` under the **theory of arrays** (read-over-write
  axioms only). No assumption about the contents of an unread cell.
- **`Call`** — by default a **pure uninterpreted function**: `f(a) = f(a)` and
  nothing more. A **short, audited allow-list** `ModelledBuiltins` may attach
  axioms. M0 ships this allow-list **EMPTY by intent**, with two *candidate*
  entries written down for the reviewer to accept or reject — each candidate is
  a trust assumption the reviewer signs (§7 open question Q4):

  | Candidate builtin | Proposed axiom / model | Why it is a trust assumption |
  |---|---|---|
  | `checkSig(sig, key)` | **left uninterpreted** (Bool-valued UF). NOT given `true`. | Capability is sema's job (C2). Lens must not assume a signature *checks out*; modelling it as UF means a VC that depends on `checkSig` being true is provable only if the guard asserts it, exactly as the covenant does. |
  | `blake2b(x)` | uninterpreted, **injective-on-demand** only if the reviewer accepts collision-resistance as an axiom (`blake2b(a)=blake2b(b) ⇒ a=b`). M0 default: plain UF, no injectivity. | Injectivity is a cryptographic assumption; encoding it as a logical axiom is unsound if used to "prove" a value property. Default OFF. |

  The governing rule (Proposal §2.3): **when in doubt, stay uninterpreted and
  return `UNKNOWN`.** A `PROVED` must mean *proved under an encoding the reviewer
  validated*, never *proved because the encoding quietly over-assumed*.

### 2.4 Bounded / bit-vector mode (optional, for `Mul` and range VCs)

For the range/overflow VCs (§4(c)) and for non-constant `Mul`, Lens may switch a
field to a **fixed-width bit-vector** sort `(_ BitVec w)` (`w` = the on-chain
integer width the engraver targets), with `bvadd`/`bvsub`/`bvmul` and explicit
overflow predicates (`bvuaddo`, `bvusubo`, …). This makes overflow reasoning
*exact* and keeps non-constant `Mul` **decidable** (QF_BV is decidable, unlike
QF_NIA). The trade-off (cost, and the choice of `w`) is a reviewer question
(§8 Q2). Bounded mode is **opt-in per VC**; the default integer mode is used
where overflow is not the property under test.

### 2.5 Theory selection per entrypoint, and the `UNKNOWN` ladder

Per entrypoint, Lens computes the **least logic** that covers every term it
emits:

```
core ⊑ QF_LIA ⊑ QF_AUFLIA            (add Field/Index/Call → arrays+EUF)
QF_LIA ⊑ QF_NIA                       (non-constant Mul, integer mode)
QF_LIA ⊑ QF_BV / QF_ABV               (bounded mode)
```

- All-`Add`/`Sub`/comparison covenants → **QF_LIA**, decidable and fast. This is
  the overwhelming majority of compliance-pattern covenants (Proposal §2.2).
- A `Field`/`Index`/`Call` lifts to **QF_AUFLIA** (still decidable; the UF/array
  parts are just opaque).
- A non-constant `Mul` in integer mode lifts to **QF_NIA** (undecidable in
  general): Lens sets a per-VC timeout; on timeout it returns **`UNKNOWN`**, never
  `PROVED` (§5.4, Proposal §6 R2). In bounded mode the same `Mul` is QF_BV
  (decidable) at the cost of fixed width.

`UNKNOWN` is a **first-class outcome**: it is reported as "outside Lens's decidable
fragment" (nonlinear, or opaque term the property genuinely depends on), and is
never silently upgraded to `PROVED`.

---

## 3. The transition-relation encoding `T(s, p, s')`

A covenant entrypoint is modelled as a **state-transition relation**
`T(s, p, s') ≡ G(s, p) ∧ (s' = ⟦return⟧)` (Proposal §2.1). This is a faithful,
mechanical reading of the AST: `Require`s are guards, the object return is the
next-state function.

### 3.1 Declaring the variables (typing pre-state, params, post-state)

For an entrypoint of role `R` with state fields `s = (f₁…fₙ)` and params
`p = (a₁…aₘ)`:

- **Pre-state** `s`: one SMT constant per state field, `(declare-const f_i <sort(τ_i)>)`,
  sort by §2.1. These are the bare names referenced in bodies (sema binds state
  fields as bare vars, lowered to `prev_states[0].field` by the emitter —
  `lib.rs:326-328`). So `Var("balance")` denotes the pre-state `balance`.
- **Params / args** `p`: one SMT constant per entrypoint arg,
  `(declare-const a_j <sort>)`. These are **caller-supplied and unconstrained**
  except by the guards `G`.
- **Post-state** `s'`: one fresh SMT constant per state field, `f_i'`.
- **`prev_states`**: `(declare-const prev_states (Array Int State))`, with the
  modelled access `prev_states[0].f = f` asserted (the bare-name convention), so
  `prev_states[0].balance` and the bare `balance` are the *same* term.

### 3.2 The guard `G(s, p)`

`G(s, p) ≡ ⋀_{Require(e) ∈ body} ⟦e⟧`. Each `Stmt::Require(e)` is a typed
boolean expression (sema guarantees `require` operands are `bool`,
`lib.rs:345-352`), so `⟦e⟧` is a Bool term and the conjunction is well-sorted.

A `Stmt::Raw` in a covenant entrypoint **cannot occur**: sema fail-closes on a
`Raw` in any non-`NonCovenant` entrypoint (`lib.rs:369-378`), so by the time Lens
runs (behind sema), every covenant-entrypoint statement is a typed
`Require`/`Return`. Lens asserts this as a precondition and refuses to run (not
`UNKNOWN`, a hard refusal) if it ever sees a `Raw` — there is no sound encoding of
an untyped hole.

### 3.3 The next-state function `s' = ⟦return⟧`

From `ReturnExpr::Object { fields }`: for each returned `(field, value)`,
assert `f' = ⟦value⟧`; **any state field not mentioned carries unchanged**:
`f' = f`. (This frame rule mirrors the emitter, which only rewrites referenced
fields.)

From `ReturnExpr::Scalar(expr)`: sema guarantees a scalar return references **at
most one** state field (`check_return`, `lib.rs:399-414`); that single field `f`
gets `f' = ⟦expr⟧` and all others carry unchanged. (If it references zero state
fields it is a pure verification value, not a mutation, and there is no `s'`
constraint beyond the frame.)

**Domain axioms** (asserted unconditionally, part of the model, both for `s` and
`s'`): every `coin`-sorted variable `v` carries `v >= 0` (§2.1). These are sound
domain facts (coin amounts are non-negative on-chain), and they are asserted on
*both* pre- and post-state so a VC cannot be vacuously discharged by ignoring
them.

---

## 4. The verification-condition catalogue

Each VC is discharged by the **negate-and-check** protocol:

> Assert `T(s,p,s') ∧ ¬VC`. Ask the solver.
> `unsat` ⇒ **PROVED** (no transition satisfies the guards yet violates the VC).
> `sat` ⇒ **REFUTED**, and the satisfying model is the **counter-example**
> (concrete field/param values that fire the guards and break the VC).
> `unknown` / timeout ⇒ **UNKNOWN** (first-class, §2.5, §5.4).

The four VC classes (Proposal §2.4):

### (a) Total value conservation — the headline (supersedes C1 + D4)

Let `V` be the set of **value-bearing** state fields. Note sema has **two**
value-bearing predicates and Lens must use the **wider** one for the
conservation-field set, because the narrower one would silently empty `V` for
multi-leg covenants:

- `is_value_bearing` (`lib.rs:499-501`) — declared type `coin`, or name in
  `{balance, amount, supply}` (`VALUE_BEARING_NAMES`, `lib.rs:497`). This is the
  rule **C1** uses per-field.
- `is_value_bearing_split` (`lib.rs:1054`) — `is_value_bearing(...) ||
  name.ends_with("balance")`. This is the rule **D4** (`conservation_split`)
  uses, and it is the one that catches multi-leg fields such as
  `pool_a_balance` whose names *end in* `balance` but are not exactly `balance`
  and are not `coin`-typed.

Because Lens **supersedes C1 + D4**, `V` is defined by `is_value_bearing_split`
(the D4 rule), i.e. `V = { f : is_value_bearing(f.name, f.ty) ∨
f.name.ends_with("balance") }`. Using the narrower `is_value_bearing` here would
make `V = ∅` for a covenant like `InternalSplit` (whose legs are `int`-typed and
named `pool_*_balance`), collapsing `Σ_{f∈V}` to the vacuous `0 = 0` and
**spuriously PROVING** any rebalance — the exact failure mode worked example
§6.2 is designed to refute. (This is a precision point a reviewer would catch;
it is called out as part of Q6, VC-catalogue completeness.)

- **Internal-flow transitions** (no value leaves the covenant):
  `VC_cons ≡ (Σ_{f ∈ V} f') = (Σ_{f ∈ V} f)`.
- **Spend transitions** (value leaves to a declared external output `spent_out`):
  `VC_cons ≡ (Σ_{f ∈ V} f) − (Σ_{f ∈ V} f') = spent_out`, where `spent_out` is
  the entrypoint's declared external-output amount (M0: the `amount` arg of a
  spend; the precise binding of `spent_out` to the emitted output is a reviewer
  question, §8 Q3, and is the model/script boundary, §7).

Because this is **real arithmetic over the actual delta terms** (not a shape
match), Lens **does** see `x * 2 == x + x`, **does** case-split conditionals
(via the guard conjunction / branch terms), and produces a **concrete
counter-model** when conservation fails — the substance D4 cannot reach
(`lib.rs:1027`).

The mint/burn exemption (sema's `is_mint_or_burn`, `lib.rs:505`) carries over:
an entrypoint named `mint…`/`burn…` is an authorised supply change and `VC_cons`
is **not** asserted for it (it is replaced by the relevant bounds VC, §4(c)).

### (b) Refinements as implications `G ⟹ refinement`

The opt-in refinements that today are **narrow structural matches** (C3/D3,
`lib.rs:871-988`) become **real implications**, each `VC ≡ G(s,p) ⟹ φ`:

| Declared invariant (sema) | Lens VC `G ⟹ φ` |
|---|---|
| `non_negative_amount` (`lib.rs:905`) | `G ⟹ amount >= 0` |
| `bounded_supply` (`lib.rs:918`) | `G ⟹ supply' <= total` (post-state ceiling) |
| `spending_cap` (`lib.rs:931`) | `G ⟹ amount <= limit` |
| `monotonic_seq` (`lib.rs:891`) | `G ⟹ seq' = seq + 1` |
| `temporal_guard` (`lib.rs:966`) | `G ⟹ now_bucket >= deadline` (or `>= last_active + timeout`) |

`multisig_threshold` and the C2 capability property stay **syntactic** (sema);
they are not arithmetic facts and Lens does not re-prove them (it relies on the
sema pass having passed, and records that as an assumption).

### (c) Range / overflow obligations

For each `Sub`/`Add` on a bounded field, a per-operation obligation
(bounded mode, §2.4): `VC ≡ G(s,p) ⟹ (operation does not overflow/underflow)`,
e.g. for `balance - amount`: `G ⟹ amount <= balance` (no underflow), and for
`supply + amount`: `G ⟹ supply + amount <= MAX` (no overflow at width `w`).
This directly answers the C1 gap at `lib.rs:47-49` ("does not know `balance -
amount` can underflow").

### (d) Invariant preservation `I(s) ∧ G ⟹ I(s')`

A user-declared arithmetic invariant `I` (e.g. a custom ceiling `balance <=
cap`) is **preserved across the transition**: `VC ≡ I(s) ∧ G(s,p) ⟹ I(s')`.
This is the classic inductive-invariant obligation; it composes with (a)–(c).

---

## 5. Over-approximation soundness argument

This is the heart of the M0 review. The claim to defend:

> **Soundness claim.** For every VC `φ` and every covenant entrypoint, if Lens
> reports `PROVED` (i.e. the solver returns `unsat` for `T(s,p,s') ∧ ¬φ`), then
> `φ` holds for **every** real execution of the covenant **model**.

The argument has the standard shape of an abstraction-soundness proof: the SMT
encoding is a **sound over-approximation** of the model's concrete semantics, so
`unsat` of a negated VC transfers to the concrete level. We give it in four
parts, with the assumptions made explicit.

### 5.1 Concrete semantics (the thing being over-approximated)

Define a *concrete model semantics* for the covenant: a concrete state is an
assignment of values (mathematical integers for `int`/`coin`, booleans, and
opaque values for `bytes32`/`pubkey`/`sig`) to the role's state fields; a concrete
*execution* of entrypoint `E` is a triple `(σ, ν, σ')` of pre-state, param
valuation, and post-state such that every `require` evaluates to `true` under
`(σ, ν)` and `σ'` is the value of the return object under `(σ, ν)` (unmentioned
fields framed). Let `⟦E⟧_C` be the set of all such concrete executions. *(This is
a definition the reviewer is asked to confirm matches intended Portrait
semantics — §8 Q1.)*

### 5.2 The encoding admits a superset of concrete executions

Let `⟦E⟧_S` be the set of `(s, p, s')` valuations satisfying the SMT relation
`T(s,p,s')`. **Lemma (over-approximation):** every concrete execution maps to a
model of `T`, i.e. there is an injection `⟦E⟧_C ↪ ⟦E⟧_S` (and `⟦E⟧_S` may contain
*extra* valuations that no concrete execution realises). Proof obligation,
discharged node-by-node by the encoding table (§2.2):

1. **Integer / boolean nodes are exact.** `Add/Sub/Neg`, all comparisons, and
   `And/Or/Not` are encoded by the *same* operation in the theory of integers /
   booleans. For these nodes `⟦e⟧` evaluates, under any model, to the same value
   the concrete semantics computes. (Exact ⇒ over-approximation a fortiori.)
2. **Uninterpreted nodes lose information in the safe direction.** `Field`,
   `Index`, `Call` (non-allow-listed), and opaque-sorted values
   (`bytes32`/`pubkey`/`sig`/`Set`/`Named`) are encoded as uninterpreted
   functions / fresh constants whose **only** law is functional consistency
   (equal syntactic terms ⇒ equal values). The concrete semantics assigns these
   *some* specific value; the SMT model is free to assign them *any* value
   consistent with the equalities actually asserted. Hence the SMT relation
   permits **at least** the concrete behaviour (and generally more). This is the
   crux: uninterpreted modelling can only **widen** the admitted set, never
   narrow it — so it cannot exclude a real counter-example, and cannot
   manufacture a proof by assuming a fact.
3. **`Mul` is exact or escalated.** Constant `Mul` (`* k e`) is linear and exact.
   Non-constant `Mul` is either encoded exactly (QF_NIA `*`, or QF_BV `bvmul` at
   the model's integer width) or, if neither is enabled / the solver times out,
   the **whole VC** is reported `UNKNOWN` (§5.4) — never `PROVED`. So whenever
   Lens *does* answer `PROVED`, the `Mul` was modelled exactly.
4. **Domain axioms are sound facts.** The coin `v >= 0` constraints (§3.3) are
   true of every concrete state (coin amounts are non-negative on-chain), so
   adding them excludes only *non*-concrete valuations — it preserves the
   over-approximation and does not drop any real execution.

By structural induction over `Expr`/`ReturnExpr` using (1)–(4), every concrete
execution `(σ,ν,σ') ∈ ⟦E⟧_C` yields a model of `T`, establishing the Lemma. ∎
*(sketch — full mechanisation is M1+ work; this is the argument the reviewer
audits.)*

### 5.3 Soundness of `PROVED` follows

Suppose Lens reports `PROVED` for VC `φ`: the solver found `T ∧ ¬φ` **unsat**,
i.e. **no** valuation in `⟦E⟧_S` violates `φ`. By the Lemma, `⟦E⟧_C ⊆ ⟦E⟧_S`, so
**no concrete execution violates `φ` either** — `φ` holds for every real
execution of the model. This is exactly the soundness claim. The direction is
the standard one: over-approximate the behaviours, prove the *negation*
unsatisfiable, and the property transfers downward. ∎

Note the asymmetry, which is the design's safety margin:

- A **`PROVED`** is sound (above).
- A **`REFUTED`** (sat) exhibits a model of `T ∧ ¬φ`. Because `⟦E⟧_S` can be a
  *strict* superset of `⟦E⟧_C`, a `REFUTED` counter-model *might* be a "spurious"
  over-approximation artifact (a valuation no concrete run reaches). Lens
  therefore treats a `REFUTED` as **"property not proved; here is a candidate
  counter-example to investigate,"** and the differential fuzz harness (Proposal
  §6 R3) checks whether the counter-model actually breaks the property when run
  concretely. **A spurious `REFUTED` is a usability cost, never a soundness
  bug** — the dangerous direction (a false `PROVED`) is the one the
  over-approximation rules out.

### 5.4 Every conservative choice fails toward `UNKNOWN`, never false `PROVED`

The explicit list of "I cannot model this precisely" situations and what each
does — all of them either widen the admitted set (safe, §5.2(2)) or bail to
`UNKNOWN`:

| Situation | Encoding response | Why it cannot cause a false `PROVED` |
|---|---|---|
| Non-constant `Mul`, integer mode, solver timeout | **UNKNOWN** (per-VC timeout) | No `unsat` is claimed; `PROVED` is never reported on timeout. |
| Opaque term (`Call`/`Field`/`Index`/exotic sort) the VC genuinely depends on | uninterpreted ⇒ solver typically returns `sat` (a model assigning the opaque term a property-breaking value) ⇒ **REFUTED/UNKNOWN**, not `PROVED` | Over-approximation (§5.2(2)): the opaque value is unconstrained, so a property *relying* on it cannot be `unsat`. |
| `Stmt::Raw` in a covenant entrypoint | **hard refusal to run** (§3.2) | Cannot occur (sema fail-closes); if it did, no encoding is emitted, so no `PROVED`. |
| Allow-listed builtin axiom in doubt | **default: no axiom** (§2.3) | Without the axiom the term is uninterpreted (safe direction). |
| Unsupported VC shape (outside §4 catalogue) | **not generated** (Proposal §6 R6) | A VC that is never asserted is never reported `PROVED`. |

**Assumptions made explicit (the reviewer must accept these for §5.3 to hold):**

- **A1.** The concrete semantics of §5.1 is the intended Portrait covenant model
  semantics (§8 Q1).
- **A2.** The encoding table (§2.2) is faithful for the exact nodes (integers,
  booleans) — i.e. SMT `+`/`<`/… mean what Portrait `+`/`<`/… mean over the same
  domain. (For mathematical-integer mode this is immediate; for bounded mode it
  requires the chosen width `w` to match the engraver's, §8 Q2.)
- **A3.** The solver is trusted: a reported `unsat` is a true `unsat` (Proposal
  §6 R4; proof-certificate checking is the M5 stretch to discharge A3).
- **A4.** Lens runs **behind** a passing `portrait-sema` (so: no `Raw` holes, all
  bodies typed, capability/threshold already checked syntactically). Lens does
  not re-establish A4; it depends on it.

If any of A1–A4 fails, the soundness claim is voided — which is precisely why
they are written here for sign-off.

---

## 6. Two worked examples, end-to-end

Both use **real library covenants** read on 2026-06-29. The SMT-LIB shown is the
*proposed* encoding (M0 — not produced by any tool).

### 6.1 PROVED — `MultisigTreasury.spend` value conservation

Source: `portrait/library/governance/treasury/MultisigTreasury.portrait`
(`spend`, lines 58-73). State: `pubkey signer_a, signer_b; int balance`. Args:
`sig auth_a, auth_b; int amount`. Guards: two `checkSig`, `amount >= 0`,
`amount <= balance`. Return: `{ signer_a: signer_a, signer_b: signer_b,
balance: balance - amount }`.

`balance` is the only value-bearing field (name in `VALUE_BEARING_NAMES`). This
is a **spend** (value leaves), so `VC_cons ≡ balance − balance' = amount`
(spent_out = `amount`, §4(a)/§8 Q3).

Encoding (`(set-logic QF_AUFLIA)` — UF for `checkSig`, LIA for the rest):

```smt2
(declare-sort PubKey 0) (declare-sort Sig 0)
(declare-fun checkSig (Sig PubKey) Bool)      ; uninterpreted (§2.3): NOT assumed true

; pre-state
(declare-const signer_a PubKey) (declare-const signer_b PubKey)
(declare-const balance Int)
; params
(declare-const auth_a Sig) (declare-const auth_b Sig) (declare-const amount Int)
; post-state
(declare-const signer_a_p PubKey) (declare-const signer_b_p PubKey)
(declare-const balance_p Int)

; T = G ∧ s'=⟦return⟧
(assert (checkSig auth_a signer_a))           ; require checkSig(auth_a, signer_a)
(assert (checkSig auth_b signer_b))           ; require checkSig(auth_b, signer_b)
(assert (>= amount 0))                        ; require amount >= 0
(assert (<= amount balance))                  ; require amount <= balance
(assert (= signer_a_p signer_a))              ; frame: signer keys carried
(assert (= signer_b_p signer_b))
(assert (= balance_p (- balance amount)))     ; balance: balance - amount

; ¬VC_cons : the spent-out delta is NOT exactly `amount`
(assert (not (= (- balance balance_p) amount)))

(check-sat)        ; ⇒ EXPECTED: unsat  ⇒  PROVED
```

Why `unsat`: from `balance_p = balance − amount`, `balance − balance_p =
balance − (balance − amount) = amount`, contradicting the negated VC. The two
`checkSig` UFs are irrelevant to conservation (they constrain capability, not
arithmetic) — which is correct: conservation should hold regardless of *who*
signs. Result: **PROVED** that the spend removes exactly `amount` and conserves
the rest. (D4/C1 only accept the *shape* `balance - amount`; Lens proves the
*arithmetic* delta, and would equally prove a `balance - (amount*2)/2` form that
C1's shape match cannot see.)

### 6.2 REFUTED — a deliberately broken `InternalSplit.rebalance`

Base source (correct): `portrait/library/finance/internal-split/InternalSplit.portrait`
(`rebalance`, lines 59-75). Three value-bearing legs `pool_a_balance,
pool_b_balance, pool_c_balance` — `int`-typed, names *end in* `balance`, so they
are caught by `is_value_bearing_split` (`lib.rs:1054`), **not** by the narrower
`is_value_bearing`; this is why `V` must be defined by the D4 rule, §4(a) — and
`pubkey owner`. Args
`sig auth; int x, y`. Guards: `checkSig(auth, owner)`, `x>=0`, `y>=0`,
`x + y <= pool_a_balance`. **Correct** return splits `(x+y)` out of leg a into
legs b and c — deltas `{−(x+y), +x, +y}` net to zero, so the correct version is
**PROVED** by `VC_cons ≡ Σ f' = Σ f` (internal flow).

**The injected bug** (a plausible fat-finger an author could ship): leg c gains
`x` instead of `y`:

```
pool_c_balance: pool_c_balance + x      // BUG: should be + y
```

Now deltas are `{ −(x+y), +x, +x }`, summing to `+x − y` ≠ 0. C1 would *accept*
both legs (each is a single additive `±` shape — `lib.rs:533`); D4's structural
atom-cancellation *would* catch this particular one, but it is blind to the
arithmetic-identity and conditional cases — Lens catches the whole class. Encoding:

```smt2
(set-logic QF_AUFLIA)
(declare-sort PubKey 0) (declare-sort Sig 0)
(declare-fun checkSig (Sig PubKey) Bool)
(declare-const pool_a_balance Int) (declare-const pool_b_balance Int)
(declare-const pool_c_balance Int) (declare-const owner PubKey)
(declare-const auth Sig) (declare-const x Int) (declare-const y Int)
(declare-const pool_a_balance_p Int) (declare-const pool_b_balance_p Int)
(declare-const pool_c_balance_p Int) (declare-const owner_p PubKey)

(assert (checkSig auth owner))
(assert (>= x 0)) (assert (>= y 0))
(assert (<= (+ x y) pool_a_balance))
(assert (= pool_a_balance_p (- pool_a_balance (+ x y))))   ; a: a - (x+y)
(assert (= pool_b_balance_p (+ pool_b_balance x)))         ; b: b + x
(assert (= pool_c_balance_p (+ pool_c_balance x)))         ; c: c + x  ← BUG
(assert (= owner_p owner))

; ¬VC_cons (internal flow): Σ s' ≠ Σ s
(assert (not (= (+ pool_a_balance_p pool_b_balance_p pool_c_balance_p)
                (+ pool_a_balance pool_b_balance pool_c_balance))))

(check-sat)        ; ⇒ EXPECTED: sat  ⇒  REFUTED
(get-model)        ; ⇒ a concrete counter-model, e.g. below
```

A satisfying model (the **counter-example** surfaced to the user), e.g.:

```
pool_a_balance = 10, pool_b_balance = 0, pool_c_balance = 0
x = 1, y = 0
⇒ pool_a_balance_p = 9, pool_b_balance_p = 1, pool_c_balance_p = 1
   Σ s' = 9+1+1 = 11  ≠  Σ s = 10+0+0 = 10        (value CREATED: +1)
```

(Any model with `x ≠ y` and the guards satisfied works; `x=1, y=0` is the
minimal witness — the guards `x>=0, y>=0, x+y<=pool_a_balance` are all met.)
Result: **REFUTED**, with the concrete inputs that mint value out of nothing —
the substance a shape check cannot produce. Per §5.3, the fuzz harness would
replay this model concretely to confirm it is a true (non-spurious) counter-
example before it is reported as a hard failure.

---

## 7. Soundness boundary: model vs. emitted silverscript

**Lens proves a property of the Portrait MODEL, not of the emitted `.sil`
script the chain enforces.** This boundary is stated loudly and is the single
most important caveat on any Lens output:

- The chain enforces the **emitted silverscript** produced by `portrait-emit`.
  Lens's soundness (§5) is about the **Portrait covenant model** (the AST-derived
  transition relation `T`). The two coincide **only to the fidelity of
  `portrait-emit`'s lowering**, which Lens does **not** verify.
- Concretely: even a sound Lens `PROVED` does **not** establish that the deployed
  covenant is safe. It establishes that *the model is correct with respect to the
  VC*. If the emitter mistranslates the model, a true `PROVED` can sit atop an
  unsafe script. (Proposal §3.3, §6 R5.)
- The honest closing of this gap is **translation validation** between the model
  and the emitted script — proving the emitter preserves the semantics the VC
  was proved over. This is **explicitly future / out of scope for Lens v0**
  (Proposal §5 M5, §6 R5). M0 names it; it does not attempt it.
- Therefore every Lens output must carry, on its face: *"Proven over the Portrait
  model under assumptions A1–A4; not a proof of the emitted script or the
  deployed covenant. Pre-production, unaudited, testnet-only."*

A second, narrower boundary: §4(a) for **spends** binds `spent_out` to the
entrypoint's external output. The covenant model does **not itself read UTXO coin
values** (MultisigTreasury's own header, lines 30-35: "The covenant does NOT
itself read UTXO coin values"). So the spend-conservation VC proves a property of
the *declared* `amount`, not of the actual coins moved — another model/chain seam
the reviewer must weigh (§8 Q3).

---

## 8. Open questions for the reviewer (must close before M1)

These are the specific things a PL/FM professional must validate or decide
before any solver code (M1) is written. They are the M0 review gate.

- **Q1 — Concrete semantics (§5.1).** Is the concrete model semantics
  (require-as-guard, object-return-as-next-state, unmentioned-fields-framed, bare
  state name = `prev_states[0].field`) a faithful statement of intended Portrait
  covenant semantics? Any divergence voids A1 and the §5.3 soundness transfer.
- **Q2 — Integer model: mathematical vs. bounded (§2.4, A2).** Should the default
  be mathematical `Int` (clean, but does not model on-chain overflow) or bounded
  `(_ BitVec w)` (models overflow exactly, needs a `w` matching the engraver)?
  What is the on-chain integer width `w`, and must the conservation VC and the
  range VC use the *same* mode to be jointly sound?
- **Q3 — `spent_out` for spends (§4(a), §7).** How should `spent_out` be bound to
  the real external output, given the covenant model does not read UTXO coin
  values? Is the §4(a) spend VC meaningful at the model level, or should spend
  conservation be deferred to translation validation (§7)?
- **Q4 — Modelled-builtins allow-list (§2.3).** Confirm the M0 default (EMPTY:
  `checkSig` uninterpreted, `blake2b` no injectivity). For each future candidate
  axiom, is it sound to assert as a logical axiom, and does any VC's `PROVED`
  *depend* on it (if so, the axiom enters the TCB and must be justified)?
- **Q5 — Theory / solver choice (§2.5; Proposal §6 R2/R4).** Is QF_LIA-baseline +
  QF_AUFLIA + (QF_NIA or QF_BV) the right ladder? Which solver (Z3 vs. a
  vetted alternative vs. Rust-native), and is proof-certificate checking (to
  discharge A3) worth pulling earlier than M5?
- **Q6 — VC catalogue completeness (§4).** Are the four VC classes (conservation,
  refinement-implication, range, invariant-preservation) the right closed set for
  v0? Is anything missing that a compliance covenant needs, or anything present
  that should be cut to hold scope (Proposal §6 R6)?
- **Q7 — Frame rule (§3.3).** Is "unmentioned state field carries unchanged
  (`f' = f`)" the correct frame for every covenant entrypoint, or are there
  entrypoints where an unmentioned field should be unconstrained?
- **Q8 — Spurious `REFUTED` (§5.3).** Is the differential-fuzz replay an adequate
  filter for over-approximation-spurious counter-models, and how should a
  not-yet-replayed `REFUTED` be presented so it is not mistaken for a confirmed
  bug?

---

## 9. Summary

This M0 spec pins the **AST → SMT-LIB encoding** for the proposed Lens proof
engine: a total node-by-node term map (§2) over the small closed Portrait grammar,
landing the common case in **decidable QF_LIA** and modelling the three hard
corners (`Field`, `Index`, `Call`) and nonlinear `Mul` **conservatively** so they
widen the admitted behaviour set or bail to **`UNKNOWN`** — never to a false
`PROVED`. Each covenant entrypoint becomes a transition relation
`T(s,p,s') ≡ G(s,p) ∧ s'=⟦return⟧` (§3); four VC classes (total value
conservation superseding C1/D4, refinement implications, range/overflow,
invariant preservation) are discharged by **negate-and-check** with `UNKNOWN` as
a first-class outcome (§4). The **over-approximation soundness argument** (§5)
shows, with assumptions A1–A4 made explicit, that a `PROVED` (`unsat` of the
negated VC) transfers to every concrete model execution, and that every
conservative modelling choice fails safe. Two **real-covenant worked examples**
exercise both verdicts: `MultisigTreasury.spend` ⇒ **PROVED**, a fat-fingered
`InternalSplit.rebalance` ⇒ **REFUTED** with a concrete value-creating
counter-model. The **model-vs-emitted-script boundary** (§7) is named loudly —
Lens proves the model, not the `.sil`; translation validation is future work —
and the **eight open questions** (§8) are the explicit M0 review gate.

**M0 design artifact — NOT implemented; no solver / dependency / code; for
external PL/FM review.** Nothing here is built, deployed, or claimed beyond the
structural checks already in the tree.
