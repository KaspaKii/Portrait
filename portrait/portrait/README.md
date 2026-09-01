# Portrait compiler workspace

> **Pre-production, unaudited, testnet-only.** No part of this has been through a
> security audit. On-chain claims below are dev-mode / off-chain unless stated
> otherwise. Testnet evidence is perishable (testnets reset by design).

The Rust toolchain for **Portrait** — a high-level surface language that compiles
down to Kaspa SilverScript covenants (`.sil`) plus, for cross-layer patterns, a
RISC Zero guest program.

## Verified artifacts (what actually works today)

These are the empirically checked outputs, reproducible in this repo:

- **`portrait engrave` emits `.sil` that the real `silverc` accepts.** Two worked
  examples compile to exit code 0 against the installed `silverc`
  (`~/.cargo/bin/silverc`):
  - `../examples/Counter.sil` → `silverc --ctor ../examples/Counter_ctor.json -c ../examples/Counter.sil` → `../examples/Counter.json` (exit 0).
  - `../examples/tier3-demo/ComplianceToken.sil` → `silverc --ctor ../examples/tier3-demo/ComplianceToken_ctor.json -c ../examples/tier3-demo/ComplianceToken.sil` → `ComplianceToken.json` (exit 0).
- **`tier3-demo` emits a covenant plus a RISC Zero guest with an auto-derived
  KovId.** `examples/tier3-demo/compliancetoken_guest_main.rs` carries
  `const COV_ID: [u8; 32] = …` derived as `sha256(silverc compiled script bytes)`
  — the same covenant ID the on-chain script commits to. The guest builds the
  104-byte CSCI journal `covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]`.
- **349 workspace tests pass.** `cargo test --workspace` from this directory →
  349 passed, 0 failed (re-verified 2026-07-09 from a fresh clone, summed across
  workspace crates). Tests that shell out to `silverc` skip cleanly with a printed
  `SKIP` notice when it is not installed, so the suite is green on a fresh clone
  without it.
- **35 covenant sources** compile through the pipeline (engrave/verify →
  silverc exit 0), drawn from `../library/` and `../examples/`. By family:
  finance 18, custody 3, governance 2, attestation 1, state 1, cross-layer
  (vProg) 10. `DigitalReit.portrait` is the only multi-role source (emits 2
  `.sil`). The authoritative per-pattern list lives in the bundled
  `compliance-patterns/` library.
- **10 cross-layer (vProg) patterns** under `../library/vprog/`: each
  `portrait engrave` → silverc exit 0 and `portrait atelier-build` emits a RISC
  Zero guest. **Five are settled live on testnet-10** (ProofOfReserves,
  ComplianceCredential, ConfidentialTransfer, BatchRollup, PrivateVoting) — a real
  RISC Zero STARK verified in-consensus via the `tag-0x21` precompile, each with a
  per-pattern negative control the live node rejected. **The other five are
  emit-verified only** (MerkleProofOfSolvency, PrivateOrderMatch,
  PrivateVickreyAuction, ZkAllowlistTransfer, ZkExecutionRollup) — they compile,
  engrave, and emit a guest, but are **not** settled live. Honest residuals for
  the live vProgs (fixed sample inputs, `sha256` commitments, v1 audit key) plus
  the provenance + negative-control txids are in
  `compliance-patterns/examples/portrait-settlement/PROVENANCE.json`. Language
  features exercised: real `blake2b` builtin (true HTLC hashlock) + invariants
  `spending_cap` / `multisig_threshold` / `temporal_guard`.
- **`portrait new` + `portrait verify`.** `portrait new <Name> --template
  counter|escrow|csci|treasury` scaffolds a covenant; `portrait verify` emits a
  human summary + a rederivable `*.hallmark.json`.
- **Live on TN10 (testnet, perishable).** The `CsciInstrument.sil` covenant
  self-enforced its state machine on-chain (settle `5731a203…`, genesis
  `27de5b3c…`) and a cross-bound settle (`60738aff…`) committed the engine
  per-instance covenant_id into the STARK journal. Full provenance +
  negative controls are archived in the bundled `compliance-patterns/`
  library (see `compliance-patterns/examples/portrait-settlement/PROVENANCE.json`).

## Pipeline

```
source.portrait
  → portrait-syntax   (parse)
  → portrait-sema     (check)
  → portrait-ir       (lower to Cartoon IR)
  → portrait-project  (Pounce — projection)
  → portrait-emit     (Engraver — .sil + CTOR JSON)
  → silverc           (accepts .sil → JSON script)
```

Twelve crates: `portrait-syntax`, `portrait-sema`, `portrait-ir`, `portrait-pounce`,
`portrait-project`, `portrait-emit`, `portrait-atelier`, `portrait-plan`,
`portrait-verify`, `portrait-lens`, `portrait-compose`, `portrait-cli`.

