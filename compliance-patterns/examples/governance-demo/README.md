# Foundation Treasury Governance Cycle

**Reference implementation** showing `kcp-vault` + `kcp-governance` working
as a complete governance system on the Kaspa BlockDAG.

This is the `Governor` + `TimelockController` equivalent for Kaspa.
DAA heights serve as the on-chain clock — no globally-sequential block numbers
exist in Kaspa's DAG.

Pre-production, unaudited, testnet-only.

## What this demonstrates

```
[1] Treasury vault established   kcp-vault (2-of-3 multisig P2SH)
[2] Governance proposal raised   kcp-governance GovernanceProposal
[3] Committee votes              MultiSigVote (k-of-n quorum)
[4] Proposal passes              vote applied → Passed
[5] Timelock scheduled           TimelockAction (DAA delay)
[6] Action executed              GovernorState::execute
```

## Run it

```sh
cargo run --manifest-path examples/governance-demo/Cargo.toml
```

## Run the smoke tests

```sh
cargo test --manifest-path examples/governance-demo/Cargo.toml
```

## EVM pattern equivalence

| EVM pattern | kcp-governance equivalent |
|---|---|
| `Governor` | `GovernorState` |
| `GovernorVotes` | `MultiSigVote` (k-of-n; no token-weighted voting yet) |
| `TimelockController` | `TimelockAction` |

Note: token-weighted voting is deferred until KRC20 snapshots exist on Kaspa mainnet.

## Before live use

- Replace `[0xANu8; 32]` keys with real Schnorr x-only pubkeys from your wallet.
- Replace synthetic DAA heights with real heights from a live Kaspa node.
- Anchor `GovernorState` to a `kcp-sealed-lineage` lineage for on-chain continuity.
- `GovernorState` is a pure value type — persist it in your application state between calls.
- Verify Schnorr signatures before calling `MultiSigVote::approve()` — the vote tracker records approvals by key but does NOT verify cryptographic signatures. Verify with a Schnorr primitive before forwarding a key to `approve()`.
- **Post-quantum upgrade**: the vault authorisation can be secured with a RISC Zero ML-DSA-44 proof via `kcp-pq-anchor`. Replace the Schnorr multisig satisfier with a KIP-16 tag-0x21 spend; the governance committee's key set becomes the proof's `control_digests`. See `book/src/patterns/pq-anchor.md`.
