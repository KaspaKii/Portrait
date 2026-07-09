# Covenant patterns for Kaspa

**kaspa-compliance-patterns** · v0/v1 — unaudited — testnet first
**Steward:** Stichting Kii Foundation (Dutch non-profit foundation, incorporated 2026-06-21) · **Licence:** MIT
**Status:** Publication-ready (v0.1.0).

> Every quantitative or behavioural claim below resolves to the public evidence
> index ([docs/EVIDENCE.md](docs/EVIDENCE.md)) or to repository code. The
> identifiers in brackets (e.g. `[KCP-TR-001]`) are those entries. Testnet
> transaction ids are perishable by design — testnets reset.

## 1. What this is

A small, honest library of covenant compliance patterns for Kaspa, written in
Rust against the released Toccata engine (rusty-kaspa v2.0.0). Each pattern is
a worked, testable building block — not a product, not a standard, not audited.
The library is a Foundation public good; it carries no token and makes no
investment claim.

Five patterns, plus shared plumbing, share one workspace (234 tests, default
features, `cargo test --workspace`, verified 2026-06-22; previously 275 under
`--all-features` on 2026-06-12 — the `--all-features` path currently has a
transitive `cc-1.2.63` upstream build issue, to be investigated before v0.2;
`cargo clippy -D warnings` clean):

| Crate | Pattern | Maturity |
|---|---|---|
| `kcp-transferable-record` | Transferable registry record | v0 — on-chain lineage, off-chain-validated invariants |
| `kcp-sealed-lineage` | Append-only sealed evidence lineage | v0 — on-chain lineage, off-chain-validated invariants |
| `kcp-vault` | Covenant-locked custody (timelock / multisig) | **v1 — consensus-enforced** (multisig + timelock + composite Any; All offline engine-proven) |
| `kcp-ktt-token` | KCC20-shape-aligned regulated-token profile | v0 — off-chain state transitions, on-chain target enforce-verified |
| `kcp-paired-attestation` | Two-party mutual attestation | **v1 — consensus-enforced two-datasig**; v0 off-chain mating retained |
| `kcp-common` | Shared plumbing incl. P2SH covenant spend-path | — |

## 2. Evidence (testnet-10, kaspad v2.0.0)

Each pattern was exercised on a live testnet running the released Toccata code.
The network ("Toccata testnet" per the kill-date) is testnet-10 on rusty-kaspa
v2.0.0 `[KCP-NET-001]`.

- **transferable-record** — record create + controller-rotation transfer `[KCP-TR-001]`.
- **sealed-lineage** — genesis + append, L-1..L-4 validated `[KCP-SL-001]`.
- **vault v0** — real-opcode script compiled, digest anchored `[KCP-VT-001]`.
- **vault v1** — value **locked under a 2-of-2 multisig covenant (P2SH) and
  released by satisfying it** — consensus-enforced `[KCP-VT-002]`.
- **ktt-token** — full issue → transfer → burn over the 4-field KCC20 state shape `[KCP-KTT-001]`.
- **paired-attestation v0** — two-party commit + mate, off-chain disclosed-blind proof `[KCP-PA-001]`.
- **paired-attestation v1** — value **released only on two valid independent
  oracle data-signatures, enforced on-chain by `OP_CHECKSIGFROMSTACK`** `[KCP-PA-002]`.
- **P2SH spend-path** — lock under a redeem script and spend by satisfying it,
  engine-preflighted `[KCP-P2SH-001]`.
- **state-continuity covenant shape — first LIVE covenant-id-bound deployment.**
  The anchor-only reserve covenant ran a covenant **genesis + append** on
  testnet-10, the append accepted by the live consensus covenant engine
  (`validateOutputState` introspection + oracle `checkSig`) `[KCP-RE-003]`.
  covenant_id
  `7ba54cfa7fc3e644a09ff68f8cf41a9a4c24f561f627ee6539b8c803b4f7786e`;
  genesis_tx
  `fcecef64c666c4935ef746790165493eab420f4ef8e0bb7992dd49496394c3f0`;
  append_tx
  `980ca03aa11df9581d1f6080a409558fe805104710a829b008fc39a9a96883ae`.
  v0, unaudited, **synthetic** data. The three pattern covenants share this
  covenant-id-bound shape and were then **also deployed live** on testnet-10
  `[KCP-SL-003, KCP-TR-003, KCP-KTT-003]` (see §3).

