# Evidence

Public evidence index for `kaspa-compliance-patterns`. The bracketed identifiers
in this index (e.g. `[KCP-VT-002]`) resolve to the rows below. Each
on-chain item is a real transaction on **testnet-10** (rusty-kaspa **v2.0.0**,
the released Toccata build) and is independently verifiable on a Kaspa testnet
explorer.

> **v0/v1 — unaudited — testnet-first.** No external security audit. No mainnet
> deployment. **Testnet evidence is perishable** — testnets reset by design, so
> these transaction ids are evidence of the mechanism at a point in time, not
> permanent records; the examples can be re-run to refresh them. Synthetic data
> only.

This index is a **curated subset** scoped to the library's five patterns and the
substrate findings. One further live run — the anchor-only reserve-attestation
covenant `[KCP-RE-003]`, a synthetic mechanism demonstrator that gates no value
and is not a library pattern — is documented in the launch note and the fuller
evidence register, not indexed here.

## Network

| Id | What it establishes |
|---|---|
| `KCP-NET-001` | Evidence network is testnet-10 on released rusty-kaspa v2.0.0 (post-testnet-reset). |
| `KCP-NET-002` | Toccata covenant introspection is **active** on testnet-10 (virtual DAA past the activation score), so covenant-id-bound transactions are consensus-eligible. |

## Patterns and plumbing

| Pattern | Maturity | On-chain evidence (testnet-10) | Id |
|---|---|---|---|
| `kcp-common` P2SH spend-path | live lock→spend round-trip + offline real-engine preflight | lock `ee8c43dd…`, spend `f4f0ba09…` | `KCP-P2SH-001` |
| `kcp-vault` | **v1 consensus-enforced** 2-of-2 multisig + timelock | lock `973707fc…`, spend `81ab3171…` | `KCP-VT-002` |
| `kcp-vault` composite | **v1 consensus-enforced** Any(2) branch selection (live); All(leaves) offline engine-proven | lock `291f0e56…`, spend `fd01e75f…` | `KCP-VT-003` |
| `kcp-paired-attestation` | **v1 consensus-enforced** two-datasig (CSFS) | lock `e0febd53…`, spend `8484003d…` | `KCP-PA-002` |
| `kcp-sealed-lineage` | state-continuity covenant **engine-proven** (script VM; reproducible in-repo, `tests/covenant_engine.rs`) **+ live** (not reproducible) | genesis `34d0e6f7…`, append `c7c24194…` | `KCP-SL-002`, `KCP-SL-003` |
| `kcp-transferable-record` | state-continuity covenant **engine-proven** (script VM; reproducible in-repo, `tests/covenant_engine.rs`) **+ live** (not reproducible) | genesis `7d26a125…`, append `9d27f8fa…` | `KCP-TR-002`, `KCP-TR-003` |
| `kcp-ktt-token` | state-continuity covenant **engine-proven** (script VM; reproducible in-repo, `tests/covenant_engine.rs`) **+ live** (not reproducible) | genesis `85e4cc37…`, append `24244754…` | `KCP-KTT-002`, `KCP-KTT-003` |
| All five patterns | v0 testnet evidence on the released engine | per-pattern carrier / lineage transactions | `KCP-TR-001`, `KCP-SL-001`, `KCP-VT-001`, `KCP-KTT-001`, `KCP-PA-001` |

Full transaction ids (the table above abbreviates for readability; these are the
values to paste into a testnet-10 explorer or
`GET https://api-tn10.kaspa.org/transactions/{txid}`):

- `KCP-P2SH-001` lock:    `ee8c43ddff8898c4571e405f9e60b2d01a3d60661f34e7e68e3c89f7f032d475`
- `KCP-P2SH-001` spend:   `f4f0ba092a6738a73f1318bcb8c4f0e09f841802c0c7591cdb7798f99d64c6db`
- `KCP-VT-002` lock:      `973707fc5630e350b63915b1ac62a2c832f797be3669ead25d96634e4df0d06f`
- `KCP-VT-002` spend:     `81ab3171bd0f942134504dd4cd28bea4caca0807501747a74c3f2a270517596d`
- `KCP-VT-003` lock:      `291f0e56a60daf3b49b35ff537258e105e16c29510b01adadf051bfa5ce67db1`
- `KCP-VT-003` spend:     `fd01e75fa312613962598c5470f39fc35158c9ce9d603849b9cbf9e4c027c062`
- `KCP-PA-002` lock:      `e0febd53595f5e3d343f5280cba58481ad66908d24856b1078afca94e2ecb30d`
- `KCP-PA-002` spend:     `8484003dc00034f51d50988f1a2ac7f195f8ecceaebdcced725b63e50f28a2b7`
- `KCP-SL-003` genesis:   `34d0e6f7f06f045ae36c2236ba44d84ae10e4d05bfceabad17d31223a11774f1`
- `KCP-SL-003` append:    `c7c24194ae311674b03b3a83f7d3b3c9d00d51b18c701d12b94d04489621a736`
- `KCP-TR-003` genesis:   `7d26a125c8c03b2503e761fc282136d4c2c7f9e53e4114a44ee6478219f27c4e`
- `KCP-TR-003` append:    `9d27f8fa77f0de68a435b3515b3cd4f89060bb201699dd0e8da80369941dee40`
- `KCP-KTT-003` genesis:  `85e4cc3791c00935d18adc8480190fce7b605e5204ad72e452fa2b6da00282d2`
- `KCP-KTT-003` append:   `24244754dd7825981265d2119507425bf4e96d40458549ab8b80b808cb0349ea`

