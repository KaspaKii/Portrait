# Frontier complete — Lens + Composer feature-complete within scope

**Status:** pre-production, unaudited, testnet-only. Nothing deployed to
kaspanet. No KIP. This is a Foundation engineering record, intended to
be read alongside the M0 specs ([LENS-M0-ENCODING-SPEC.md](LENS-M0-ENCODING-SPEC.md),
[COMPOSER-M0-DESIGN.md](COMPOSER-M0-DESIGN.md)) and the two adversarial sweeps
([PORTRAIT-LENS-SWEEP.md](PORTRAIT-LENS-SWEEP.md),
[COMPOSE-SWEEP-FINDINGS.md](COMPOSE-SWEEP-FINDINGS.md)).

**Provenance (this program):**
- Portrait repo: `portrait/portrait` @ `ff3d1dc`
  (the only working-tree delta is the hardening fix to
  `crates/portrait-lens/src/translation.rs`).
- `kaspa-compliance-patterns` (this repo) @ `0249648`.
- Engine reference pinned: `rusty-kaspa` tag `v2.0.0` (= commit `90dbf07`),
  unchanged by this program.

This document states, without inflation, what the two frontier engines are, what
each milestone delivers, their **exact** soundness claims, the build→attack→fix
discipline that produced them, and the gaps that are genuinely out of reach here.

---

## 1. Lens — what it is and what it proves

**Lens** (`portrait-lens`) is the *verification* frontier: given a Portrait
covenant **model**, it generates verification conditions (VCs), discharges them
with z3, and reports honest verdicts. It is exercised in this program through the
real `portrait prove` and `portrait validate-translation` CLI commands against
`/opt/homebrew/bin/z3`.

### Milestones (M1–M6)

| M | Delivers |
|---|---|
| **M1** | **Conservation VC** — no value created/destroyed across a state transition for value-bearing fields. z3 discharge; `unsat` of the negated property → `PROVED`. |
| **M2** | **Range VC** — value/quantity fields stay within their declared `u64` sompi window (no overflow/underflow); a documented refinement of assumption A2. |
| **M3** | **Refinement VC** + **Invariant VC** — declared field refinements and covenant invariants are preserved by every transition. |
| **M4** | **Counter-model validation** — on a non-`PROVED` result, the model is replayed and classified **CONFIRMED** (a concrete, in-model counterexample reproduces the violation) vs **CANDIDATE** (the witness touches an uninterpreted function / out-of-model term, so it cannot be confirmed — **fail-closed**). On `unsat`, an honest **unsat-core** is surfaced (`a4/a5/a6_eq_*` next-state equalities + `a8_not_*` negated goal). |
| **M5** | **Spend VC** — the dedicated "no value created" check on the spend path (the `spent_out` reasoning), keyed by the documented value-bearing predicate. |
| **M6** | **Structural translation validation** — `validate_translation` checks that an emitted `.sil` *structurally corresponds* to the Portrait model (no dropped guard, no extra op, no dropped write), reporting `CORRESPONDS` / `DIVERGES`. |

Cross-cutting, all classes share the **SAT(T) vacuity guard**: before trusting an
`unsat` (→ `PROVED`), Lens checks the premises are jointly satisfiable. A
contradictory-guard covenant whose VCs are vacuously valid resolves to
**`UNKNOWN`, never `PROVED`**.

### The honestly-scoped cross-check

The `prove` footer runs the same query a second time under a *perturbed solver
configuration* and reports agreement — and **explicitly disclaims independence**:
"same binary, perturbed config — A3 reduced, not discharged." Assumption **A3**
("trust z3") is *reduced* (one narrow failure class — verdict-flips on seed /
assertion order — is caught), not eliminated. This is stated in the output, not
buried.

### Exact soundness claim

> Lens proves a property of the Portrait **covenant model** — under the explicit
> assumptions **A1–A4** of the M0 encoding spec (A1: the concrete semantics is
> the intended model; A2: the encoding table is faithful; A3: trust z3; A4: the
> negated-goal encoding is faithful). A `PROVED` verdict is a statement about the
> **model**, NOT about the emitted `.sil`, NOT about deployed on-chain bytecode,
> and NOT a behavioural-equivalence claim. Uninterpreted builtins (e.g.
> `blake2b`) are plain UF by default — no injectivity/collision-resistance axiom
> is assumed for value reasoning.

---

## 2. Composer — what it is and what it proves

**Composer** (`portrait-compose`) is the *protocol* frontier: it takes a global
protocol description, projects it to per-role local types, checks **safety**,
and can realize / emit / locally execute it. It is exercised in this program
through `Score::check` / `lift` / `realize` / `emit_real_covenants` / `execute`.

### Milestones (M1–M5)

| M | Delivers |
|---|---|
| **M1** | **`Score` type + projection + safety checker** — the global protocol type, projection to local types, and the path-exhaustive `check()` that rejects unsafe protocols (non-projectable branch-dependent receives, stranded `Value`, `Par` resource overlap). |
| **M2** | **Front-end lift** (`lift`) — lowers a flow into a `Score`, synthesising internal-choice structure, plus skeleton emission. |
| **M3** | **Realization** (`realize`) — KIP-20 binding + timelock-escape, with the **not-stranded-beyond-T** property (a `Value` cannot be left unreclaimable past the timeout T; escape `reclaim ⊇ escrowed`). |
| **M4** | **Flow surface syntax** — the human-authored flow notation that lifts into a `Score`. |
| **M5** | **Real `.portrait` emission** (parses + sema-checks) + a **local executor** — a single-path *simulation*, not a chain runtime. |

