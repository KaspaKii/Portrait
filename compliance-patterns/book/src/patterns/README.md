# Patterns

`kaspa-compliance-patterns` provides one Rust crate per pattern. Each crate is
independently usable and can be composed with others.

## Which pattern?

| I need to… | Use | Crate |
|---|---|---|
| Lock assets with multi-party or time conditions | **Vault** | `kcp-vault` |
| Have two parties mutually commit to a statement | **Paired Attestation** | `kcp-paired-attestation` |
| Build a tamper-evident append-only audit log | **Sealed Lineage** | `kcp-sealed-lineage` |
| Transfer ownership of a unique record with provenance | **Transferable Record** | `kcp-transferable-record` |
| Issue a regulated token with supply conservation + minter guard | **KTT Token** | `kcp-ktt-token` |
| Run a proposal → vote → timelock → execute governance cycle | **Governance** | `kcp-governance` |
| Time-based linear token release to a beneficiary | **Vesting** | `kcp-vesting` |
| Pooled-asset vault with yield accrual and share accounting | **Yield Vault** | `kcp-yield-vault` |
| Verify Merkle inclusion proof (sorted-pair SHA-256) | **MerkleProof** | `kcp-common::cryptography` |
| Anchor a credential with a post-quantum signature | **PQ Anchor** | `kcp-pq-anchor` |

### Composing patterns

Patterns are designed to compose. If you need multiple properties, chain them:

- **Compliance credential lifecycle** — issue a bilateral attestation (`kcp-paired-attestation`) → seal into an evidence log (`kcp-sealed-lineage`) → transfer the record (`kcp-transferable-record`) → represent as a regulated token (`kcp-ktt-token`). See `examples/compliance-workflow`.

- **Governed treasury** — lock assets in a vault (`kcp-vault`) → raise a governance proposal and vote (`kcp-governance`). See `examples/governance-demo`.

- **Vault + governance** — the vault's multisig key set can be the same as the governance committee's signatory set, so a governance vote directly authorises a spend.

## EVM pattern equivalence

| EVM pattern | Kaspa equivalent | Notes |
|---|---|---|
| `Ownable` | `kcp-common::access::Ownable` | Identical shape |
| `Ownable2Step` | `kcp-common::access::Ownable2Step` | Two-step handshake |
| `AccessControl` | `kcp-common::access::AccessControl` | Role-based, x-only keys instead of addresses |
| `Pausable` | `kcp-common::security::Pausable` | Pure value type — no modifier semantics |
| `TimelockController` | `kcp-governance::TimelockAction` | DAA heights as clock |
| `Governor` | `kcp-governance::GovernorState` | k-of-n, not token-weighted |
| `ERC20` | `kcp-ktt-token` | KCC20 shape — supply conservation + minter guard |
| `ERC721` | `kcp-transferable-record` | Unique record with ownership provenance |
| `VestingWallet` | `kcp-vesting::VestingSchedule` | Linear, DAA-height clock |
| `MerkleProof` | `kcp-common::cryptography::merkle_verify` | Sorted-pair SHA-256 |
| `ECDSA` | `kcp-common::cryptography::sign_schnorr` | Schnorr (not ECDSA — secp256k1 Schnorr) |
| `ReentrancyGuard` | **N/A** | UTXO model is structurally non-reentrant |
| `TransparentUpgradeableProxy` | **N/A** | `kcp-sealed-lineage` IS the upgrade primitive |
| `PaymentSplitter` | **N/A** | UTXO model: construct multiple outputs natively |
| `ERC4626` | `kcp-yield-vault::YieldVaultProfile` | Shares/assets accounting, floor division |

## Pattern maturity

| Pattern | v0 (off-chain invariants) | v1 (on-chain enforcement) | Live TN10 evidence |
|---|---|---|---|
| Vault | ✓ | ✓ | ✓ `[KCP-VT-002]` |
| Paired Attestation | ✓ | ✓ CSFS | ✓ `[KCP-PA-002]` |
| Sealed Lineage | ✓ | ✓ covenant | ✓ `[KCP-SL-003]` |
| Transferable Record | ✓ | ✓ covenant | ✓ `[KCP-TR-003]` |
| KTT Token | ✓ | engine-proven | ✓ `[KCP-KTT-003]` |
| Governance | ✓ | deferred | — |
| Vesting | ✓ | deferred | — |
| Yield Vault | ✓ | deferred | — |
| PQ Anchor | — | script assembly | — |

See the [evidence index](../evidence.md) for transaction IDs.
