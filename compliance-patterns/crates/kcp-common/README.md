# kcp-common

> **v0 — unaudited — testnet first.**

Shared plumbing for the `kaspa-compliance-patterns` workspace. Used by every
pattern crate (`kcp-vault`, `kcp-paired-attestation`, `kcp-sealed-lineage`,
`kcp-transferable-record`, `kcp-ktt-token`).

Part of the `kaspa-compliance-patterns` workspace, targeting the
[Toccata](https://github.com/kaspanet/rusty-kaspa) hardfork (`rusty-kaspa`
tag `v2.0.0` = commit `90dbf07`).

---

## Modules

| Module | What it provides |
|---|---|
| `access` | Single-controller (`Ownable`) and k-of-n multisig (`Multisig`) access primitives. Pure offline; no engine dependency. Pre-production, unaudited. |
| `cryptography` | BIP-340 `tagged_hash`, `script_digest` re-export, `sign_schnorr` (`wrpc`). EVM equivalents: `MessageHashUtils.sol`, `ECDSA.sol`. `MerkleProof` and standalone CSFS helper deferred. Pure offline (except `sign_schnorr`). Pre-production, unaudited. |
| `security` | `Pausable` (pause-state value type; pure data, no enforcement — callers must check `is_paused()` at each guarded site) and `TimelockController` (DAA-score or unix-seconds structural deadline; `validate()` is shape-only, not a temporal check; not interchangeable with `kcp-vault`'s `SpendCondition` variants). `ReentrancyGuard` intentionally omitted — UTXO model is structurally non-reentrant. Pure offline; no engine dependency. Pre-production, unaudited. |
| `p2sh` | P2SH covenant spend-path: lock-script construction, redeem-script hashing, sighash computation, satisfier signing, signature-script building, **offline real-engine preflight (`verify_p2sh_spend_offline`)**. The preflight runs the real `rusty-kaspa` script engine over a fully-built spend before any RPC — a passing preflight means the engine has performed the genuine signature verification. |
| `digest` | SHA-256 digesting helpers used across patterns + KIP-14 payload commit. |
| `tx` | Transaction construction helpers + `CARRIER_FEE_SOMPI` (the conventional minimum fee allocation for carrier outputs). |
| `wallet` | BIP-32/BIP-44 wallet derivation + Schnorr keypair plumbing (behind `wrpc` feature). |
| `wrpc` | wRPC client config + node connection (behind `wrpc` feature). |
| `canonical` | Canonical serialization helpers for content hashing + binding stable identifiers to bytes. |
| `error` | Shared error type + `Result` alias. |

## Why this exists

Every pattern crate uses the same plumbing to:
- Construct + verify P2SH spends with the real `rusty-kaspa` script engine offline.
- Build KIP-14 payloads for anchored evidence.
- Derive wallet keys consistently with the Kaspa wallet conventions.
- Connect to a Kaspa testnet-10 node via wRPC.

Extracting these into `kcp-common` keeps each pattern crate's source focused
on the pattern itself, and ensures every pattern uses the same engine-preflight
discipline before any live submission.

## On-chain evidence

`kcp_common::p2sh` round-trip is proven live on testnet-10 — `[KCP-P2SH-001]`,
recorded in [`docs/EVIDENCE.md`](../../docs/EVIDENCE.md).

## Features

- `default` — pure-Rust, no rusty-kaspa dep tree. Useful for canonical-
  serialization-only consumers.
- `wrpc` — enables the rusty-kaspa dependency tree (wallet + wRPC + p2sh
  engine preflight). Required by every pattern crate that touches on-chain
  evidence.

## Quick example (offline P2SH preflight)

See [`examples/hello-vault/`](../../examples/hello-vault/) at the repo
root — the shortest path to "the real `rusty-kaspa` script engine accepted
my covenant spend." Adapted directly from the kcp-vault unit test
`multisig_2of2_lock_spend_executes_on_engine`.

## Status + caveats

- **Pre-production, unaudited, testnet-first.** See repo-level
  [`SECURITY.md`](../../SECURITY.md) and
  [`KNOWN-ISSUES.md`](../../KNOWN-ISSUES.md).
- The engine reference is pinned at `rusty-kaspa` tag `v2.0.0` (commit
  `90dbf07`). Any caller diverging from v2.0.0 consensus rules is out of
  scope.