### Exact soundness claim

> Composer establishes **type-level SAFETY of the protocol MODEL** — projectability,
> linearity/duality by construction, resource non-stranding. It is **NOT** a
> liveness guarantee on a Kaspa DAG, **NOT** a fidelity claim about a deployed
> covenant, and the executor's `Completed` is a **single-path simulation result,
> not a liveness verdict** (`happy_path_completion_guaranteed() == false`). Duality
> is not re-derived from externally-authored locals — it follows by construction
> from projecting one well-formed global type.

---

## 3. The credibility story — build → attack → fix

Both engines were built increment-by-increment, with an **adversarial attack
pass at each increment** and a **full-crate seam sweep at feature-complete**. The
discipline is the point: the soundness claims above are only worth what the
attacks that failed to break them are worth. Concrete soundness defects caught
and fixed across the program:

| Defect | Engine | Failure it would have allowed | Fixed by |
|---|---|---|---|
| **Vacuous false-PROVED** | Lens | A contradictory-guard covenant "proving" anything (premises unsatisfiable) | SAT(T) vacuity guard → `UNKNOWN`, never `PROVED` |
| **Double-receive** | Composer | A branch-dependent receive on a non-decider slipping through as projectable | `check()` rejects `NotProjectable` |
| **Recursion-strand** | Composer | A `Value` escrowed-and-abandoned at a loop-back going unnoticed | strandable-`Value`-in-loop check → `ResourceStranded` |
| **Lift tail-drop** | Composer | Trailing steps after an unbounded `repeat` silently dropped | `TrailingStepsAfterUnboundedRepeat` |
| **Vacuous liveness / not-stranded** | Composer | A timelock-escape that doesn't actually reclaim the escrowed value passing not-stranded | non-vacuous escape `reclaim ⊇ escrowed` |
| **One-directional translation / money-printing** | Lens (M6) | A `.sil` that drops the overdraw guard and mints value reported `CORRESPONDS` — via (a) a whitespace-evaded shadow `function  rebalance(` block, (b) a commented-out `require(...)` harvested as a live guard | whitespace-tolerant `find_function_block` + identifier-boundary guard; `strip_sil_comments` before any structural scan |
| **Non-ident-role false-real** | Composer | An emit whose labels collide with role names / `next` producing a false-real covenant | rejected at emit |
| **Cross-check independence overclaim** | Lens | The footer implying the second solver run is independent corroboration | footer honestly states "same binary, perturbed config — A3 reduced, not discharged" |

The two M6 translation defects (whitespace-shadow, commented-require) are the
sharpest finding: the validator is a **text scan with no lexer**, so a `.sil`
that mints money could be waved through as `CORRESPONDS`. Both were fixed at root
cause via TDD (failing test first), with the workspace left GREEN
(`cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -D warnings` all clean; lens + compose suites green). The full
feature-complete sweeps then confirmed: the `prove` side held on **every** probe
(no false-PROVED in any VC class, vacuity guard intact, no false-CONFIRM,
honest CLI output), and the Composer sweep found **no in-scope unsoundness** (17
adversarial protocols; the three residual LOW notes are documented modeling
boundaries, not false-accepts).

---

## 4. Honest remaining gaps — genuinely out of reach here, and why

These are not "todo soon"; they are the boundary of what these engines claim. An
external reviewer should read this section as the limit of the guarantee.

### Lens

- **A `.sil` operational semantics + SMT refinement for true behavioural
  equivalence.** M6 validates *structural* correspondence (tokens, guards,
  blocks). It does **not** prove the emitted `.sil` *behaves* identically to the
  model. Closing this needs a formal operational semantics for `.sil` and an SMT
  refinement check between the two — a substantial, separate body of work, not a
  text scan. Out of scope: the program's claim is about the model, and structural
  correspondence is the honestly-stated ceiling of M6.
- **A verified proof kernel / proof certificates.** Lens trusts z3 (assumption
  A3, only *reduced* by the cross-check). There is no independently-checkable
  proof certificate and no verified kernel. Eliminating A3 requires either a
  certified solver or proof-term replay — out of scope for a pre-production model
  checker.
- **Binding `spent_out` to the on-chain amount.** The spend VC reasons about the
  model's `spent_out`; it is **not** bound to the actual on-chain transaction
  amount. That binding requires the on-chain settlement layer, which is not part
  of these engines.

### Composer

- **A real permissionless / on-chain runtime.** The executor is a single-path
  in-memory simulation. There is no permissionless DAG runtime, no liveness
  reasoning under adversarial scheduling. Out of scope: Composer's claim is
  type-level safety of the model, explicitly not DAG liveness.
- **Full covenant-body emission.** `emit_real` produces a parsing,
  sema-checking `.portrait` skeleton/realization — not a complete, deployable
  covenant body. Full-body emission is downstream engraver work, separate from
  the protocol-safety frontier.

---

## 5. Maturity stamp

**Pre-production · unaudited · testnet-only.** Nothing in this program is
deployed to kaspanet. No KIP is drafted or implied. The engines prove/handle the
**model**, not deployed `.sil` and not on-chain behaviour. Every figure here
traces to the program record (commits `ff3d1dc` / `0249648`, engine pin
`v2.0.0` = `90dbf07`) and the cited sweep + M0 documents. This is the Foundation's
engineering record of what the Lens and Composer engines deliver; it carries the
perishable-evidence stamp and must not be read as a production-readiness or audit
claim.
