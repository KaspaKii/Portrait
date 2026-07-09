# Changelog

All notable changes to `kaspa-compliance-patterns` (the Covenant Patterns Library
for Kaspa) are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
with the understanding that **pre-1.0 releases may include breaking changes
between minor versions**.

> **Maturity stamp:** every release before v1.0 is **pre-production, unaudited,
> testnet-only.** On-chain evidence is **perishable by design** — testnets reset.
> Anchor identifiers (covenant_id, tx_id) cited in any release note refer to the
> testnet state at the time of writing and may not resolve on a later testnet.
> See `KNOWN-ISSUES.md` for the full caveats.

## [0.1.0] — initial public release

First public release of the **Covenant Patterns Library for Kaspa** — the
OpenZeppelin-equivalent catalogue of reusable, threat-modelled covenant
components — together with **Portrait**, the covenant language and toolchain that
builds them. MIT-licensed, stewarded by the **Stichting Kii Foundation** (a Dutch
non-profit). Pre-production, unaudited, testnet-only.

### Covenant Patterns Library — the Rust crates

- **`kcp-common`** — shared plumbing: `p2sh`, `digest`, `tx`, `wallet`,
  `canonical`, `wrpc`, `error`; offline real-engine preflight
  (`verify_p2sh_spend_offline`); the P2SH covenant spend-path; KIP-14 payload
  helpers; a live round-trip on testnet-10 `[KCP-P2SH-001]`. Also ships the
  reusable `access` (Ownable, Multisig, AccessControl), `security` (Pausable,
  TimelockController), and `cryptography` (tagged hashing, Merkle proofs)
  primitive modules.
- **`kcp-vault`** — covenant-locked custody. v0 (digest-anchor) plus
  **v1 consensus-enforced** multisig + timelock + composite Any/All
  branch-selected P2SH lock+spend `[KCP-VT-001, KCP-VT-002]`.
- **`kcp-ktt-token`** — KCC20-shape-aligned regulated-token profile. v0 4-field
  state machine (issue → transfer → burn) `[KCP-KTT-001]`; state-continuity
  covenant engine-proven `[KCP-KTT-002]` and deployed live on testnet-10
  `[KCP-KTT-003]`.
- **`kcp-sealed-lineage`** — append-only sealed evidence lineage. v0 lineage;
  state-continuity covenant engine-proven `[KCP-SL-002]` and live on testnet-10
  `[KCP-SL-003]`.
- **`kcp-transferable-record`** — transferable registry record with lineage
  continuity. v0 lineage; engine-proven `[KCP-TR-002]` and live on testnet-10
  `[KCP-TR-003]`.
- **`kcp-paired-attestation`** — two-party mutual attestation. v0 off-chain
  mating `[KCP-PA-001]`; **v1 consensus-enforced two-datasig** via
  `OP_CHECKSIGFROMSTACK` (CSFS) on testnet-10 `[KCP-PA-002]`.

Additional crates in the workspace: `kcp-governance`, `kcp-vesting`,
`kcp-yield-vault` (an ERC4626-shaped vault profile), `kcp-pq-anchor` (the
tag-0x21 verifier-script helpers), `kii-solidity-compat` (a Solidity-shaped
Rosetta facade), `kcp-csci`, and `kcp` — a scaffolding CLI that generates
ready-to-run covenant projects.

### Portrait — the covenant language + cross-layer catalogue

- **35 covenant sources** compile through the pipeline (`engrave → silverc`
  exit 0), spanning finance, custody, governance, attestation, and state
  patterns. `DigitalReit.portrait` is the only multi-role source (emits 2 `.sil`).
- **10 of the 35 are cross-layer (vProg) patterns.** **Five are settled live on
  testnet-10** — ProofOfReserves, ComplianceCredential (ZK-KYC),
  ConfidentialTransfer, BatchRollup, PrivateVoting — each a real RISC Zero STARK
  (`RISC0_DEV_MODE=0`) verified in-consensus via the KIP-16 `tag-0x21` precompile,
  each with a per-pattern negative control the live node rejected. **The other
  five are emit-verified only** (MerkleProofOfSolvency, PrivateOrderMatch,
  PrivateVickreyAuction, ZkAllowlistTransfer, ZkExecutionRollup): they compile,
  engrave, and emit a RISC Zero guest, but are **not** settled live.
