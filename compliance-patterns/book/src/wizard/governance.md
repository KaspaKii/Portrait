# kcp scaffold governance

Generates a k-of-n committee governance cycle demo: proposal → multisig vote → timelock → execute. Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--title` | `Fund auditor` | Proposal title |
| `--threshold` | `2` | Signatures required for quorum |
| `--n` | `3` | Total committee size |
| `--voting-window` | `1000` | DAA units for the voting window |
| `--timelock-delay` | `500` | DAA units between vote close and execution |
| `--out` | `./kii-covenants-out/my-governance` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold governance \
  --title "Approve auditor" \
  --threshold 2 --n 3 \
  --voting-window 1000 --timelock-delay 500 \
  --workspace-path $PWD \
  --out /tmp/my-gov
cd /tmp/my-gov && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-governance`; `publish = false`
- `src/main.rs` — committee key setup, multisig vote with quorum, proposal lifecycle, timelock schedule + execute
- `tests/governance_smoke.rs` — full cycle, quorum-required failure test
- `README.md` — before-live-use callouts

## Before live use

Replace synthetic `[0xNNu8; 32]` committee keys with real Schnorr x-only public keys.
Verify Schnorr signatures **before** calling `MultiSigVote::approve()` — the vote
tracker records approvals by key but does NOT verify cryptographic signatures.
Use real DAA heights from a connected `kaspad` node.
Persist `GovernorState` after each state transition.
