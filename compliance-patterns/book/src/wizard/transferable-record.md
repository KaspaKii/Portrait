# kcp scaffold transferable-record

Generates a transferable-record ownership-transfer chain demo. Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--record-type` | `MyRecord` | Record type label |
| `--out` | `./kii-covenants-out/my-transferable-record` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold transferable-record \
  --record-type "LandTitle" \
  --workspace-path $PWD \
  --out /tmp/my-tr
cd /tmp/my-tr && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-transferable-record`; `publish = false`
- `src/main.rs` — genesis + transfer + validate-chain demo
- `tests/record_smoke.rs` — TR-1..TR-3 invariant tests
- `README.md` — before-live-use callouts

## Before live use

Replace synthetic controller keys with real Schnorr x-only pubkeys.
