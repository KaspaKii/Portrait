# COMPOSER — M0 design artifact (multi-role protocol projection)

> **M0 design artifact — companion to the M1 type-level implementation; for
> external session-types review.**
>
> This document specifies, at the
> level a session-types / concurrency-theory professional can audit, the design
> of the **Composer** pass and its global protocol IR, the **Score**.
>
> **Status (M1, type-level — implemented).** The grammar (§2.2), projection
> (§3.1), and the projectability / duality / linearity checks (§3.2–§3.3) are
> now realised as a **dep-free** Rust crate, `portrait-compose`
> (`portrait/portrait/crates/portrait-compose/`). It provides the
> `Score`/`Global`/`Local` types, a `project()` function, and a `Score::check()`
> that returns the per-role local types on accept or a *named* error pinpointing
> the violation. The 3-party escrow (§5) is the worked example
> (`asset_escrow_example()`); a non-vacuity test-suite accepts it and rejects
> several malformed protocols (non-projectable, orphan message, double-spend,
> stranded resource, self-interaction, unguarded recursion, `∥` overlap).
>
> **Soundness fixes (2026-06-29).** A soundness fix was applied after running
> the real `check()` surfaced two **vacuous accepts**: (1) the same linear
> `Value`/`Capability` delivered to two distinct receivers (`A→B{coin} . A→C{coin}`)
> was accepted because the holder rebound to the *sender*; per-resource state is
> now `Live{sort, holder, receiver}` and a re-carry to a different receiver →
> `ResourceConsumedTwice`. (2) A resource left live at a non-terminating recursion
> cut was accepted though the docs claimed `ResourceStranded` covered it — the
> check is now implemented (`!path.terminates` + residual holder map). Portrait
> commits `a48c2fa` (build) + `6ffde23` (hardening).
>
> **Status (M2, front-end lift — implemented).** The dormant
> front-end carriers are now wired into `portrait-compose` (new module
> `src/lift.rs`). `lift(&App) -> Result<Score, LiftError>` maps a **parsed**
> Portrait surface program into the Score grammar per §2.4
> (`Step::Move → Interact`, `Step::Choose → Branching`, `Step::Par → ∥`,
> `Step::Repeat(n,body) → μX.(body.X)`); when no `flow {}` block is present — the
> case for *every* current library program, where `flow` appears only in comments —
> the lift falls back to the cross-role **lifecycle** edges, which are the real
> parsed multi-role carrier. The canonical real program **DigitalReit** (`token`,
> `splitter`; lifecycle `token.distribute → splitter.payout`) lifts, `check()`
> accepts it, and `emit_role_skeletons()` produces one per-role covenant
> **skeleton** per role. A non-vacuity test pins the other direction: an UNSAFE
> `choose` (a non-decider diverging with no notification) lifts FAITHFULLY to a
> `Branching` and `check()` then rejects it with `NotProjectable` — the lift does
> **not** paper over it. Un-liftable shapes return *named* `LiftError`s
> (`SingleRole`, `UnknownRole`, `SelfInteractionStep`, `DegenerateChoice`,
> `EmptyChoiceBranch`, `EmptyRepeatBody`, `NestedControlInRepeat`, `NoFlow`,
> `TrailingStepsAfterUnboundedRepeat`)
> rather than a silently-wrong Score. The crate stays **path-dep-only** (a single
> `portrait-syntax` path dep; no crates.io). The opt-in end-to-end harness is the
> test-only `compose_lift_report`
> (`cargo test -p portrait-compose -- --nocapture compose_lift_report`): it
> parses a program, lifts it, checks it, and prints the per-role projection, the
> emitted skeletons, and the honest safety-not-liveness footer. `portrait-cli` is
> intentionally **not** touched.
>
> **What M2 does and does NOT add.** M2 adds NO new safety reasoning — it hands
> the lifted Score to the *same* M1 `check()`. Its load-bearing claim is
> **lift FAITHFULNESS**: every flow construct maps to its designated global-type
> construct, and an un-representable construct is a named refusal, so an unsafe
> program cannot be quietly lifted into a safe-looking Score (§4.4). The emitted
> per-role covenants are **SKELETONS** (structural: entrypoints + message/resource
> handoffs), clearly labelled, **not** deployable `.sil`. Two honest lift
> simplifications, recorded here so they are not mistaken for proven properties:
> (1) the lifecycle→flow fallback reads lifecycle edges as a *linear* Move
> sequence and threads a single `step` `Continuation`, so a terminal move with no
> successor hands off to a declared counterparty (modelling the loop-back of a
> `distributing → distributing` self-edge); (2) a `repeat` body is lifted only
> when it is Move-only (`NestedControlInRepeat` otherwise).
>
> **Soundness fix.** A faithfulness break was found and fixed: `append_global`
> previously returned a
> `Rec`/`Var` leaf *unchanged*, so any flow steps **sequenced after a `repeat`**
> (which lifts to the unbounded `μX.(body.X)` with no `End` leaf) were **silently
> dropped**. Consequence — exactly the §4.4 worst case: a program whose unsafe
> construct (e.g. an orphan-wait `choose`) sat *after* a `repeat` lifted to an
> ACCEPTED Score for a *different, truncated* program. The fix makes
> `append_global` total and honest: appending a non-empty tail past a recursion
> boundary is **unrepresentable** in the M2 unbounded-loop model, so the lift now
> refuses with the named `TrailingStepsAfterUnboundedRepeat` rather than dropping
> the tail or fabricating an exit. (Re-order so the `repeat` is last, or fold the
> post-loop work into the body.) This also closes the secondary case — a shared
> post-`choose` tail appended onto a branch body that ends in a
> `Rec`. A trailing `repeat` with nothing after it still lifts and `check()`
> accepts (no over-refusal). Pinned by four tests:
> `unsafe_choose_after_repeat_is_not_dropped_lift_refuses`,
> `safe_tail_after_repeat_is_a_named_lift_error_not_dropped`,
> `shared_choose_tail_onto_branch_ending_in_repeat_is_refused`, and
> `repeat_as_last_step_still_lifts_and_accepts`. Two lower-severity
> notes (the `choose` decider is always the branch-leader; uniform `step`
> threading makes a Move-flow double-spend unexpressible) are safe-direction
> simplifications, not unsoundness — left as documented behaviour.
>
> **What M1 does and does NOT prove.** A `check()` accept proves **safety of the
> protocol MODEL** (no stuck state + exactly-once linear handoff) *under the
> assumption that each role acts per its projected local type*. It does **not**
> prove liveness on a permissionless UTXO/DAG (a counterparty can simply never
> act; the timelock-escape discipline of §4.3 is design-level and **not** modelled
> or checked in the crate), and it proves the **model, not the deployed
> covenants** (covenant bodies are Lens's job; model-vs-emitted-script fidelity is
> a separate, unclaimed gap). This is **type-level — not a runtime**: nothing in
> the crate executes a protocol, reads a chain, or emits an on-chain artifact. No
> solver and no new runtime dependency is introduced.
>
> **Status (M3, realization layer — implemented).** The §4
> realization disciplines are now realised on top of the M2 emission as a new
> dep-free module `portrait-compose/src/realize.rs`. M3 adds **no new safety
> reasoning** — it consumes the *same* checked projection from `Score::check()`
> and the M2 per-role skeletons, and attaches two structural disciplines plus an
> honest liveness report:
>
> 1. **KIP-20 cross-role binding (§4.2).** `derive_instance_id(score)` derives a
>    deterministic, structure-sensitive `InstanceId` (a `KovId`-like tag) from the
>    Score; `realize(score, locals)` stamps **every** role covenant with that one
>    shared id and emits a binding clause mirroring
>    `require(instance_id == OpInputCovenantId(0))`. `realize_binding(score, roles)`
>    confirms every role carries the *same* derived id — a role realized against a
>    *different* Score carries a different id and is rejected as a named
>    `InstanceIdMismatch` (no cross-instance splice). This is a **structural
>    emission mirroring the on-chain `OpInputCovenantId` pattern; it is NOT an
>    on-chain settlement and proves nothing on-chain.**
> 2. **Timelock-escape discipline (§4.3) — hardened, NON-VACUOUS.** The
>    honest unit of risk is an **at-risk wait**: a `Recv`/`Offer` at which the role
>    has its OWN escrowed (previously-sent) resources at stake. `realize` walks the
>    *ordered* projected `Local` accumulating own-sent resources and records, per
>    at-risk wait, the escrowed set (`RealizedRole::at_risk_waits`); for each it
>    emits a relative-timelock escape whose `reclaim` is exactly **the role's own
>    escrowed resources — never the incoming/awaited resource** (which the silent
>    counterparty holds and the waiter never received). `has_escape_for_every_waiter`
>    is non-vacuous: an at-risk wait is covered only by an escape on that
>    counterparty whose `reclaim` is **non-empty and a superset of the escrowed-at-risk
>    set** — an empty-reclaim escape, or one reclaiming only the incoming resource,
>    is **rejected** with a named `MissingEscape`. A role with no at-risk waits
>    (only receives, only sends, or always receives before it escrows anything)
>    has nothing of its own to lose and is **not** flagged (no over-rejection).
>    *(This closes an earlier soundness gap: the original check matched escapes
>    by peer only and never inspected `reclaim`, so a vacuous escape — including the
>    backwards `reclaim = incoming resource` that `realize` itself emitted on the
>    §5 escrow — satisfied the check while recovering nothing.)*
> 3. **Liveness property — NOT-STRANDED-BEYOND-`T` (§3.4).** `liveness_report(roles)`
>    asserts the property the escape buys, now backed by the non-vacuous check:
>    every at-risk role can RECOVER **its own escrowed resource** after `T` via an
>    escape that actually reclaims it. It is stated so it cannot be upgraded:
>    `LivenessReport::happy_path_completion_guaranteed()` **always returns `false`**
>    — a silent counterparty still blocks progress; the escape only bounds
>    stranding. `render_realization` prints the binding id, each role's binding
>    clause + escape branches, and the honest liveness footer (`NOT_STRANDED_BEYOND_T`).
>
> The crate stays **path-dep-only** (the `realize` module adds only `std`). The
> opt-in end-to-end harness is the test-only `compose_realize_report`
> (`cargo test -p portrait-compose -- --nocapture compose_realize_report`):
> parse → lift → check → realize → binding-check → escape-check → render. M3 is
> pinned by 14 tests, including the three required directions — a realized Score
> with all roles bound to one id and every at-risk waiter escaped (both checks
> ACCEPT, property holds); an at-risk role stripped of its escape (escape check
> REJECTS naming the strandable role); a role stamped with a foreign/zero id
> (binding check REJECTS, no splice) — plus the non-vacuity guards: an
> empty-reclaim escape is rejected, an escape reclaiming only the incoming
> resource is rejected, `realize` is pinned to reclaim the waiter's OWN escrowed
> resource (not the incoming one), and a waiter that escrowed nothing is correctly
> not flagged. **HONESTY:** M3 is **type/realization-level, not a runtime
> and not a deployed covenant**; the binding is structural emission, not on-chain;
> the liveness property is **NOT-STRANDED-BEYOND-`T` only, never happy-path
> completion**. `portrait-cli` is intentionally **not** touched.
>
> Pre-production, unaudited, testnet-only posture applies.
>
> **Status (M4, flow control-construct surface syntax — implemented).**
> Until M4 the `flow {}` parser only accepted the
> `<role>.<entry>` Move form; `Step::Choose`/`Par`/`Repeat` existed in the AST
> (and M2 lifted all four) but had **no concrete syntax**, so a real authored
> flow could not exercise them. M4 adds the missing surface syntax + parser in
> `portrait-syntax` (dep-free, AST shape unchanged — the M2/M3 lift/check
> semantics are untouched):
>
> - `choose { branch { <steps> } branch { <steps> } .. }` → `Step::Choose`
>   (deciding role = the branch-leader, per the existing lift rule);
> - `par { thread { <steps> } thread { <steps> } .. }` → `Step::Par`;
> - `repeat <N> { <steps> }` → `Step::Repeat(N, body)`.
>
> The control keywords are recognised only at the head of a step (a Move always
> begins `<ident>.`), so the forms do not collide; nested constructs reuse the
> same step grammar. Degenerate/malformed shapes fail loudly as **parse errors**
> rather than mis-parsing: an empty `choose {}`, a mislabelled `branch`/`thread`,
> an unterminated block, a `repeat` with no count, and a `repeat 0` are all
> rejected with named diagnostics. (`repeat 0` is rejected because the M2 lift
> models any `repeat` as an *unbounded* loop, so a `repeat 0` — "run zero times"
> — would silently lift to the opposite meaning; a loop that never runs is not
> authorable as a loop, so it is a parse error.) **HONESTY:** M4 is **surface
> syntax + parser only** — no new
> safety reasoning and no AST change; an authored `flow {}` with `choose`/`par`/
> `repeat` now parses to `App.flow` and flows through the *unchanged* M2 lift and
> `check()`. Pinned by parse tests in `portrait-syntax` and end-to-end
> `authored_choose_flow_parses_lifts_and_checks` /
> `authored_repeat_flow_parses_lifts_and_checks` in `portrait-compose`. All
> existing Move-only flows and parse tests still pass (no regression);
> `portrait-cli` is intentionally not touched.

