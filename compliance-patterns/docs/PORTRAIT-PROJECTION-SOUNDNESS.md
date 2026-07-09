# Portrait projection soundness — a semi-formal argument

**Status:** pre-production, unaudited, testnet-only. This is a *semi-formal*
soundness argument, not a machine-checked proof. No external audit has been
performed. A full Coq/Lean mechanization is explicitly out of scope; see
[§5 What this does NOT prove](#5-what-this-does-not-prove).

This document states and argues a projection-soundness property for the Portrait
compiler: informally, *a Portrait program that passes the structural static
checks and emits will produce a covenant set whose on-chain behaviour respects
the program's declared lifecycle and its declared value-conservation intent.*
Every step of the argument is tied to a concrete mechanism that already exists in
the codebase, and every rejection claim is backed by a named test that
demonstrates the compiler refusing the violating program.

---

## 0. Objects, notation, and the trusted base

Let `P` be a Portrait program (the `.portrait` surface: an `app` with one or more
`role`s, a `lifecycle` edge set, an optional `flow`, and a set of `invariant`s).
The compiler pipeline (driven end-to-end by
`crates/portrait-cli/tests/golden.rs::run_pipeline`) is the composition:

```
parse        :  text        -> Program          (portrait-syntax)
check        :  Program     -> Result<(),Vec<Diagnostic>>   (portrait-sema)
lower        :  Program     -> CovenantModel set (portrait-ir)
project      :  models      -> per-role models   (portrait-project)
emit         :  models      -> .sil + CTOR JSON  (portrait-emit)
```

We write `S(P)` for the emitted covenant set (the `.sil` sources plus their CTOR
constructor arrays). On chain, each `.sil` is compiled by **`silverc`** into a
covenant script that the **Toccata engine** (`rusty-kaspa` v2.0.0 = `90dbf07`)
evaluates against a spending transaction.

**Trusted base (assumptions T1–T4).** The argument is *relative to*:

- **T1 — silverscript semantics.** `silverc` correctly compiles a well-typed
  `.sil` contract, and a `return({...})` in a `transition` function constrains
  the output covenant's state fields to the returned expression, evaluated over
  `prev_states[..]`. We do not re-derive silverc's own soundness here.
- **T2 — engine semantics.** The Toccata engine enforces covenant scripts as
  written, and the surface opcodes (`OpInputCovenantId`, `OpCovInputIdx/Count`,
  etc.) have their documented meaning.
- **T3 — covenant-id model (KIP-20).** A covenant's identity is a deterministic
  function of its script; `OpInputCovenantId(i)` returns the covenant id bound to
  input `i`.
- **T4 — engine-level PQ/ZK precompile (KIP-16 tag `0x21`).** Where a STARK
  validity check is claimed, it is enforced by the *engine* precompile assembled
  by `crates/kcp-pq-anchor`, **not** by anything `silverc` or Portrait emits. See
  [§4](#4-case-c--covenant-id-binding-and-the-cross-layer-zk-boundary).

Everything below is conditional on T1–T4. The contribution of Portrait's static
checks is to guarantee that *the emitted artifact is the kind of artifact whose
on-chain meaning, under T1–T4, matches the declared spec.*

---

## 1. The theorem (informal but precise)

> **Theorem (Portrait projection soundness, semi-formal).**
> Let `P` be a Portrait program. Suppose:
>
> 1. `parse(P)` succeeds, and
> 2. `check(P) = Ok(())` (P passes all of `portrait-sema`'s structural checks),
>    and
> 3. `emit` succeeds, yielding the covenant set `S(P)`.
>
> Then, assuming the trusted base T1–T4:
>
> **(A) Lifecycle enforcement.** Every lifecycle edge `from -> to via R.e`
> declared in `P` corresponds to a real entrypoint `e` on a real role `R` whose
> emitted covenant constrains the next state via an emitted `return({...})`; and
> no lifecycle edge targets an undeclared state. Consequently no transition in
> `S(P)` can move the UTXO to a state the author did not declare, and no non-terminal transition can be emitted WITHOUT a state-rebuilding return.
>
> **(B) Value conservation (proxy).** If `P` declares `invariant value_conserved`,
> then every reachable `transition` entrypoint emits a `return({...})` that
> rebuilds the state from `prev_states[..]` — the structural precondition for
> conservation. This is a *proxy*, not a full linear-resource proof; see
> [§3](#3-case-b--value-conservation).
>
> **(C) Cross-layer binding.** For any role carrying a VProg counterpart, the
> emitted covenant binds the expected proof covenant id via a real
> `OpInputCovenantId(0)` guard. The STARK *validity* check itself is an
> engine-level tag-`0x21` obligation (T4), discharged by `kcp-pq-anchor`, and is
> **not** claimed to be enforced by the emitted `.sil`.

The honest gap — why this is "structural" and not "fully typed" — is stated in
[§3](#3-case-b--value-conservation) and [§5](#5-what-this-does-not-prove). In one
line: `portrait-sema` checks the *shape* of the program (reachability, flow
integrity, return-presence, no dangling states), not a refinement/linear type
system, so it can certify "a conserving `return` is present and well-targeted"
but not "the arithmetic inside that `return` actually conserves the resource."

---

## 2. Case A — lifecycle enforcement

**Claim.** Under (1)–(3) and T1–T3, every lifecycle edge of `P` maps to an
emitted covenant constraint, and no edge targets an undeclared state.

**Mechanism (already present).**

- *Edge resolution.* `check` rule 1
  (`portrait-sema/src/lib.rs`, lines 41–57) iterates every lifecycle edge and
  requires `find_role(edge.via_role)` and `find_entry(role, edge.via_entry)` to
  resolve. An edge naming a non-existent role or entrypoint is a hard rejection.
- *Transition emits the binding.* `check` rule 3 (lines 64–90) requires that any
  `Transition` entrypoint reached by a *non-terminal* edge contains a `Return`
  (`has_return`, line 153). The pipeline then lowers that `return` to a
  silverscript `return({...})` over `prev_states[..]`. The golden test asserts
  the lowered binding is literally present:
  `golden_counter_emits_expected_anchors` asserts the emitted `.sil` contains
  `return({ value: prev_states[0].value + delta })`; the ComplianceToken golden
  asserts `return({ balance: prev_states[0].balance - amount })`. Under T1 this
  `return` is exactly what constrains the next state of the output covenant —
  i.e. the edge's `prev -> next` relation is on-chain enforced.
- *No undeclared target.* `check` rule 5 — `no_undeclared_state`
  (lines 110–136) — rejects any non-terminal edge whose target is neither a
  source of some edge nor a declared terminal. A lifecycle cannot route the UTXO
  into a state the author never declared.

**Reject vectors (compiler refuses the violating program).**

| Violation | Test (file) |
|---|---|
| Edge names a non-existent entrypoint | `rejects_unknown_via_entry` (portrait-sema) |
| Edge names a non-existent role | `rejects_unknown_via_role` (portrait-sema) |
| Non-terminal transition with no `return` (silently drops state) | `rejects_transition_missing_return` (portrait-sema) |
| Verification entrypoint that returns a value | `rejects_verification_with_return` (portrait-sema) |
| Edge target is a dangling/undeclared state | `rejects_dangling_no_undeclared_state` (portrait-sema) |
| Flow step references unknown entrypoint | `rejects_unknown_flow_step` (portrait-sema) |
| Lifecycle edge → unknown entrypoint (full pipeline) | `reject_unknown_lifecycle_entry_fails_at_sema` (portrait-cli/tests/golden.rs) |

**Argument.** Suppose (A) failed: `S(P)` contains a transition reaching an
undeclared state, or an edge with no on-chain successor constraint. Either the
edge would have failed resolution (rule 1) or targeted an undeclared state (rule
5) or omitted the `return` (rule 3) — each a rejection contradicting `check(P) =
Ok(())`. Hence under T1–T3 every edge of `P` is a resolved, return-carrying,
declared-target transition, and `emit` (golden-verified) lowers exactly that
binding. ∎ (relative to the trusted base)

**Differential reinforcement.** `differential_counter_compiles_under_silverc`
and `differential_compliance_token_compiles_under_silverc` write the emitted
`.sil` + CTOR JSON and invoke the *real* `silverc` (exit 0 asserted), so "the
emitted artifact is accepted by the real engine compiler" is a
continuously-checked invariant — not a one-time manual claim. If `silverc` is
absent the differential test *skips with a message*, never silently passes.

---

## 3. Case B — value conservation

**Claim.** Under (1)–(3), if `P` declares `invariant value_conserved`, every
reachable `transition` entrypoint emits a state-rebuilding `return({...})`.

**Mechanism.** `check` rule 4 (`portrait-sema/src/lib.rs`, lines 92–108): when
`value_conserved` is among the invariants, every reachable `Transition`
entrypoint must satisfy `has_return`. Combined with rule 3, this guarantees the
emitted covenant carries a `return({ field: f(prev_states[..]) })` that
reconstructs the state from the previous state rather than discarding it.

**Reject vector.** `rejects_value_conserved_with_dropping_transition`
(portrait-sema) declares `invariant value_conserved` over a transition that
drops state (no `return`); the edge is marked `terminal` specifically so rule 3
does *not* fire, isolating rule 4 as the rejecting check. The compiler refuses
with `invariant 'value_conserved' violated`.

**This is a proxy — the honest limitation.** Rule 4 proves the *presence and
correct targeting* of a conserving `return`, **not** that the expression inside
it is value-preserving. `return({ balance: prev_states[0].balance - amount })`
satisfies rule 4 whether or not `amount` is constrained to be non-negative or
bounded by the balance. A genuine conservation theorem needs a linear/affine
resource type or a refinement check on the arithmetic — neither of which
`portrait-sema` performs (it states this itself: lib.rs lines 1–18, "NOT a full
type system … linearity is explicitly out of scope"). So (B) is best read as:
*the structural precondition for conservation holds, and the violating shape
(state-dropping transitions under a conservation invariant) is mechanically
rejected* — the proxy is sound for what it claims and honestly weaker than a full
conservation proof.

---

## 4. Case C — covenant-id binding and the cross-layer ZK boundary

**Claim.** For a role with a VProg counterpart, the emitted covenant binds the
expected proof covenant id with a real `OpInputCovenantId` guard; the STARK
*validity* check is an engine-level obligation, not an emitted-`.sil` one.

**Mechanism (covenant-id binding — Portrait's responsibility).** The
ComplianceToken example pairs an L1 `transfer` covenant with an off-L1
`verify_compliance` VProg. `golden_compliance_token_emits_expected_anchors`
asserts the emitted `.sil`:

- declares the injected proof argument `byte[32] proof_cov_id`,
- emits the guard `OpInputCovenantId(0)` (binding input 0's covenant id), and
- does **not** emit `function verify_compliance` — the VProg entrypoint is owned
  by Atelier (RISC Zero guest), never lowered into the covenant.

`OpInputCovenantId` is a *real* silverc surface opcode (confirmed in
`docs/PORTRAIT-PHASE-A-UNBLOCK.md`), so this binding is genuinely on-chain
enforced under T1–T2.

**The boundary (STARK validity — NOT Portrait's, NOT silverc's).** Per
`docs/PORTRAIT-PHASE-A-UNBLOCK.md`: probing `silverc` for `OpZkPrecompile`
returns *"unknown function call"* — silverscript has **no surface function for
the tag-`0x21` precompile**. The on-chain STARK validity check and the
state-commitment binding live in the **engine-level KIP-16 tag-`0x21`
precompile**, assembled by `crates/kcp-pq-anchor` (`anchor_script.rs`,
`journal_spec.rs`, `sigop.rs`) as a raw redeem script — *outside* anything
Portrait emits. Portrait's emitted covenant therefore enforces the **covenant-id
binding only**; STARK validity is a separate, engine-level obligation (T4).

**No overstatement.** Per `docs/CSCI-PROVENANCE.json` the CSCI prover status is
`PARTIAL — prover guest verified; TN10 txids PENDING`. The tag-`0x21` PQ pipeline
has been validated on TN10 by a *separate* project (`kii-ml-dsa`, per
`KNOWN-ISSUES.md`) — that is evidence for the *engine mechanism*, **not** for
end-to-end live ComplianceToken ZK verification. We claim only the binding that
the golden test actually verifies. [UNVERIFIED: end-to-end live tag-0x21
verification for this specific program on TN10.]

---

## 5. What this does NOT prove

This section is load-bearing; read it as part of the theorem.

1. **No external audit.** This is an internal, semi-formal argument. No
   third-party security audit has been performed. Pre-production, unaudited.
2. **Structural, not full-type, soundness.** `portrait-sema` performs *structural*
   checks (reachability, flow integrity, return-presence/consistency,
   no-dangling-state). It does **not** do type inference, refinement checking, or
   linearity (its own module doc, lib.rs lines 1–18). Case B is a conservation
   *proxy*: it certifies a conserving `return` is present and well-targeted, not
   that its arithmetic conserves value. A program with an under-constrained
   `amount` can satisfy every check and still be economically unsafe.
3. **Relative to a trusted base.** Soundness is *conditional* on T1–T4: silverc's
   compiler correctness, the Toccata engine's opcode semantics, the KIP-20
   covenant-id model, and the engine-level tag-`0x21` precompile. We re-derive
   none of these; a bug in silverc or the engine is outside this argument.
4. **ZK validity is engine-level and only partially evidenced here.** Portrait
   emits the covenant-id *binding*; it does not emit (and silverc cannot lower) a
   tag-`0x21` STARK check. The validity check is `kcp-pq-anchor`'s raw script.
   End-to-end live verification for ComplianceToken on TN10 is PENDING
   (`docs/CSCI-PROVENANCE.json`).
5. **Not mechanized.** No Coq/Lean proof object exists. The "proof" is a
   case argument whose every leaf is pinned to a named test; the tests are the
   machine-checked part, the prose is not.
6. **Testnet-only.** All on-chain evidence is testnet (TN10). Nothing here is a
   mainnet-readiness claim.
7. **Scope of the example surface.** The proof leans on two stable examples
   (`counter.portrait`, `tier3-demo/ComplianceToken.portrait`) as witnesses.
   Reject vectors generalize the rules, but the accept-side golden/differential
   coverage is two programs, not an exhaustive corpus.

---

## 6. Provenance of every claim

| Claim | Backed by |
|---|---|
| Five structural checks exist as described | `crates/portrait-sema/src/lib.rs` (lines 37–192) |
| Lifecycle edges resolved; dangling targets rejected | rules 1 & 5; tests `rejects_unknown_via_{entry,role}`, `rejects_dangling_no_undeclared_state` |
| Non-terminal transitions must `return`; verifications must not | rule 3; tests `rejects_transition_missing_return`, `rejects_verification_with_return` |
| `value_conserved` requires a return (proxy) | rule 4; test `rejects_value_conserved_with_dropping_transition` |
| Emitted `.sil` carries the lowered `return({...})` binding | `golden_{counter,compliance_token}_emits_expected_anchors` |
| Emitted `.sil` compiles under real `silverc` | `differential_{counter,compliance_token}_compiles_under_silverc` |
| Full-pipeline sema rejection | `reject_unknown_lifecycle_entry_fails_at_sema` |
| `OpInputCovenantId(0)` + `proof_cov_id` binding emitted; VProg not emitted | `golden_compliance_token_emits_expected_anchors` |
| tag-`0x21` STARK validity is engine-level, not silverc | `docs/PORTRAIT-PHASE-A-UNBLOCK.md`; `crates/kcp-pq-anchor/*` |
| Live ZK status is PARTIAL/PENDING | `docs/CSCI-PROVENANCE.json`; `KNOWN-ISSUES.md` |

Test files: `crates/portrait-sema/src/lib.rs` (`#[cfg(test)] mod tests`),
`crates/portrait-cli/tests/golden.rs` — both under the Portrait workspace
(`portrait/portrait` in the Portrait repository). Run with `cargo test --workspace`
from that root.
