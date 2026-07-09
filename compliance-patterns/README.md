# Covenant Patterns Library for Kaspa

> **v0 — pre-production, unaudited, testnet-first.**

The **covenant pattern library for Kaspa** — the OpenZeppelin-equivalent catalogue
of vetted, reusable covenant components (each with tests and a threat model).
Written in Rust and [SilverScript](https://github.com/kaspanet/silverscript),
targeting the Toccata hardfork (rusty-kaspa v2.0.0, mainnet activation at DAA
474,165,565, ~30 June 2026).

This is the **`compliance-patterns/`** component of the
[Portrait monorepo](https://github.com/kaspakii/portrait) (formerly the standalone
`kaspa-compliance-patterns` repo); it keeps its own Cargo workspace and builds
independently.

## What this is

Five compliance-oriented covenant patterns, one crate each, written as
*shape-aligned profiles* on top of SilverScript's covenant-declaration system
and the upstream KCC20 reference contracts:

| Crate | Pattern | Status (pre-production, unaudited, testnet) |
|---|---|---|
| `kcp-ktt-token` | KCC20-shape-aligned regulated-token profile (KTT) | v0 state model; state-continuity covenant engine-proven **and demonstrated live on testnet-10** |
| `kcp-vault` | Covenant-locked custody (timelock / multisig spending conditions) | v1 — consensus-enforced on testnet-10 (multisig, timelock, composite Any/All P2SH lock+spend) |
| `kcp-sealed-lineage` | Append-only sealed evidence lineage | v0 lineage; state-continuity covenant engine-proven **and demonstrated live on testnet-10** |
| `kcp-paired-attestation` | Two-party mutual attestation | v1 — consensus-enforced two-datasig (CSFS) on testnet-10; v0 off-chain mating retained |
| `kcp-transferable-record` | Transferable registry record with lineage continuity | v0 lineage; state-continuity covenant engine-proven **and demonstrated live on testnet-10** |
| `kcp-common` | Shared transaction / canonicalisation plumbing | P2SH covenant spend-path + offline real-engine preflight; live round-trip on testnet-10 |

The three state-continuity pattern covenants (ktt-token, sealed-lineage,
transferable-record) have each been deployed in live covenant-id-bound
transactions on testnet-10 and validated by the consensus covenant engine. This is
**testnet evidence of the mechanism on synthetic data** — not a mainnet deployment, not
audited, and testnet evidence is perishable. See [LAUNCH-NOTE.md](LAUNCH-NOTE.md) for
per-pattern detail, [docs/INDEX.md](docs/INDEX.md) for the documentation map, and
[KNOWN-ISSUES.md](KNOWN-ISSUES.md) for scope boundaries.

## Portrait — the covenant compiler & cross-layer catalogue

Beyond the five legacy crates above, the project ships **Portrait**: a high-level
surface language and Rust toolchain that compiles one source program down to **both**
Kaspa layers — a **SilverScript covenant** (`.sil`, the L1 spending policy) and, for
cross-layer patterns, a **RISC Zero vProg** (an off-L1 guest whose succinct STARK is
verified in-consensus via the KIP-16 `tag-0x21` precompile).

- **35 covenant patterns** compile through the pipeline (`engrave → silverc` exit 0);
  `DigitalReit.portrait` is the only multi-role source and emits 2 `.sil`. Backed by
  **349 passing** Portrait workspace tests (0 failed).
- **10 of the 35 are cross-layer (vProg) patterns** (a **subset** of the 35, not
  additional). **Five are settled live on testnet-10** — ProofOfReserves,
  ComplianceCredential (ZK-KYC), ConfidentialTransfer, BatchRollup, PrivateVoting:
  a real RISC Zero STARK over an in-zkVM predicate, verified in-consensus, each with
  per-pattern negative controls the live node rejected, plus the flagship
  `CsciInstrument` reference and a combined two-input cross-layer tx (`abc2d13f…`).
  The **other five are emit-verified only** (MerkleProofOfSolvency, PrivateOrderMatch,
  PrivateVickreyAuction, ZkAllowlistTransfer, ZkExecutionRollup) — they compile,
  engrave, and emit a RISC Zero guest, but are **not** settled live.
- **Independent second-compiler cross-check.** The experimental Kaspa Python SDK
  silverscript bindings — an independently maintained second compiler — emit
  **byte-for-byte identical** locking scripts to our `silverc` for every covenant
  cross-checked (0 divergences), and the same P2SH addresses. See
  [docs/SDK-CROSSCHECK.md](docs/SDK-CROSSCHECK.md) and the Rosetta-style
  [Python on-ramp](examples/python-onramp/) (offline scaffold / compile / build-spend).

**Honest residuals (the live vProgs):** the live covenant is the `tag-0x21` verifier
P2SH (image-id-pinned), *not yet* a SilverScript state machine the way
`CsciInstrument`'s seq/auth rules are; inputs are fixed sample data over small fixed
sets (not Merkle-rooted registries; no persistent nullifier set); commitments are
`sha256(value‖blinding)` not Pedersen; the audit key is a v1 symmetric pad; the SDK
bindings and Python on-ramp are experimental. Pre-production, unaudited, testnet-only,
perishable evidence. Full detail in [KNOWN-ISSUES.md](KNOWN-ISSUES.md).

## Honest maturity

- **Shape-aligned, not conformant.** The upstream KCC20 contracts are
  documented by their authors as examples, not a production token standard.
  These patterns align to those reference shapes and track upstream shape
  changes under a defined SLA (published with the launch pack); they do not
  claim conformance to a frozen standard, because none exists.
- **Unaudited.** No external security audit has been performed. Do not hold
  mainnet value with these patterns.
- **Testnet first.** Every pattern targets the Toccata testnet, and testnet
  transaction evidence is recorded per pattern before anything else is
  claimed about it.

## Disambiguation

KCC20, KRC-20, and KTT are three different things. KCC20 is the
covenant-native token example set in the SilverScript repository. KRC-20 is a
separate inscription-style token convention that originated with Kasplex. KTT
(Kaspa Trust Token) is the regulated-token profile in this library,
KCC20-shape-aligned. None of these is a synonym for another.

## Build

```sh
cargo build --workspace
cargo test --workspace
```

357 Rust tests pass across the workspace (default features, verified
2026-07-09); `cargo clippy --workspace --all-targets -- -D warnings`
clean; `cargo fmt --check` clean.
Rust 1.88+, edition 2021. The workspace pins
[`rusty-kaspa`](https://github.com/kaspanet/rusty-kaspa) tag **`v2.0.0`** —
the released Toccata engine — as its consensus reference.

## Build your first covenant in 10 minutes

**Option A — generate a project with the Kii Wizard CLI:**

```sh
# Generate a ready-to-run vault project (no network needed)
cargo run -p kcp -- scaffold vault --workspace-path /path/to/kaspa-compliance-patterns
cd ./kii-covenants-out/my-vault
cargo run   # "✓ PASSED — engine accepted the 2-of-2 multisig spend"

# Or a DAA-height timelock covenant:
cargo run -p kcp -- scaffold timelock --deadline 1000000 --workspace-path /path/to/kaspa-compliance-patterns

# Or a composite All([TimelockHeight, MultiSig]):
cargo run -p kcp -- scaffold composite --deadline 1000000 --threshold 2 --n 3 --workspace-path /path/to/kaspa-compliance-patterns
```

**Option B — run the library tests directly:**

The shortest path from a fresh checkout to a passing covenant test is the
**vault** crate (`kcp-vault`), the simplest consensus-enforced pattern in the
library.

```sh
# 1. Clone + build
git clone https://github.com/<stichting-kii-foundation>/kaspa-compliance-patterns.git  # URL finalised once Foundation org is created
cd kaspa-compliance-patterns
cargo build --workspace

# 2. Run the vault crate's tests — exercises the v1 P2SH consensus-enforced
#    lock+spend path (offline real-engine preflight; no live node needed)
cargo test -p kcp-vault

# 3. Read the vault README to see what just ran
$EDITOR crates/kcp-vault/README.md
```

What that does, end-to-end:

1. **Compiles a real Kaspa script** — a 2-of-2 multisig spending condition
   (or a timelock, or a composite Any/All; all live in `kcp-vault`).
2. **Locks value under it** by deriving the script's P2SH address.
3. **Spends it back** by constructing a satisfier (the witness bytes that
   the script accepts).
4. **Runs the spend through the real `rusty-kaspa` script engine offline**
   — `verify_p2sh_spend_offline` — so the engine itself signs off on the
   spend before any RPC.

If that test passes, you have run the same code path that produced
`[KCP-VT-002]` on testnet-10. To go further:

- `crates/kcp-vault/README.md` — variants (multisig, timelock-height,
  timelock-unix, composite Any/All, branch-selected P2SH).
- `crates/kcp-paired-attestation/README.md` — two-party datasig
  (consensus-enforced via `OP_CHECKSIGFROMSTACK`/CSFS, `[KCP-PA-002]`).
- `crates/kcp-sealed-lineage/README.md` — append-only state-continuity
  covenant (`validateOutputState` introspection, live `[KCP-SL-003]`).
- `crates/kcp-ktt-token/README.md` — KCC20-shape regulated-token profile
  (live `[KCP-KTT-003]`).
- `crates/kcp-transferable-record/README.md` — transferable registry
  record with lineage continuity (live `[KCP-TR-003]`).
- `crates/kcp-common/` — shared plumbing every crate above depends on
  (P2SH, KIP-14 payload, sighash, canonical serialization, offline
  engine preflight).

Live-network paths (real submission to testnet-10) require a synced
testnet-10 `kaspad` node + a funded wallet; see [docs/ENVIRONMENT.md](docs/ENVIRONMENT.md).

## Stewardship and licence

Stewarded by **Stichting Kii Foundation** — a Dutch non-profit foundation
(*stichting*), incorporated 2026-06-21; Stichting Ethereum Foundation
precedent. Licensed MIT — see [LICENSE](LICENSE).
Security disclosures: see [SECURITY.md](SECURITY.md). Contributions: see
[CONTRIBUTING.md](CONTRIBUTING.md). Release history: see [CHANGELOG.md](CHANGELOG.md).