## Quick start

```sh
cargo test --workspace
cargo run --bin portrait -- engrave ../examples/counter.portrait
silverc --ctor ../examples/Counter_ctor.json -c ../examples/Counter.sil
```

## Status board

Legend — 🟢 verified and tested · 🟡 built, not fully verified · ⬜ planned.

| Component | State | Evidence |
|---|---|---|
| Parser (`portrait-syntax::parse`) | 🟢 | Parses the example sources; exercised by workspace tests. |
| `portrait engrave` → `.sil` accepted by `silverc` | 🟢 | `Counter.sil` + `ComplianceToken.sil` compile, exit 0, JSON emitted. |
| CTOR JSON emission (`emit_ctor`) | 🟢 | `Counter_ctor.json`, `ComplianceToken_ctor.json` consumed by `silverc --ctor`. |
| Covenant-ID binding emission (`require(proof_cov_id == OpInputCovenantId(0))`) | 🟢 | Emitted in transition functions for VProg-paired roles; `OpInputCovenantId` is a real silverc surface op; accepted by `silverc`. |
| tier3-demo RISC Zero guest + auto-derived KovId | 🟢 | `compliancetoken_guest_main.rs` `COV_ID` = `sha256(script bytes)`; 104-byte journal schema confirmed. |
| Workspace test suite | 🟢 | `cargo test --workspace` → 349 passing, 0 failed (verified 2026-07-09 from a fresh clone; silverc-dependent tests skip cleanly when silverc is absent). |
| Library breadth (35 covenant sources) | 🟢 | 35 sources across `../library/` + `../examples/`, each engrave/verify → silverc exit 0. Authoritative per-pattern list in `compliance-patterns/`. |
| Cross-layer (vProg) catalogue (10 patterns) | 🟢 | 5 settled live on TN10 (ProofOfReserves, ComplianceCredential, ConfidentialTransfer, BatchRollup, PrivateVoting — real STARK verified in-consensus, per-pattern negative controls rejected); 5 emit-verified only (MerkleProofOfSolvency, PrivateOrderMatch, PrivateVickreyAuction, ZkAllowlistTransfer, ZkExecutionRollup). See `compliance-patterns/examples/portrait-settlement/PROVENANCE.json`. |
| `portrait new` scaffold + `portrait verify` summary/hallmark | 🟢 | CLI verified; `verify` emits rederivable `*.hallmark.json`. |
| Semantic checks (`portrait-sema::check`) — §4.4 type stack | 🟢 | Covers value-conservation, capability/authorization, refinement; scalar-return bypass closed. **Scope:** structural/relational, not an SMT solver (no cross-field flow proof). |
| Self-enforcing covenant settled LIVE on TN10 | 🟢 | `CsciInstrument.sil` settle `5731a203…`; cross-bound settle `60738aff…` (per-instance covid in journal). Perishable testnet evidence; see PROVENANCE.json. |
| RISC Zero real STARK (`RISC0_DEV_MODE=0`) verified in-consensus | 🟢 | Real-instrument settle `11b4d7d9…` accepted only because tag-0x21 verified the STARK; tampered-journal reject `e44d4f7b…`. |
| Single silverscript redeem that *both* verifies the STARK and binds its journal | ⬜ | Needs `silverc` `OpZkPrecompile` emission — **deliberately not pursued** (no upstream path; nothing to kaspanet). tag-0x21 verification runs as a separate verifier input; 2-input combined enforcement proven offline against v2.0.0. |
| Mainnet deployment / external audit | ⬜ | None. All evidence is testnet-10, unaudited, perishable. |

## Honest limitations

- **Semantic checks are structural/relational, not an SMT solver.**
  `portrait-sema::check` enforces value-conservation, capability/authorization,
  and refinement, but **per-field** only — there is no cross-field flow proof. A
  malformed-but-structurally-valid flow the checker does not model could slip
  through.
- **Cross-layer ZK verification is not emitted by Portrait.** Portrait emits the
  covenant-ID binding (`OpInputCovenantId`), which `silverc` supports. The STARK
  validity check is an **engine-level** KIP-16 tag-0x21 precompile and is *not* a
  `silverc` surface function — so Portrait cannot emit it today, and a single
  redeem that both verifies the proof and binds its journal is **deliberately not
  pursued** (no upstream path; nothing to kaspanet). tag-0x21 verification runs as
  a separate verifier input; the 2-input combined design is proven offline against
  the v2.0.0 engine. The covenant-id cross-binding is closed in-house (the STARK
  journal commits the engine per-instance covenant_id).
- **Testnet-only, unaudited, perishable.** Live evidence is on TN10 and may not
  resolve after a testnet reset. No mainnet deployment; no external audit.

## Licence

MIT — Stichting Kii Foundation.