The three state-continuity covenants (`sealed-lineage`, `transferable-record`,
`ktt-token`) were each deployed in a **covenant-id-bound genesis + append** and
validated by the live consensus covenant engine. Only the **valid** successor
was submitted on-chain; rejection of invalid successors is proven by the offline
v2.0.0-engine preflight and the engine proofs `[KCP-SL-002, KCP-TR-002,
KCP-KTT-002]`, not by live submission of invalid transactions.

**Superseded 2026-09-01.** The original `KCP-SL-002` / `KCP-TR-002` /
`KCP-KTT-002` engine proofs ran on an archived, out-of-repo harness pinned to a
different rusty-kaspa commit (the exact pin is `[FACT-NEEDED]` — the harness
predates this repo and carries the `KCP-COV-SKEW-001` cross-version argument
rather than the release pin). They are replaced by the in-repo
`tests/covenant_engine.rs` in each of the three crates, which runs on
**v2.0.0 / `90dbf07`**, the same engine the workspace pins.

Those in-repo engine proofs are **reproducible from a clean clone** — each loads
the committed `covenant/*.compiled.json`, splices per-state scripts into its
`state_layout` region, and runs them through the pinned v2.0.0 script VM with
covenants enabled. Every rejection is two-sided (the violating transition is
refused *and* the same transition with the offending field restored is accepted),
so the failure is pinned to the named invariant. They run in the gate under
`cargo test --workspace --all-features`.

**What the in-repo proofs do NOT establish.**

1. *That the committed artifact is the script that was deployed.* Nothing in this
   repo records the deployed script, its scriptPubKey, or the on-chain
   `covenant_id` of the `-003` genesis outputs, and the `covenant_id` is derived
   from the funding outpoint rather than from the script — so it cannot be
   recomputed from the artifact. That correspondence is attested by an **archived
   out-of-repo byte capture** and is **not reproducible in-repo**. The one in-repo
   corroboration is an execution-cost fingerprint: re-running each committed
   artifact consumes exactly the script-unit count recorded for that covenant's
   live preflight — 107 149 (`KCP-SL-003`), 105 047 (`KCP-TR-003`), 111 410
   (`KCP-KTT-003`) — asserted by `*_engine_cost_matches_recorded_live_preflight`.
   Script units are a deterministic function of the executed opcode trace, so a
   different program body would be very unlikely to match; but the count is
   insensitive to the state *values*, and the recorded numbers are themselves
   out-of-repo. This is evidence, not a binding.
2. *Transaction-level validity.* Only input 0's script runs: no transaction mass,
   no KIP-9 storage mass, no standardness. The covenants place no floor on the
   output value.
3. *The live half.* Submitting a transaction is not reproducible from a test; the
   live claims still rest on the perishable testnet-10 ids above.

A multi-step run
(a lineage advancing several appends deep) and an auditor reader that re-verifies
a live lineage head from public information are included as examples (see
[INDEX.md](INDEX.md)).

## Engine-level findings (verified against rusty-kaspa source + the script engine)

| Id | Finding |
|---|---|
| `SS-026` | The KCC20 enforcement primitives (`validateOutputState` and friends) are real and engine-enforced — upstream's own `kcc20_tests` pass, including negative cases. |
| `KCP-COV-SKEW-001` | Covenant **validation semantics are byte-identical** between the engine the proofs ran on and the released v2.0.0 engine, so engine proofs and embedded compiled scripts carry to the release. |
| `KCP-COV-GEN-001` | The covenant-id **genesis** mechanism is engine-proven on the v2.0.0 release engine (upstream `covenants::tests` pass, including the wrong-id reject and the genesis→continuation handoff). |
| `KCP-FEE-001`, `KCP-FEE-002` | Live-network operational findings: relay-fee floor sizing, and a mempool already-spent race handled by retry. |

Other engine facts captured during the build (documented in the launch note):
Kaspa's `OP_CHECKLOCKTIMEVERIFY` **pops** the deadline (so a P2SH timelock redeem
omits `OP_DROP`); `OP_CHECKMULTISIG` consumes **no** dummy element; covenant
outputs require the Toccata transaction version and commit a compute budget; and
KIP-9 storage mass penalises small outputs (a covenant output is two storage
units).

## ERC20→KTT Rosetta on-ramp (kii-solidity-compat)

| Id | What it establishes | On-chain evidence (testnet-10) |
|---|---|---|
| `KCP-ERC20-WEDGE-001` | `kii-solidity-compat` ERC20-shaped API drives a complete KTT issue→transfer→burn cycle over `kcp-ktt-token`; all three carrier txes independently accepted on TN10. | issue `f0e05875…`, transfer `23cfe7a9…`, burn `3b588f56…` |

**Verified 2026-06-27.** Three transactions submitted via `examples/erc20-to-ktt-wedge`
(binary `erc20-to-ktt-wedge`, node `wss://vector-10.kaspa.green`) and confirmed
by `GET https://api-tn10.kaspa.org/transactions/{txid}` returning `is_accepted: true`
for each. Full txids:

- issue:    `f0e058758845a7c9c938da726c4f276f93d35be3a45d79902cee0abef64567e5`
- transfer: `23cfe7a9576eace41026fe67db0979774d2a13435ab61f3ee56bb52ce80415d9`
- burn:     `3b588f562f03e5dd0d6d81ee17380b8e171e3450dcb899ab79f6d66dfb9e8410`

Token: `HelloKTT (HKT)`, token_id `92d90fc8871e4481a6edbacaeb6f80c49502e90ae40ae6d5e7f92e26bb4e095e`,
decimals 8, genesis supply 1 000 000 HKT. v0 — unaudited — ERC20-shaped
kii-solidity-compat API over kcp-ktt-token; synthetic data, no value gated.

---

*This is a curated public extract. Quantitative and behavioural claims in the
published docs trace here or to repository code; a fuller internal evidence
register exists and is available on request. Nothing above should be read as a
mainnet, audited, or production claim.*