> **Status (M5, real per-role emission + local executor — implemented).**
> M2 emitted per-role covenant *skeletons* (a structural,
> non-parseable summary) and M3 attached realization data. M5 closes the honesty
> gap and adds an executable model, in two new `portrait-compose` modules:
>
> - `src/emit_real.rs` — `emit_real_covenants(&locals)` emits, for each role, a
>   per-role `.portrait` covenant **TEXT** that genuinely round-trips:
>   `portrait_syntax::parse(&source)` is `Ok` AND `portrait_sema::check(&parsed)`
>   passes. This is asserted on the **real** parser + sema in the tests, not
>   self-claimed. The role's authorised entrypoints (its `Send`/`Select` labels)
>   become real `#[covenant(mode = transition)]` declarations with matching
>   lifecycle edges and a state-carrying `return`. **The faithful subset, and the
>   recorded gap:** the Portrait covenant grammar is single-app / role-local, so
>   the cross-role *handoff* (awaited messages, peers, the `Continuation`
>   transfer) has no surface form in one covenant; it is recorded as covenant
>   **comments**, never fabricated as declarations. A label that cannot be a valid
>   entrypoint identifier (reserved keyword / non-ident) is recorded as a named
>   `EmitGap` rather than emitted as broken text — we never emit something that
>   fails to parse/check and call it real. **Hardened (soundness fix):** the
>   role *name* is now validated too — a non-ident role name (e.g. `My Role`)
>   was previously spliced verbatim into `app {role}` and produced a covenant
>   claimed real that did **not** parse; it is now recorded as
>   `EmitGap::NonIdentRole` and emitted under a safe placeholder identifier with
>   no entrypoints/lifecycle (the honest minimal subset that still
>   parses + sema-checks). A pure receiver emits a valid
>   empty-role covenant. The `step` carry is a structural placeholder, **not** an
>   economic body (value conservation is Lens's job, out of scope).
> - `src/execute.rs` — `execute(&score)` / `execute_with_choices(&score, choices)`
>   drives the Score through its interactions under **cooperative scheduling**,
>   moving the linear resources per the §2.3 handoff rules, following
>   `Branching`/`Par`/`Rec` (bounded), and producing a `Trace` + terminal
>   `Status` (`Completed` / `Stuck{where}` / `LoopBounded`). The well-formed
>   escrow `Completed`s with the correct movements; a broken handoff /
>   double-delivery is `Stuck` (not a false `Completed`); a recursive protocol is
>   `LoopBounded`.
>
> **HONESTY:** emission is **real within stated limits** — the emitted text
> genuinely parses and sema-checks (proven on the real tools), and the cross-role
> handoff that the single-covenant grammar cannot express is recorded as a gap,
> not faked. The executor is an **in-memory SIMULATION of the Score model**, NOT
> a chain runtime: it executes no transaction, reads no chain, builds no UTXO,
> verifies no covenant; a `Completed` run does **NOT** imply on-chain liveness
> (the M3 NOT-STRANDED-BEYOND-`T` boundary still governs the permissionless
> reality — a counterparty can simply never spend). `portrait-sema` is a
> **dev-dependency only**: the default lib build stays dep-free of it (the
> round-trip assertion lives in the tests). Pinned by `emit_real`/`execute` tests
> and the opt-in `m5_report` harness in `portrait-compose`; no engine pin bump,
> no `portrait-cli` change, no regression.

> **Status (CLI front-end — implemented).** The M1–M5
> pipeline, previously reachable only through the test-only harnesses
> (`compose_lift_report`, `m5_report`), now has a real CLI entry:
> `portrait compose <file>`. It is a **thin front-end** over the existing checked
> `portrait-compose` public APIs — it adds **no** new semantics. The dispatch arm
> reads + parses the source (same handling as `prove` / `validate-translation`),
> then runs `lift::lift` → `Score::check` and, on ACCEPT, prints the per-role
> projection (`render_local`), `realize` + `render_realization` (KIP-20 binding,
> timelock escapes, NOT-STRANDED-BEYOND-`T`), `emit_real_covenants` +
> `render_real_covenants` (each recorded `EmitGap` printed, none dropped or
> invented), and the `execute` trace. **Honesty carries through verbatim:** all
> three footers — the safety-not-liveness boundary (`HONEST_BOUNDARY_FOOTER`), the
> simulation-not-a-runtime executor footer, and the model-not-deployed realization
> banner — always print, including on a clean ACCEPT (where the output still reads
> `happy-path completion guaranteed: false`). **Exit codes:** 0 only on a clean
> ACCEPT; non-zero on a `ComposeError` (unsafe protocol — REJECT, named), a
> `LiftError` (un-liftable surface shape — e.g. single-role), a parse error, or
> IO; 2 on a missing argument. Added a `portrait-cli → portrait-compose` workspace
> path dependency (no new external crate). Pinned by 4 new `portrait-cli` unit
> tests over the pure `render_compose`; `fmt`/`clippy -D warnings`/full
> `cargo test --workspace` green; the `prove` and `validate-translation`
> subcommands are unchanged; no engine pin bump, no regression.

---

## 0. Scope, audience, and what "soundness" means here

**Audience.** A reviewer fluent in **multiparty session types (MPST)** — the
Honda–Yoshida–Carbone line and successors (Scalas–Yoshida generalised MPST,
Deniélou–Yoshida communicating automata, Coppo–Dezani–Padovani–Yoshida progress
proofs). Familiarity with **linear/affine resource typing** and with the gap
between **synchronous/reliable-ordered** channel models and **asynchronous /
permissionless distributed-ledger** settlement is assumed.

**The single claim this design must earn.** *If the Composer accepts a Score,
then the projected per-role covenants, when each behaves according to its
projected local type, cannot reach a configuration in which some role is blocked
waiting on a step that no other role will ever take (no stuck state), and every
declared resource is produced once and consumed once (linear handoff).*

**The boundary, stated up front (expanded in §6).** That claim is a property of
the **protocol abstraction** (the Score and its projection), under explicit
assumptions about the UTXO/DAG realisation. It is **not**:

- a formal verification of each covenant *body* (value-conservation of a
  transition body is Lens's job — a separate value-flow proof layer — and remains
  `conservation_split`'s structural check until then);
- a guarantee of **liveness on a permissionless DAG**, where a counterparty can
  simply *never act* — session-type progress assumes participants eventually
  follow the protocol, an assumption a permissionless ledger does not give for
  free (§3.4, §6);
- a proof that the *emitted silverscript* faithfully realises the local type
  (the model-vs-emitted-script gap, named in §4.4 and §6).

The discipline mirrors `conservation_split`'s honest in-code scope note and
Lens's "`UNKNOWN` is a first-class result" stance: a Composer *accept* carries
its assumptions on its face; an *unprojectable* or *incompatible* protocol is a
**loud rejection**, never a silent pass.

---

## 1. Honest current state, re-verified read-only (2026-06-29)

Grounding facts, read from source today (not asserted from memory). These pin
exactly how thin the existing base is, so the Score is not implied to exist.

| Component | What it actually is today | Evidence (read-only) |
|---|---|---|
| **IR has roles + a dormant `channels` field** | `Cartoon { roles: Vec<RoleGraph> }`; each `RoleGraph` carries `channels: Vec<Channel>`, and `Channel { to_role, authorizing_entry }` is documented as "a lineage edge to another role, realised on-chain as covenant-ID inheritance." | `portrait-ir/src/lib.rs:13-20, 64-69` |
| **…but `channels` is never populated** | In `lower()`, every `RoleGraph` is built with `channels: Vec::new()`. The type exists; no front-end fills it and no pass reads it. | `portrait-ir/src/lib.rs:217` |
| **A `Flow`/`Step` AST already exists — also unconsumed** | `App { flow: Option<Flow> }`; `Flow { steps: Vec<Step> }`; `Step` is `Move { role, entry } \| Choose(Vec<Flow>) \| Par(Vec<Flow>) \| Repeat(u32, Box<Flow>)`. This is parsed but **`lower()` never reads `program.app.flow`** — it lowers only `lifecycle` edges and entrypoints. | `portrait-syntax/src/lib.rs:16, 256-267`; `portrait-ir/src/lib.rs:93-237` (no `flow` reference) |
| **Allocation is attribute-driven + read-only** | `classify()` is a 12-line `match` on `Mode`/`to`; `allocate()` annotates each transition with a `Layer` (`Covenant`/`VProg`) and returns a `PounceResult`. It rewrites nothing and does no cross-role reasoning. | `portrait-pounce/src/lib.rs:34-62` |
| **Intra-role binding that DOES exist** | `CovenantModel.has_vprog` makes the engraver add a `proof_cov_id` arg + an `OpInputCovenantId` check binding a role's own covenant to its own VProg proof. Intra-role, not a multi-party protocol. | `portrait-ir/src/lib.rs:78-83` |
| **Cross-covenant binding that DOES exist** | `portrait-plan`: a `LineagePlan` over `Covenant` nodes with `LineageEdge { parent, child, binding_field }`; `deploy_order()` topologically sorts (parents before children), **rejects unknown covenants and cycles**, and serialises a `DeployManifest`. The on-chain meaning is the child's `require(parent_kov_id == OpInputCovenantId(0))` guard. | `portrait-plan/src/lib.rs:61-90, 145-201`; `examples/digital_reit_manifest.rs` |
| **DigitalReit "2-role" demo** | Hand-authored `LineagePlan` over two covenants (`DigitalReitToken` → `DigitalReitSplitter`) with one `LineageEdge` binding the child's `parent_kov_id`. Deploy order computed, cycle rejected, manifest emitted. | `examples/digital_reit_manifest.rs`; `portrait-plan/src/lib.rs:316-403` (tests) |

**The precise gap.** DigitalReit proves: (a) emit >1 covenant from one source,
(b) bind a child to a parent **covenant ID**, and (c) order their deployment with
**cycle rejection**. What is missing — and what nothing in the stack does:

1. **No global protocol object.** `Flow`/`Step` exists in the *syntax* but is
   dropped at lowering; `channels` exists in the *IR* but is never filled. There
   is no consumed object that says "Buyer pays, then Seller releases, else
   Arbiter resolves" as one type.
2. **No compatibility / duality check.** `portrait-plan` rejects **cycles in the
   lineage DAG** (`cycle_is_rejected`) — that is graph *acyclicity*, not
   *deadlock-freedom*. Nothing checks that a send is matched by a receive, nor
   that resources are consumed exactly once.
3. **No projection guarantee.** `portrait-project` projects each role
   independently (field-union per role); there is no derivation of a per-role
   local view *from* a global protocol with a re-composition guarantee.

The Composer is the proposed layer that (i) **consumes** the existing
`Flow`/`channels` carriers into a typed global protocol, (ii) **projects** it to
per-role local types, and (iii) **checks** projectability + duality + linearity
before any emit. The on-chain binding and deploy-ordering mechanisms it targets
(`OpInputCovenantId`, `portrait-plan`) **already exist**; the new work is the
type theory and its checks.

---

## 2. The Score — the global protocol model

### 2.1 Design intent

The Score is a **global session type** (MPST "global type") plus a **linear
resource ledger**, describing a multi-party interaction *once*. It sits between
the existing Cartoon IR and the existing per-role projection. It is **additive**:
the engraver/atelier backends are unchanged; what changes is that the
`CovenantModel`s they receive arrive *with* a proven-compatible set of cross-role
bindings, and unprojectable/incompatible protocols are rejected before emit.

The Score is *lifted* from the already-parsed-but-dropped `App.flow` (the
`Move`/`Choose`/`Par`/`Repeat` steps) plus role/entrypoint information, and it
*populates* the dormant `RoleGraph.channels` field as its IR-level record of
each interaction edge.

### 2.2 Grammar of the global type

Let **R** be a finite set of role identifiers (the `RoleGraph.role` names),
**X** a set of recursion variables, **L** a set of branch labels, and **Res** a
finite set of *resource* identifiers (defined in §2.3). The proposed global type
**G** is:

```
G  ::=  p →[m] q { r }  . G          (interaction: p sends message m carrying resources r to q, then G)
     |  p → q { ℓ_i : G_i }_{i∈I}     (branching: p selects label ℓ_i ∈ L, informs q, continues G_i)
     |  G_1 ∥ G_2                      (parallel: independent sub-protocols over DISJOINT roles & resources)
     |  μX. G                          (recursion: guarded; bounded unfolding for finite covenants)
     |  X                              (recursion variable)
     |  end                            (termination: all resources consumed)

m  ::=  a message label (an interaction name; maps to an `authorizing_entry`)
r  ⊆  Res, the (possibly empty) multiset of resources transferred in this step
p, q ∈ R,  p ≠ q  (no self-interaction)
```

**Well-formed-syntax side conditions (checked before any semantic check):**

- **Guarded recursion.** Every `X` occurs under at least one interaction or
  branching prefix inside its binding `μX` (no `μX. X`). Bounds the protocol
  unfolding so each role projects to a *finite* covenant lifecycle — required
  because a covenant lifecycle is a finite state machine
  (`StateNode`/`Transition` graph), not an unbounded process.
- **Disjointness of `∥`.** The two branches of `G_1 ∥ G_2` must mention disjoint
  role sets *and* disjoint resource sets. This is the strongest simplifying
  restriction we propose for M0/M1: it sidesteps the hard MPST interleaving
  cases and matches the only parallelism the on-chain model gives cheaply
  (independent UTXO sub-trees that never touch the same coin). **Flagged for the
  reviewer (§7, Q4): is this too strong to be useful, and what is the minimal
  safe relaxation?**
- **Single sender/decider per construct.** Each interaction has exactly one
  `from`; each branching has exactly one selecting role `p`. This is the
  on-chain "single-active-role per step" condition: a settling transaction is
  authorised by one spending party (§3.1, §4.1).

### 2.3 The linear resource ledger

`Score.resources : Vec<Resource>` declares each linear asset the protocol moves.
A resource has a **sort**, which fixes how it realises on-chain:

| Resource sort | Meaning | UTXO/covenant realisation |
|---|---|---|
| `Value(coin)` | fungible value units | UTXO **value** (sompi) moved by the settling tx |
| `Capability` | an authorisation / guard token | a covenant **guard** (e.g. a key-sig or `OpInputCovenantId` predicate) that must hold to fire a transition |
| `Continuation` | the right/obligation to take the next protocol step | a **covenant-ID handoff**: the next role's spend is gated on the previous role's covenant ID (KIP-20), exactly the `parent_kov_id` mechanism generalised |

Each resource is **linear**: introduced exactly once and consumed exactly once
along *every* path through `G` (formalised in §3.3). `Continuation` is the
load-bearing sort — it is what turns "A's message to B" into "B can only act in a
tx that spends A", and it is the design's bet that *single-spend of a UTXO* is
the faithful on-chain image of *linear consumption of a session resource* (the
core technical risk; §6, R2; §7, Q1).

### 2.4 Relationship to the existing carriers

- **`App.flow` → Score.** `Step::Move { role, entry }` lifts to an `Interact`
  whose `from` is `role`, whose message `m` is `entry`, and whose `to`/resources
  are determined by the entry's lifecycle edge and resource annotations.
  `Step::Choose(Vec<Flow>)` lifts to a `Branching`. `Step::Par(Vec<Flow>)` lifts
  to `∥`. `Step::Repeat(n, body)` lifts to a **bounded** `μX. (body . X)`
  unfolded `n` times (the `u32` bound is the finite-covenant guarantee, already
  in the syntax). *This is why the front end needs no new grammar for M1 — the
  `Move/Choose/Par/Repeat` shapes already exist; they are simply not consumed by
  `lower()` today.*
- **Score → `RoleGraph.channels`.** Each interaction `p →[m] q` populates a
  `Channel { to_role: q, authorizing_entry: m }` on `p`'s `RoleGraph`. The IR
  edge type already exists (`portrait-ir/src/lib.rs:64-69`); the Composer is what
  finally fills it.

---

## 3. Projection, well-formedness, and compatibility

### 3.1 Projection: global type → per-role local type

Projection `G ↾ p` derives role `p`'s **local type** **T** — its private view of
the protocol:

```
T  ::=  q ![m] { r } . T          (send to q)
     |  q ?[m] { r } . T          (receive from q)
     |  q ⊕ { ℓ_i : T_i }         (internal choice: p decides, sends selection to q)
     |  q & { ℓ_i : T_i }         (external choice: p awaits q's selection)
     |  T_1 ∥ T_2  |  μX. T  |  X  |  end
```

The projection rules (standard MPST, specialised to the single-sender model):

- `(p →[m] q { r } . G) ↾ s` =
  - `q ![m]{r} . (G ↾ p)`  if `s = p` (the sender's view: a send);
  - `p ?[m]{r} . (G ↾ q)`  if `s = q` (the receiver's view: a receive);
  - `G ↾ s`                 otherwise (a non-participant skips this step) —
    **subject to the merge side-condition below.**
- `(p → q { ℓ_i : G_i }) ↾ s` =
  - `q ⊕ { ℓ_i : (G_i ↾ p) }`  if `s = p` (internal choice — `p` decides);
  - `p & { ℓ_i : (G_i ↾ q) }`  if `s = q` (external choice — `q` is told);
  - `⊓_i (G_i ↾ s)`             otherwise — the **merge** of the per-branch
    projections, which must be *defined* (see projectability, §3.2).
- `(G_1 ∥ G_2) ↾ s`, `(μX.G) ↾ s`, `X ↾ s`, `end ↾ s` are homomorphic.

### 3.2 Well-formedness condition #1 — **projectability** (no orphan branches)

The merge `⊓_i (G_i ↾ s)` for a non-deciding role `s` is **defined** only if
`s`'s behaviour is *consistent across all branches*: either identical in every
branch, or the branches differ only after `s` has been **notified** which branch
was taken (a distinguishing receive at the head). If two branches give `s`
incompatible obligations with no prior notification, the merge is **undefined**
and the Score is **rejected** with a diagnostic naming the role `s` and the two
divergent branches.

This is exactly the classical MPST projection side-condition. Operationally it
rules out: *a role waiting on a message that, on the branch actually taken, will
never be sent.* On-chain, "notifying `s`" is a **derived covenant-ID binding**
(a `Continuation` resource handed to `s` on each branch), so projectability is
the type-level statement of "every role that must act later is bound, on every
branch, to a UTXO it can detect and spend."

**Single-active-role per step** falls out of the grammar (§2.2): each interaction
has one `from`, each branching one decider — so every protocol step corresponds
to exactly one authorising party, which is what a settling transaction needs
(§4.1).

**No orphan messages.** Every message label `m` that appears as a *send* in some
role's projected local type must appear as a *receive* in exactly one other
role's local type, and vice versa. A send with no matching receive (or a receive
with no matching send) is an **orphan** and is rejected. (This is the static,
per-label precondition; the *ordering* of matched send/receive is the duality
check, §3.3.)

### 3.3 Well-formedness conditions #2 and #3 — **duality** and **linearity**

**Duality / compatibility (no stuck state).** After projection, the set of local
types `{ T_p }_{p∈R}` is **compatible** iff their parallel composition is
**deadlock-free**: along every reachable interleaving, whenever any role is at a
`receive`/`external-choice` point, some other role is at the *matching*
`send`/`internal-choice` point, until all roles reach `end`. Formally this is the
standard MPST *duality up to the global type*: because each `T_p = G ↾ p` is a
projection of one well-formed `G`, compatibility is **guaranteed by
construction** *for the synchronous semantics* — the theorem the Composer relies
on is the MPST result that *projections of a (projectable) global type are
deadlock-free by construction.* The Composer's job is therefore to **verify
projectability and the syntactic side-conditions**, from which duality follows;
it does **not** re-derive duality by exploring the product automaton (that is the
fallback diagnostic path, §5, used only to *explain* a rejection).

> **Reviewer checkpoint (§7, Q2).** The clean "compatible by construction" story
> is the synchronous-MPST theorem. The UTXO/DAG realisation is **asynchronous and
> permissionless** (§3.4). The design claim is that the *safety* half (no stuck
> state *if all parties act*) transfers, while the *liveness* half (parties *do*
> act) does **not** transfer for free and must be bought with timelock-based
> escape hatches (§4.3). This split is the most important thing for the reviewer
> to validate.

**Linearity (exactly-once handoff).** Define, for the resource ledger, a usage
discipline over `G`:

- **Introduced once.** Each `r ∈ Res` is the payload of exactly one interaction
  that *creates* it (its source), reachable on every path that uses it.
- **Consumed once.** On every maximal path from the root to `end`, each `r` is
  the payload of exactly one *consuming* step, and resources offered on the
  branches of a `Branching` are **mutually exclusive** (a resource consumed on
  branch `ℓ_1` is not also consumed on `ℓ_2`) — so exclusive branches do not
  double-count.
- **None stranded.** No path reaches `end` with an unconsumed resource still
  live.

A resource consumed twice (double-spend of a `Continuation`), or produced and
never consumed (stranded value), is **rejected**. On-chain this mirrors the
single-spend nature of a UTXO/covenant continuation: linearity in the Score is
the off-chain, statically-checked image of the on-chain single-spend guarantee,
checked **before** deployment. This is an *affine/linear typing* discipline over
the global type, not a solver obligation.

### 3.4 Safety and liveness — precise definitions and their UTXO boundary

Let the **configuration** of a deployed protocol be the tuple of each role's
current local-type continuation plus the live resource multiset.

- **Safety (no stuck state).** A configuration is *stuck* if some role's
  continuation is a `receive`/`external-choice` whose matching send is not
  offered by any other role's continuation, **and** the configuration is not
  `(end, …, end)`. The protocol is **safe** if no reachable configuration is
  stuck. *Composer guarantee (in scope):* a Score that passes projectability +
  duality + linearity has no stuck configuration **under the assumption that
  every role, when it acts, acts according to its projected local type.** This is
  a property of the abstraction and transfers to the chain *for the steps that
  actually happen*.

- **Liveness (progress).** The protocol is **live** if every reachable non-`end`
  configuration *can* take a step toward `end`. *Composer guarantee (in scope):*
  the abstraction is live in the standard MPST sense — there is always an enabled
  step. *Boundary (out of scope, the hard part):* on a **permissionless DAG**,
  "can take a step" ≠ "will take a step." A counterparty may **never spend** the
  UTXO that holds the `Continuation` it was handed. Session-type progress assumes
  participants eventually follow the protocol; a permissionless ledger does not
  enforce that. Therefore **liveness against an adversarially-silent
  counterparty is NOT a Composer guarantee.** The only honest on-chain remedy is
  a **timelock escape** (§4.3): a relative-timelock (`this.age`) branch that lets
  a waiting party reclaim/advance after a deadline. The Composer can *require*
  that every external-choice point has such an escape branch and *check its
  presence* — but the resulting guarantee is "no party is stranded **forever**,"
  bounded by the timelock, **not** "the happy-path protocol completes."

This safety/liveness split, and its precise boundary on UTXO, is the spine of the
honesty story (§6) and the central reviewer ask (§7).

---

## 4. UTXO / covenant realisation and cross-role binding

### 4.1 A protocol step is a settling transaction

Each `Interact { from: p, to: q, message: m, resources: r }` realises as **one
settling Kaspa transaction**:

- **Inputs:** the UTXO(s) carrying `p`'s covenant continuation (and any
  `Value`/`Capability` resources in `r`).
- **Authorisation:** `p` is the single spending/authorising party (the
  single-active-role condition, §2.2).
- **Outputs:** a UTXO bound to `q`'s covenant, carrying the moved `Value` and a
  `Continuation` for `q`. The output script encodes the next-step guard.

A `Branching { p decides }` realises as `p` choosing *which* settling tx to
broadcast (one per label `ℓ_i`); the chosen tx publishes a label-distinguishing
`Continuation` so that every notified role can detect the branch on-chain.

### 4.2 Cross-role binding via KIP-20 covenant IDs (the lineage mechanism generalised)

The `Continuation` handoff from `p` to `q` is realised by the **exact mechanism
that exists today**:

- on `p`'s covenant: the settling tx produces an output (or terminal) carrying
  `p`'s covenant ID into the slot `q` reads;
- on `q`'s covenant: the transition that consumes the continuation is guarded by
  `require(parent_kov_id == OpInputCovenantId(0))` — the same guard the engraver
  emits for `has_vprog` (`portrait-ir/src/lib.rs:78-83`) and that DigitalReit
  hand-declares as `parent_kov_id` (`portrait-plan/.../digital_reit_manifest.rs`).

The Composer's contribution is to **derive** the set of `(parent, child,
binding_field)` lineage edges *from the Score's interaction edges* — rather than
hand-authoring them as DigitalReit does — and then to reuse `portrait-plan`
**unchanged** for deploy-ordering and **cycle rejection**. Each Score interaction
edge becomes a `LineageEdge`; `deploy_order()` orders parents before children;
its existing cycle check rejects lineage cycles. (Note the existing cycle check
gives *acyclicity of the deploy DAG*, **not** deadlock-freedom — the latter is
the new §3.3 duality check, which acyclicity does not imply.)

### 4.3 Timelock escapes — the liveness bridge (and its limit)

Because liveness against a silent counterparty does not transfer (§3.4), the
realisation adds, for every `external-choice` / `receive` point where a role
*waits* on another, a **relative-timelock escape branch** using `this.age`
(Kaspa's relative timelock; the same primitive `kcp-vesting`/timelock patterns
use). The escape lets the waiting party advance or reclaim after a deadline. The
Composer would (proposed M3) **require and check** the presence of an escape on
every wait, and reject a protocol that leaves a party able to be stranded forever.

**This is a bridge, not a closure of the gap.** A timelock turns "stranded
forever" into "stranded until `T`, then recover," which is the best a
permissionless ledger offers. It changes the local type (an extra branch) and
must be accounted for in projection and linearity. **Reviewer ask (§7, Q3):** is
the timelock-escape discipline sufficient to recover a defensible *liveness*
statement, and what exactly is that statement?

### 4.4 The honest mapping risk

Session types assume **reliable, ordered, point-to-point channels**. A
Kaspa/UTXO DAG provides **none** of those primitives directly:

- **No channel identity.** "A's message to B" is not a channel send; it is a
  UTXO that B must *find* and *choose* to spend. The `Continuation`-as-covenant-ID
  binding is how the realisation *manufactures* a point-to-point handoff from a
  shared mempool/DAG. Whether this manufactured channel is faithful is R2.
- **Ordering via single-spend, not FIFO.** Session ordering is recovered from the
  **linear single-spend chain** of continuations (each step consumes the prior
  continuation), not from channel FIFO. This is why linearity (§3.3) is
  load-bearing: it is what gives ordering on-chain.
- **Asynchrony and reorg.** The DAG is asynchronous and can reorganise before
  finality. The design assumes a step "happens" only at the engine's finality
  point; pre-finality reorgs are out of scope for the abstraction and must be
  handled by the deployer's confirmation policy. **Stated, not solved.**
- **Model ≠ emitted script.** The Composer proves a property of the Score/local
  types. The chain enforces the *emitted silverscript*. The fidelity of
  `portrait-emit`'s lowering of a local type to a covenant is a separate gap
  (translation validation), named here and **not** claimed.

These are the points where the abstraction *bridges* (continuation single-spend
chain ⇒ ordering) and where it *cannot* (silent counterparty ⇒ no happy-path
liveness; pre-finality reorg; model-vs-script fidelity). They are the review
surface of §7.

---

## 5. Worked example — 3-party escrow (Buyer / Seller / Arbiter)

A single protocol; the Composer projects three roles and checks they compose.
**This is illustrative of a design; nothing is implemented.**

### 5.1 The global type (Score)

```
protocol AssetEscrow {
  roles      { Buyer, Seller, Arbiter }
  resources  { payment : Value,         // the escrowed funds
               asset   : Capability,    // right to the off-chain/asset deliverable
               step    : Continuation }  // the right to take the next protocol step

  G =  Buyer →[fund]   Seller { payment, step } .          // (1) Buyer funds escrow, hands step to Seller
       Seller →[deliver] Buyer { asset } .                  // (2) Seller delivers asset to Buyer
       Arbiter → Buyer {                                    // (3) Arbiter adjudicates (internal choice @ Arbiter)
         release : Buyer →[settle] Seller { payment } . end // release: funds to Seller
         refund  : end  [payment ⊸ Buyer]                   // refund: payment consumed locally by Buyer
       }
}
```

Read as: Buyer funds; Seller delivers; Arbiter decides `release` or `refund`,
informing both Buyer and Seller; on `release` the payment goes to Seller; on
`refund` it returns to Buyer.

Note on the `refund` branch (a point a session-types reviewer would press):
the grammar forbids self-interaction (`p ≠ q`, §2.2), so "funds back to Buyer"
is **not** a `Buyer → Buyer` message. It is modelled as the linear consumption
of `payment` *by its current holder* — the escrow continuation is held against
Buyer, and on the `refund` verdict the branch reaches `end` with `payment`
consumed locally (notation `[payment ⊸ Buyer]` = "the live `payment` resource is
discharged to Buyer at branch end"). On-chain this is Buyer spending its own
escrow UTXO under the Arbiter's `refund` verdict guard — a single settling tx
with one authoriser (Buyer), satisfying single-active-role — not a peer-to-peer
send. Linearity still holds: `payment` is consumed exactly once, on this
exclusive branch (§5.3).

### 5.2 The three projected local types

`G ↾ Buyer`:
```
Seller ![fund]{payment, step} .
Seller ?[deliver]{asset} .
Arbiter & {                          // external choice — Buyer is told the verdict
  release : Seller ![settle]{payment} . end
  refund  : end  [payment ⊸ Buyer]   // payment discharged locally to Buyer (no self-send; see §5.1 note)
}
```

`G ↾ Seller`:
```
Buyer ?[fund]{payment, step} .
Buyer ![deliver]{asset} .
Arbiter & {                          // external choice — Seller is told the verdict
  release : Buyer ?[settle]{payment} . end
  refund  : end                      // Seller receives nothing; MUST be notified (see check)
}
```

`G ↾ Arbiter`:
```
Buyer ⊕ {                            // internal choice — Arbiter decides
  release : end
  refund  : end
}
```

The Buyer/Seller realise as **covenants** (`Mode::Transition`, value-bearing).
The Arbiter realises as a **capability/guard** (its decision is a signed
authorisation gating which branch's settling tx is valid) — `classify()` would
place its decision step accordingly. If the verdict were *computed* off-chain
(e.g. an oracle attesting a delivery condition), that step would be a
`NonCovenant`/VProg settled via the existing tag-0x21 / Varnish path, bound back
by covenant ID — but the plain-arbiter version above needs no VProg.

### 5.3 What the Composer would check (and reject on failure)

- **Projectability.** In the `Arbiter` choice, **both** Buyer and Seller are
  non-deciders, so each must be **notified** of `release` vs `refund`. The
  `refund:end` branch for Seller is the dangerous one: if Seller were *not*
  notified, Seller's local type would have an undefined merge (`Buyer ?[settle]`
  on one branch vs `end` on the other, with no distinguishing receive) — Seller
  could **wait forever** for a `settle` that never comes. **The Composer rejects
  this unless a notification edge (a `step`/`Continuation` to Seller carrying the
  branch label) is present.** With the notification, the merge `release : Buyer
  ?[settle] vs refund : end` is well-defined as `Arbiter & {…}` and accepted.
  *This rejection is the property `portrait-plan`'s cycle check does not give.*
- **Duality.** Every send has its matching receive: `Buyer ![fund] ↔ Seller
  ?[fund]`; `Seller ![deliver] ↔ Buyer ?[deliver]`; on `release`, `Buyer
  ![settle] ↔ Seller ?[settle]`. No orphan messages; ordering consistent ⇒ no
  stuck state (under the act-per-local-type assumption).
- **Linearity.** `payment` is introduced once (`fund`) and consumed once — to
  Seller on `release` *or* back to Buyer on `refund`, the branches being
  exclusive (never both). `asset` flows exactly once (`deliver`). `step` is the
  continuation chain. No double-consume, none stranded ⇒ accepted.
- **Liveness boundary (the honest part).** Safety holds. But Seller could fund
  nothing if Buyer never broadcasts `fund`; Buyer could be stranded if Arbiter
  never decides. **The Composer would require a relative-timelock escape**
  (§4.3) on each wait — e.g. Buyer may `reclaim` after `this.age ≥ T` if Seller
  never delivers; either party may force a default after Arbiter silence — and
  would **reject** the protocol if any wait lacks an escape. The guarantee
  delivered is "no party stranded **beyond `T`**," **not** "the happy path
  completes."

### 5.4 What the Composer would emit (on accept)

- Buyer + Seller covenants (`.sil`), Arbiter as a capability/guard;
- the `release`/`refund` branch realised as alternative settling txs publishing a
  label-distinguishing `Continuation`;
- cross-role bindings as `require(parent_kov_id == OpInputCovenantId(0))` checks,
  **derived** from the interaction edges (not hand-authored);
- a `portrait-plan` `DeployManifest` ordering the escrow covenant before its
  children, reusing the existing `deploy_order()` + cycle rejection;
- timelock-escape branches on every wait.

*(The DigitalReit waterfall is the degenerate case of this machinery: a linear
`payment` resource flowing down `μ`-bounded tranche roles, each bound by covenant
ID to the splitter — the 2-covenant hand-authored case generalised to an N-role
provably-projectable protocol.)*

---

## 6. The soundness / overclaim boundary

Stated bluntly, so no reader upgrades the claim:

1. **Session-type guarantees are about the protocol abstraction.** A Composer
   *accept* means: *the Score is projectable; its local types are dual and
   linear; therefore the abstract protocol has no stuck state and exactly-once
   handoff.* It is **not** a formal verification of the deployed instrument.
2. **Covenant bodies are out of scope here.** Whether each transition body
   conserves value is **Lens's** obligation (a separate value-flow proof layer)
   and remains `conservation_split`'s structural check until Lens exists. The
   Composer composes roles; it does not prove their internal arithmetic.
3. **Liveness on a permissionless DAG is the hard part and is not delivered.**
   Safety (no stuck state *if parties act per their local type*) transfers.
   Progress against an **adversarially-silent** counterparty does **not** —
   session-type progress assumes eventual participation, which a permissionless
   ledger does not enforce. The timelock-escape discipline (§4.3) buys "not
   stranded forever," bounded by `T`; it does **not** buy happy-path completion.
4. **The synchronous→asynchronous transfer is assumed, not proven here.** "Dual
   by construction" is the synchronous-MPST theorem. The realisation is
   asynchronous (mempool/DAG) and recovers ordering from the single-spend
   continuation chain, not from FIFO channels. That this recovery is faithful is
   the central unproven bet (R2).
5. **Model ≠ emitted script.** The Composer reasons about the Score/local types;
   the chain enforces the emitted silverscript. Translation validation between
   the local type and the covenant is named future work, not a current claim.
6. **`∥` is artificially restricted.** Disjoint-roles-and-resources parallelism
   is a deliberate over-restriction for M0/M1 to avoid the hardest MPST cases; it
   may be too weak to be useful.

These map to the proposal's risks R2 (resource/UTXO mapping), R3
(deadlock/liveness over-claim), R4 (choice + asynchrony + ordering), R5 (no
synchronous vProg composition — standalone-ZK-settled handoffs only).

---

## 7. Open questions for the reviewer (must be resolved before M1)

A **session-types / concurrency-theory** reviewer (Honda–Yoshida–Carbone MPST
lineage and successors). The specific asks:

- **Q1 — Resource ↔ single-spend mapping (R2, the core bet).** Does
  *"linear `Continuation` resource ↔ single-spend covenant continuation"* hold as
  a faithful realisation, and **where does it leak**? In particular: does
  recovering session *ordering* from the single-spend chain (§4.4) correctly
  reproduce the orderings the global type intends, including across `Branching`?

- **Q2 — Safety transfer, sync → async/permissionless.** The design claims the
  *safety* half of MPST "deadlock-free by construction" transfers to the
  asynchronous UTXO realisation while the *liveness* half does not (§3.4). Is
  that split correct? Is there a known async-MPST result (e.g. via communicating
  automata / multiparty compatibility) that gives a cleaner statement of exactly
  what transfers?

- **Q3 — Timelock escapes and a defensible liveness statement.** Given that
  progress against a silent counterparty cannot be guaranteed, is the
  relative-timelock escape discipline (§4.3) the right bridge, and what is the
  **precise** liveness theorem it supports ("no party stranded beyond `T`")? Are
  there liveness pitfalls the escape branches introduce (e.g. griefing, escape
  races between roles)?

- **Q4 — Subset boundary: grammar and `∥` restriction.** Is the `G` grammar
  (§2.2) the right minimal covenant-projectable subset, or both too rich and
  missing a needed construct? Is disjoint-roles-and-resources `∥` an acceptable
  M0/M1 restriction, and what is the **minimal safe relaxation** toward useful
  interleaving?

- **Q5 — Projectability side-condition vs. on-chain notification.** Does the
  proposed equation *"projectability merge well-defined ⟺ every non-decider is
  notified of the branch via a derived covenant-ID `Continuation`"* (§3.2, §5.3)
  actually deliver no-orphan-receive on-chain, and are there choice topologies
  where notification edges cannot be realised within the single-sender,
  single-spend model?

- **Q6 — Where the abstraction must refuse.** Identify the protocols the Composer
  must **reject loudly** rather than mis-realise — beyond the three checks — given
  the asynchrony, reorg, and fee/timelock realities a deployed instrument faces.

The bar before any external-facing claim: a session-types professional agrees
that a Composer *accept* means exactly what §0 and §6 say it means — *safety of
the protocol abstraction with exactly-once handoff, under explicitly stated
assumptions, with liveness against a silent counterparty explicitly excluded* —
and that the synchronous→on-chain transfer (Q1, Q2) is sound or precisely
bounded.

---

*Recap of standing: **M1, type-level — implemented as the dep-free
`portrait-compose` crate; companion design artifact for external session-types
review.** The crate realises the Score grammar, projection, and the three checks;
it proves **safety of the protocol model** (no stuck state + exactly-once handoff)
under the act-per-local-type assumption, NOT liveness on a permissionless UTXO/DAG
and NOT the deployed covenants. The original IR carriers it generalises remain as
re-verified read-only from `portrait/.../crates` on 2026-06-29: `App.flow`
(`Move/Choose/Par/Repeat`) and `RoleGraph.channels` exist in the source but are
still **unconsumed** by `lower()` (wiring the front-end into `portrait-compose` is
future work); allocation is attribute-driven + a read-only advisor; `portrait-plan`
gives covenant-ID lineage + deploy order + **cycle** rejection (acyclicity, not
deadlock-freedom); DigitalReit hand-authors a 2-covenant lineage. No solver, no new
runtime dependency, no on-chain artifact is introduced. The Lens value-flow proof
is a separate layer.*