- **`CsciInstrument`** — the reference covenant that self-enforces its state
  machine on-chain (committed-state auth, seq-monotonicity, covenant-id binding),
  settled live on testnet-10, including a combined two-input cross-layer
  transaction that binds the STARK journal to the engine per-instance
  covenant_id, with negative controls the live node rejected.
- **Two verification engines** ship with the compiler, each making a narrow,
  honestly-scoped claim. **Lens** is an SMT proof engine over the covenant
  *model*, discharging value-conservation / range / refinement / invariant /
  spend verification conditions via z3, fail-closed (a contradictory premise
  returns `UNKNOWN`, never a false `PROVED`). **Composer** is a session-type
  engine that checks several covenants wired together form a well-typed protocol.
  Both prove model-level properties — not the emitted script and not on-chain
  behaviour; `validate-translation` checks the model↔`.sil` correspondence
  structurally.

**Honest residuals (the live vProgs):** the live covenant is the `tag-0x21`
verifier P2SH (image-id-pinned), not yet a SilverScript state machine; inputs are
fixed sample data over small fixed sets (not Merkle-rooted registries; no
persistent nullifier set); commitments are `sha256(value‖blinding)` not Pedersen;
the audit key is a v1 symmetric pad. Full detail in `KNOWN-ISSUES.md`.

### Evidence, on the released Toccata engine (`rusty-kaspa` v2.0.0 = `90dbf07`)

- All on-chain evidence is **testnet-10** `[KCP-NET-001]`; Toccata covenant
  introspection is active there `[KCP-NET-002]`.
- **First live covenant-id-bound deployment** — an anchor-only
  reserve-attestation covenant performed a covenant genesis + append on
  testnet-10, accepted by the live consensus covenant engine (`validateOutputState`
  introspection + oracle `OP_CHECKSIG`) `[KCP-RE-003]`. The three state-continuity
  pattern covenants (sealed-lineage, transferable-record, ktt-token) share the
  covenant-id-bound shape and are each deployed live — `[KCP-SL-003]`,
  `[KCP-TR-003]`, `[KCP-KTT-003]`.
- An **auditor** example independently re-verifies a live lineage head from
  public information alone (`kcp-sealed-lineage/examples/auditor`).

### Tests + build hygiene

- **357 Rust tests pass** across the library workspace (default features,
  `cargo test --workspace`, verified 2026-07-09); the Portrait compiler workspace
  passes **349 tests**. `cargo clippy --workspace --all-targets -- -D warnings`
  clean; `cargo fmt --check` clean. Rust 1.88+, edition 2021.
- The workspace pins `rusty-kaspa` tag **`v2.0.0`** (= `90dbf07`), the released
  Toccata engine, as its consensus reference.
- `examples/hello-vault/` standalone project: `cargo run` exit 0; the real
  `rusty-kaspa v2.0.0` script engine accepts the synthetic 2-of-2 multisig P2SH
  spend offline (no node, no funds).

### Honest non-claims (what v0.1.0 does NOT promise)

- **Not audited** — pre-production, unaudited; external security audit gates v1.0.
- **Not mainnet** — testnet-10 only; testnet evidence is perishable.
- **Not a standard** — these are worked, testable building blocks. The KCC20
  shape (`kcp-ktt-token`) is documented upstream in `kaspanet/silverscript` as
  pre-standard, not a frozen standard.
- **Not a product** — a Foundation public good; no token, no investment claim.
- **Not a covenant-introspection guarantee on every Kaspa client** — patterns are
  written against the released Toccata engine (`rusty-kaspa` v2.0.0); any client
  diverging from v2.0.0 consensus rules is out of scope.

### Steward + licence

- **Steward:** Stichting Kii Foundation — a Dutch non-profit foundation
  (*stichting*), incorporated 2026-06-21 (Stichting Ethereum Foundation
  precedent). The Foundation publishes public goods; it does not bill, audit, or
  certify.
- **Licence:** MIT — see `LICENSE`. Security disclosures: `SECURITY.md`.

---

## Version history

- **v0.1.0** — first published release. Content documented above.

Versions before v0.1.0 were internal workspace iterations and are not itemised
here; the repository's development history lives in `git log`.
