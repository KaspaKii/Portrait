# kcp scaffold ktt-token

Generates a KCC20-shape regulated-token state-machine demo: `mint → transfer → burn`.
Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--token-name` | `MyToken` | Token name embedded in generated comments |
| `--initial-supply` | `1000000` | Initial supply minted in the demo |
| `--out` | `./kii-covenants-out/my-ktt-token` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold ktt-token \
  --token-name MyToken --initial-supply 500000 \
  --workspace-path $PWD \
  --out /tmp/my-ktt-token
cd /tmp/my-ktt-token && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-ktt-token`; `publish = false`
- `src/main.rs` — mint → transfer → burn demo with `KttState` + `AuthContext`
- `tests/ktt_smoke.rs` — supply conservation (KTT-1) + auth rejection (KTT-3) tests
- `README.md` — pattern description + before-live-use callouts

## Before live use

Replace synthetic `owner_identifier` arrays with real Schnorr x-only pubkeys
from your wallet.