## 3. What "consensus-enforced" means here

For vault (multisig/timelock) and paired-attestation (two-datasig), value is
locked under a real covenant script via pay-to-script-hash and can only be
moved by satisfying that script. Before any transaction is submitted, the spend
is run through the **real rusty-kaspa script engine offline**
(`verify_p2sh_spend_offline`); a passing preflight means the engine has already
performed the genuine signature verification. The v0 patterns instead carry
their structured invariants in a transaction payload and validate them
off-chain — honest building blocks, with consensus enforcement as the
documented next step.

That next step has since moved twice. First, each v0 pattern's state-transition
rules were additionally authored as a **covenant-id-bound state-continuity
covenant** and proven against the real engine — ktt-token `[KCP-KTT-002]`,
sealed-lineage `[KCP-SL-002]`, transferable-record `[KCP-TR-002]`. Second, all
three were then **deployed live on testnet-10**: each performed a
consensus-enforced covenant-id-bound genesis + append — sealed-lineage
`[KCP-SL-003]`, transferable-record `[KCP-TR-003]`, ktt-token `[KCP-KTT-003]`.
The live network accepted the **valid** transition (sequence increment,
immutable identity, correct authorising signature); rejection of invalid
transitions was proven in the **offline v2.0.0-engine preflight** of the same
construction plus the engine proofs above — not by live submission of invalid
transactions. Covenant introspection is active on testnet-10 `[KCP-NET-002]`;
the engine proofs carry to the release engine `[KCP-COV-SKEW-001]`, and
covenant-id genesis is proven on it `[KCP-COV-GEN-001]`.

## 4. Substrate findings (verified, contributed back)

Building these surfaced several engine-level facts, each verified directly
against rusty-kaspa source and the script engine:

- The KCC20 enforcement primitives (`validateOutputState` and friends) are
  **real and engine-enforced** — upstream's own `kcc20_tests` pass including
  negative cases `[SS-026]`.
- The library builds its two-datasig covenant **directly from opcodes**
  (`OP_CHECKSIGFROMSTACK`), so it does **not** depend on the SilverScript
  `checkDataSig` builtin.
- Kaspa's `OP_CHECKLOCKTIMEVERIFY` **pops** the deadline (Bitcoin peeks) and
  `OP_CHECKMULTISIG` needs **no** dummy element; covenant opcodes like
  `OP_CHECKSIGFROMSTACK` are budgeted in script-units, not legacy sig-ops
  `[KCP-VT-002, KCP-PA-002]`.

## 5. Honest maturity

- **Unaudited.** No external security review has been performed. Do not hold
  mainnet value with these patterns.
- **Testnet-first.** All evidence is on testnet and is perishable.
- **The live covenant deployments show the mechanism, nothing more.** Each
  covenant-id-bound lineage on testnet-10 — the anchor-only reserve demonstrator
  `[KCP-RE-003]` and the three pattern covenants `[KCP-SL-003, KCP-TR-003,
  KCP-KTT-003]` — is v0, unaudited, seals **synthetic** data, and gates no value
  movement. None is a mainnet deployment and none is an audit.
- **Shape-aligned, not conformant.** The KTT profile aligns to the KCC20
  reference contracts, which their authors document as examples, not a
  production standard `[SS-021]`.
- **Scope is deliberate.** The Poseidon/ZK commitment upgrade is a documented
  next step, not a claim — see `KNOWN-ISSUES.md`. (Composite `Any` vault spends are consensus-enforced and live `[KCP-VT-003]`;
  `All` branch selection is offline engine-proven.)

---

*Author: Stichting Kii Foundation. Published under MIT licence.*
