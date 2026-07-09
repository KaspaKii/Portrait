# Documentation index

**kaspa-compliance-patterns** — covenant compliance patterns for Kaspa (Toccata).

## Start here

- [README](../README.md) — what the library is, the crate map, honest maturity.
- [LAUNCH-NOTE](../LAUNCH-NOTE.md) — "Covenant patterns for Kaspa".
- [KNOWN-ISSUES](../KNOWN-ISSUES.md) — scope boundaries and next steps.
- [EVIDENCE](EVIDENCE.md) — public evidence index (on-chain tx ids per `[KCP-*]` id).

## Patterns

| Crate | README | On-chain enforcement | Evidence |
|---|---|---|---|
| `kcp-transferable-record` | [README](../crates/kcp-transferable-record/README.md) | lineage; **state-continuity covenant engine-proven + live** | `KCP-TR-001`, `KCP-TR-002`, `KCP-TR-003` |
| `kcp-sealed-lineage` | [README](../crates/kcp-sealed-lineage/README.md) | lineage; **state-continuity covenant engine-proven + live** | `KCP-SL-001`, `KCP-SL-002`, `KCP-SL-003` |
| `kcp-vault` | [README](../crates/kcp-vault/README.md) | **multisig + timelock + composite (P2SH), live** | `KCP-VT-001`, `KCP-VT-002`, `KCP-VT-003` |
| `kcp-ktt-token` | [README](../crates/kcp-ktt-token/README.md) | 4-field state; **state-continuity covenant engine-proven + live** | `KCP-KTT-001`, `KCP-KTT-002`, `KCP-KTT-003` |
| `kcp-paired-attestation` | [README](../crates/kcp-paired-attestation/README.md) | **two-datasig CSFS (P2SH)** + off-chain mating | `KCP-PA-001`, `KCP-PA-002` |
| `kcp-common` | (lib docs) | P2SH covenant spend-path plumbing | `KCP-P2SH-001` |

## Shared plumbing (`kcp-common`)

- `canonical`, `digest` — deterministic hashing / script digests (offline).
- `wallet`, `wrpc`, `tx` — key derivation, node client, carrier transactions (feature `wrpc`).
- `p2sh` — lock value under a redeem script and spend by satisfying it, with an
  offline real-engine preflight (feature `wrpc`).

## Running the examples

- [ENVIRONMENT.md](ENVIRONMENT.md) — the environment variables every example
  reads (node URL, wallet key, network suffix, capture path, dry-run, …), with
  which example uses each and its default. Node-facing examples need
  `--features wrpc`.

## Design notes & diagnostics

- [NEXT-STEPS-introspection-enforcement](NEXT-STEPS-introspection-enforcement.md)
  — the engine-tier state-continuity design (now proven `[KCP-*-002, SS-026]`).
- [NEXT-STEPS-covenant-live-deploy](NEXT-STEPS-covenant-live-deploy.md) — how a
  state-continuity covenant went from engine-proven to **live on testnet-10**:
  the three blockers and how each was resolved. **DONE** — all four covenants
  are live (`[KCP-RE-003, KCP-SL-003, KCP-TR-003, KCP-KTT-003]`).
- `kcp-common --example reserve_covenant_live` (feature `wrpc`) — the
  covenant-agnostic live-deploy runner: builds + submits a covenant-id-bound
  genesis+append from an embedded silverc byte-capture (`KCP_CAPTURE_JSON`),
  with a `KCP_DRY_RUN` offline v2.0.0-engine gate. Drives all four covenants.
- `kcp-common --example covenant_auditor` (feature `wrpc`) — **auditor-side**
  reader: from public info (covenant_id + disclosed state scripts) it
  independently verifies the live on-chain lineage head — node-attested
  covenant_id + `P2SH(disclosed state)` match + lineage advanced — without
  trusting the publisher.
- `kcp-common --example node_status` (feature `wrpc`) — read-only probe: node
  version, sync state, virtual DAA, and whether Toccata covenants are active.
- `kcp-sealed-lineage --example auditor_verify` — off-chain auditor verification
  of a sealed lineage against the **real** canonical form + 87-byte payload codec
  (the in-library counterpart to the Kii Reserve Python demo).

## Evidence and provenance

All quantitative/behavioural claims trace to the public evidence index
([EVIDENCE.md](EVIDENCE.md)) — pattern evidence (`KCP-*`) and substrate findings
(`SS-0xx`) with their on-chain transaction ids, the node version, and the
verification dates.

## Build

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Note: `--all-features` currently has a known upstream `cc-1.2.63` build issue
(see CHANGELOG.md). The local gate is `_harness/ci.sh` (default features).

Examples that produce testnet evidence require a synced node and a funded
testnet wallet — see each crate's `examples/`.
