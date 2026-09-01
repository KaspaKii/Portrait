# Pattern catalogue

The library ships five patterns + one shared-plumbing crate. Each pattern
is a worked, testable building block written as a *shape-aligned profile*
on top of SilverScript's covenant-declaration system and the upstream
KCC20 reference contracts.

| Crate | Pattern | Status (pre-production, unaudited, testnet) |
|---|---|---|
| [`kcp-vault`](./crates/kcp-vault.md) | Covenant-locked custody (timelock + multisig) | **v1** — consensus-enforced on testnet-10 (multisig, timelock, composite Any/All P2SH) — `[KCP-VT-001, KCP-VT-002]` |
| [`kcp-paired-attestation`](./crates/kcp-paired-attestation.md) | Two-party mutual attestation | **v1** — consensus-enforced two-datasig (CSFS) on testnet-10 — `[KCP-PA-002]` |
| [`kcp-sealed-lineage`](./crates/kcp-sealed-lineage.md) | Append-only sealed evidence lineage | v0 lineage; **state-continuity covenant LIVE on testnet-10** — `[KCP-SL-003]`; multi-step lineage sequence 0→1→2 live — `[KCP-RE-004]` |
| [`kcp-transferable-record`](./crates/kcp-transferable-record.md) | Transferable registry record with lineage continuity | v0 lineage; **state-continuity covenant LIVE on testnet-10** — `[KCP-TR-003]` |
| [`kcp-ktt-token`](./crates/kcp-ktt-token.md) | KCC20-shape-aligned regulated-token profile | v0 state machine; **state-continuity covenant LIVE on testnet-10** — `[KCP-KTT-003]` |
| [`kcp-common`](./crates/kcp-common.md) | Shared P2SH covenant spend-path + offline real-engine preflight + KIP-14 payload helpers | `[KCP-P2SH-001]` round-trip live on testnet-10 |

## Pattern dependency graph

```
                  ┌───────────────┐
                  │   kcp-common  │  ◄─── shared by every pattern crate
                  │ (p2sh, sighash,│
                  │  digest, tx,   │
                  │  wallet, wrpc) │
                  └───────┬───────┘
                          │
       ┌──────────┬───────┼───────┬──────────────┬────────────┐
       ▼          ▼       ▼       ▼              ▼            ▼
  kcp-vault   kcp-paired-  kcp-   kcp-           kcp-
              attestation  sealed-  transferable- ktt-token
                           lineage record
```

## Naming disambiguation

KCC20, KRC-20, and KTT are three different things:
- **KCC20** is the covenant-native token example set in
  [`kaspanet/silverscript`](https://github.com/kaspanet/silverscript) (the
  shape this library aligns to, documented by its authors as
  pre-standard examples).
- **KRC-20** is a separate inscription-style token convention that
  originated with Kasplex.
- **KTT** (Kaspa Trust Token) is this library's regulated-token profile,
  KCC20-shape-aligned.

None of these is a synonym for another.
