# Next step: on-chain state/lineage enforcement (the introspection tier)

This note scopes the remaining on-chain enforcement work for three patterns —
`sealed-lineage`, `transferable-record`, and `ktt-token` — so it can be
executed deliberately. It is grounded in verified engine facts; it is a design,
not a claim of completed work.

## Two tiers of covenant

What this library has **proven on-chain** (vault, paired-attestation) is the
**signature-and-condition** tier: a P2SH redeem script that checks signatures,
timelocks, multisig, and data-signatures against a fixed satisfier. The spend
is self-contained — nothing about the *next* state matters.

What these three patterns need is the **state-continuity** tier: the covenant
must verify that the spending transaction's **output recreates the covenant
with a correctly-transitioned state** (e.g. `seq+1`, same identity, conserved
supply). This is fundamentally harder: the redeem script must introspect the
transaction's outputs and assert the successor is well-formed.

## The primitive exists and is verified

- Introspection opcodes are present in rusty-kaspa v2.0.0 (`90dbf07`):
  `OpInputCovenantId`, `OpOutputCovenantId`, `OpCovInputCount`,
  `OpCovOutputCount`, `OpTxOutputSpk`, `OpTxInputSpk`, `OpAuthOutputCount`,
  `OpAuthOutputIdx` (`crypto/txscript/src/opcodes/mod.rs`).
- `validateOutputState` / `validateOutputStateWithTemplate` / `readInputState`
  are **SilverScript-level** constructs that lower to these opcodes; they are
  **real and engine-enforced** — KCC20's `kcc20_tests` pass against the engine
  including negative cases `[SS-026]`. The KCC20 reference contracts
  (`kcc20.sil`, `kcc20-minter.sil`) are the worked model.

So the tier is feasible. The question is *how* to author the covenants.

## Two implementation paths

| Path | What it is | Effort | Risk |
|---|---|---|---|
| **A. SilverScript-compiled** | Write the covenant as a `#[covenant(...)]` declaration, compile with `silverc` to a script, embed the bytes, drive spends from Rust | Medium per pattern; needs a compile step in the build/release pipeline | Lower — the compiler generates correct introspection + template hashing; matches KCC20's proven approach |
| **B. Hand-built opcodes** | Assemble the introspection redeem directly with `ScriptBuilder` (like vault/PLA) | High per pattern — output introspection + template-hash reconstruction + state-field validation, far more than the CSFS choreography | Higher — easy to get the self-recreation/template hash subtly wrong; the offline engine preflight catches execution errors but the design surface is large |

**Recommendation: Path A.** It reuses the verified KCC20 machinery, keeps the
covenants readable/auditable, and is how the upstream reference does it. Path B
is a fallback only if a compile step is unacceptable.

## Per-pattern scope

### ktt-token — most tractable (reference exists)
- **Invariant to enforce:** the 4-field KCC20 state transition (supply
  conservation when `!isMinter`, no minter escalation, ownership authorisation)
  on each transfer/mint/burn.
- **Approach:** adapt `kcc20.sil` + `kcc20-minter.sil` into a Kii KTT covenant
  with the compliance-attestation fields, compile, and drive issue→transfer→burn
  spends from Rust. Verify with a `kcc20_tests.rs`-shaped harness against the
  engine, then live evidence `KCP-KTT-002`.
- **Dependency:** none blocking — `validateOutputStateWithTemplate` works in
  stock SilverScript (no `checkDataSig` needed). Subject to the KCC20
  shape-change SLA `[binding_policies.kcc20_shape_change_sla]`.
- **Effort:** the largest of the three patterns.

### sealed-lineage — covenant-id continuity
- **Invariant:** each append re-locks the lineage UTXO under the same covenant
  (same covenant-id) with `seq+1`, identical `lineage_id`, and a non-decreasing
  `t_bucket` within the 90-day envelope (L-1..L-4).
- **Approach:** a `#[covenant(binding=cov)]` declaration using `readInputState`
  for the predecessor and `validateOutputState` for the successor, asserting
  the L-invariants in the policy function. `covenant_id.sil` is the structural
  model (`OpInputCovenantId` family) `[SS-016]`.
- **Effort:** medium; the invariants are simple integer/identity checks once the
  state-recreation scaffold is in place.

### transferable-record — single-controller continuity
- **Invariant:** exactly one live controlling record; transfer reassigns the
  controller while preserving the record identity across the covenant chain.
- **Approach:** same `validateOutputState` scaffold; the policy asserts the new
  controller key is authorised by the current controller's signature and the
  record identity is carried forward. Single-output continuity (no fan-out).
- **Effort:** medium, similar to sealed-lineage.

## What unblocks this

1. A `silverc` compile step usable from the library's build/release (Path A).
2. The publication caveats are independent of this work (A: licence; D:
   provenance) — see `KNOWN-ISSUES.md`.
3. The optional upstream `checkDataSig` fix `[SS-024-v4]` is **not** required
   for this tier (these patterns use `validateOutputState`, not `checkDataSig`).

## Status of this note

The signature-and-condition tier is complete and proven on-chain (vault all shapes,
PLA two-datasig); the state-continuity tier is feasible, scoped here, and is the
next deliberate workstream. No half-built introspection covenant is shipped —
that would be the kind of inflated maturity the project forbids.

The state-continuity tier described here is now
**engine-proven** for all three patterns `[KCP-KTT-002, KCP-SL-002, KCP-TR-002]`
plus the reserve covenant `[KCP-RE-001]`, and Toccata covenants are confirmed
**active on testnet-10** `[KCP-NET-002]`. The remaining work — taking one of
these covenants from engine-proven to a live covenant-id-bound transaction — is
scoped in the sequel:
[NEXT-STEPS-covenant-live-deploy.md](NEXT-STEPS-covenant-live-deploy.md).
