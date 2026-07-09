# Portrait — Architecture

> **Maturity: pre-production, unaudited, testnet-only.** No external security
> audit or external review has taken place — internal adversarial hardening is
> not external review. Nothing is on mainnet; live evidence is perishable Kaspa
> testnet-10 (TN10) evidence (the testnet resets). Where verification is
> described: Lens proves properties of the covenant **model** under stated
> assumptions — not the emitted `.sil`, and nothing on-chain; composition
> checks are **type-level** safety — not liveness, and not a deployed covenant.

*The build bible. Status: design v0.1. This document is the technical counterpart to the Litepaper: where the Litepaper says **why**, this says **what** and **how**.*

---

## 0. Design axioms

These are non-negotiable. Every component below is downstream of them.

1. **Constraints are the type system.** Kaspa's "limitations" (local state, linear UTXOs, no gas, no unbounded loops, no global mutable storage) are not obstacles to abstract away — they are the *semantics* Portrait makes load-bearing. We type against them, not around them.
2. **No new on-chain trust surface.** Portrait emits Silverscript and nothing else for L1. The covenant-macro guarantees (`#[covenant.*]`) are *preserved and composed*, never re-implemented. The goal: if Silverscript is sound, Portrait adds no new on-chain trust surface — note that Portrait's own lowering is validated structurally (`validate-translation`), not proven correct, so this is a design axiom, not a discharged theorem.
3. **Correctness is an artifact, not a hope.** Every build produces covenants **plus a machine-checked verification report**. "It compiles" must mean "the declared invariants hold." (A verification report records what was machine-checked; it is not an audit and nothing is certified.)
4. **Ship the core; don't ship vapour.** The buildable core runs on silverscript v0.1 today; anything that needs substrate which does not yet exist stays out. We never ship a demo that secretly needs an opcode that does not exist.
5. **DX, docs, and threat models are first-class.** DX, docs, and the threat-model standard are first-class engineering, not afterthoughts.

A cohesive identity runs through the component codenames: a *portrait* is produced by transferring a full-scale preparatory drawing — the **cartoon** — onto the surface by **pouncing** it through. That is literally projection. The names below lean into that lineage; they are flavour, the functional names are authoritative.

---

## 1. System overview

Portrait is a three-movement pipeline — **Choreography → Projection → Proof** — wrapped in a toolchain.

```
            ┌──────────────────────────────────────────────────────────────┐
            │                      PORTRAIT TOOLCHAIN                        │
            │                                                                │
  .portrait │   FRONTEND          MIDDLE (IR)                   BACKEND      │
  sources ──┼─► Sketch ─► AST ─► [ Cartoon IR ] ─► Pounce ──► Engraver ──► N×.sil
            │     │                   ▲   │           │           │         │
            │   type stack:           │   │           │        Provenance ─► covenant-ID
            │   Ledger (linear)       │   │           │           │         plan + manifest
            │   Score  (session) ─────┘   │           │           │         │
            │   Seal   (capability)       │           │        Easel ─────► tx templates / SDK
            │   Lens   (refinement/SMT) ◄─┘           │           │         │
            │                                  Lens (SMT) ──► Hallmark ────► verification report
            └──────────────────────────────────────────────────────────────┘
```

Inputs: `.portrait` source + imported Library components.
Outputs (the **proof-carrying app bundle**):
- `N` compiled Silverscript contracts (`.sil` → native Kaspa Script),
- a **covenant manifest** (IDs, genesis tx shape, lineage edges),
- a **transaction template / SDK** per entrypoint for off-chain signers,
- a **verification report** recording the discharged declared and structural invariants.

---

## 2. The compilation pipeline, stage by stage

| # | Stage | Component | In → Out |
|---|---|---|---|
| 1 | Parse | **Sketch** | `.portrait` text → AST (reuses/extends silverscript's tree-sitter grammar) |
| 2 | Resolve & elaborate | **Frontend** | AST → elaborated AST (imports resolved, sugar lowered) |
| 3 | Type-check | **Type stack** | elaborated AST → typed AST (linear + session + capability + refinement) |
| 4 | Lower to IR | **Cartoon** | typed AST → Cartoon IR (typed resource/transition graph) |
| 5 | Project | **Pounce** | global IR → one local covenant model per role |
| 6 | Emit | **Engraver** | local model → `.sil` per contract |
| 7 | Plan lineage | **Provenance** | local models → covenant-ID genesis + binding plan + manifest |
| 8 | Template | **Easel** | manifest + IR → off-chain tx builders |
| 9 | Verify | **Lens** | IR + invariants → SMT obligations → discharged proofs |
| 10 | Package | **Hallmark** | all of the above → signed app bundle + verification report |

---

## 3. Layer A — the Portrait surface language

A small, declarative language. Its job is to let a developer state a *protocol over linear resources* and the *invariants that must hold*, and nothing low-level.

### 3.1 Constructs

- `app` / `protocol` — the top-level unit; a global choreography.
- `role` — a participant, which becomes one covenant (one contract / lineage).
- `resource` — a **linear** type (created once, consumed once): `coin`, `token<Asset>`, or a user struct marked `linear`.
- `state { … }` — per-role persisted fields (lowered into the covenant's `state`).
- `collection` types — `set<T>`, `map<K,V>`, `log<T>`.
- `capability` — an unforgeable, attenuable authority value (see Seal).
- `flow { … }` — the choreography body, built from:
  - sequencing `;`
  - labelled interaction `move`/`step`
  - choice `choose { … } or { … }`
  - bounded parallel `par { … } and { … }`
  - bounded repetition `repeat(n) { … }` (n is a compile-time bound → unroll)
  - guarded transition `when <guard> -> <state>`
- `invariant` — a global proof obligation (`invariant value_conserved`, or a custom predicate).
- `requires` / `ensures` — per-move pre/postconditions (refinement predicates → Lens).
- `offload { … } proves <predicate>` — mark heavy computation for off-chain discharge with a checked proof.
- `use <group>::<Component>` — import a Library component.

### 3.2 Grammar sketch (provisional EBNF)

```
program     = pragma , { use_decl } , app ;
app         = "app" , ident , "{" , { decl } , flow? , { invariant } , "}" ;
decl        = role_decl | resource_decl | capability_decl | binding ;
role_decl   = "role" , ident , [ "=" , component_ref ] , "{" , [ state_block ] , { entry } , "}" ;
resource_decl = "resource" , ident , [ "<" , ident , ">" ] , [ "linear" ] , ";" ;
flow        = "flow" , "{" , step , { ";" , step } , "}" ;
step        = move | choice | parallel | repeat | guard ;
move        = ident , "." , ident , "(" , [ arg_list ] , ")" , [ "->" , state_label ] ;
choice      = "choose" , block , { "or" , block } ;
parallel    = "par" , block , { "and" , block } ;
repeat      = "repeat" , "(" , int , ")" , block ;
invariant   = "invariant" , ( predicate | builtin_inv ) , ";" ;
```

### 3.3 Minimal example (single role)

```portrait
pragma portrait ^0.1.0;
use custody::TimeVault;

app PersonalVault {
  role vault = TimeVault { owner = param pubkey, recovery = param pubkey, delay = 1440 }

  flow {
    vault.schedule(beneficiary, amount) -> pending ;
    choose { vault.settle() -> settled }            // terminal, after delay
    or     { vault.cancel() -> idle }               // cold-key clawback
  }

  invariant value_conserved ;                        // discharged by Ledger + Lens
  invariant no_undeclared_state ;                    // discharged by Score (projection)
}
```

---

## 4. Layer B — the type-system stack

Four checkers compose into one judgement. A program type-checks only if all four agree.

### 4.1 Ledger — linear & resource types
Models coins/assets/`linear` structs as **linear resources**: each must be consumed exactly once along every path. Enforces **value conservation** as a typing rule: the sum of resources in = sum out for every transition. *Kills double-spend, value-leak, and forgotten-change at the type level.* Lowers to explicit `tx.inputs[…].value`/`tx.outputs[…].value` checks in the emitted `.sil`. Implemented as a conservation checker; full linear inference is not yet built.

### 4.2 Score — session / protocol types
The global `flow` is a **global session type**. Score computes the **projection** `G ↾ r` onto each role and checks well-formedness: every branch is mergeable, every role knows which branch it is in, and the protocol *model* is free of deadlock/race at the type level (a model-level typing property — not a DAG liveness claim). *This is the type-level check against the multi-contract coordination bug.* Its output drives Pounce. Sequential and choice protocols are supported; bounded `par` is not yet built.

### 4.3 Seal — capability types
No ambient authority exists, so authority must be a value. Seal types capabilities as **unforgeable, attenuable, delegable** tokens that an entrypoint must *hold* to act. Attenuation (`cap.restrict(…)`) yields a strictly weaker capability. *Eliminates the `msg.sender`/blanket-approval/confused-deputy class entirely.* Lowers to signature/preimage/covenant-ID checks.

### 4.4 Lens — refinement & SMT
`requires`/`ensures`/`invariant` predicates are refinement types. Lens emits them as **SMT (Z3) obligations** and discharges them at compile time, exploiting the bounded, gasless, reentrancy-free state space that makes EVM-style verification intractable but Kaspa's tractable. Produces the proof terms Hallmark packages. *Scope caveat: Lens discharges obligations over the covenant **model** under stated assumptions — it does not verify the emitted `.sil` and proves nothing about anything on-chain.*

---

## 5. Layer C — Cartoon IR

A single typed intermediate representation that every analysis and backend reads. It is a **typed resource-transition graph**:

- **Nodes** = covenant states (per role), each carrying a typed `state` shape and the invariants that must hold there.
- **Edges** = transitions, each carrying: a resource delta (Ledger), a guard predicate (Lens), the session label and the role's knowledge at that point (Score), and the capability required (Seal).
- **Channels** = lineage edges between roles' graphs, realised on-chain as covenant-ID inheritance (Provenance).
- **Obligations** = the proof goals attached to nodes/edges, drained by Lens.

Cartoon is deliberately *more* abstract than Silverscript (it knows about roles, sessions, capabilities, resources) and lowers down to it. Codename note: the IR is the "cartoon" precisely because **Pounce** transfers it onto the surface (projects it into per-role covenants).

---

## 6. Layer D — projection

### 6.1 Pounce — the projection engine *(the heart)*
Takes the global Cartoon graph + Score's projection and, for each role, produces a **local covenant model**: the state struct, one entrypoint per outgoing transition (with guards assembled from session preconditions + Ledger conservation + Seal capability + Lens `requires`), the covenant mode (`transition` vs `verification`) chosen automatically, and the set of lineage channels it participates in. This is what makes "describe the app once" real.

---

## 7. Layer E — backends

### 7.1 Engraver — Silverscript emitter
Local covenant model → idiomatic `.sil`, preserving `#[covenant.singleton(...)]` / `#[covenant(from=N,to=M)]` macros. Emits the `state` block, entrypoints, `require` guards (each commented with the invariant it protects), and bounded `for(…,MAX_ITERATIONS)` only where Ledger/Score prove the bound. Runs `silverscript check` on its own output.

### 7.2 Provenance — covenant-ID lineage planner
Computes covenant IDs (KIP-20 BLAKE2b construction), the genesis transaction, and the per-output `covenant_binding` (`authorizing_input` + `covenant_id`) for every transition, so lineage carries across transactions without recursive ancestor proofs. Writes the **covenant manifest** an indexer/wallet needs to follow the app.

### 7.3 Easel — transaction template generator
Per entrypoint, a typed description of inputs to select, outputs to build, witnesses to collect (signatures, preimages). Emits a small off-chain SDK so signers construct spends the covenant will accept — generated, never hand-rolled.

### 7.4 Lens (back-end role) + Hallmark — verification report
Lens discharges the SMT obligations; **Hallmark** packages the app bundle: contracts + manifest + templates + the **verification report** (which invariants were proved, by which method — type rule, SMT, or ZK — over which version of source and silverscript). The report records exactly what was machine-checked over the model; it is **not** an audit, not a certification, and does not cover the emitted `.sil` or on-chain behaviour.

---

## 8. Repository mapping

```
portrait/
├── docs/LANGUAGE.md        ← surface language reference (Layer A)
├── docs/ARCHITECTURE.md    ← this document
├── docs/CATALOGUE.md       ← the Library taxonomy
├── portrait/               ← the compiler workspace (Rust): sketch/ cartoon/ pounce/ engraver/ …
├── library/                ← the component library
├── examples/               ← chess-league and other full apps
└── tests/                  ← golden: .portrait → expected .sil, debugger vectors, verification reports
```

*Names are provisional; the architecture is not. Everything here is gated by the axioms in §0.*
