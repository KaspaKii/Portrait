# kcp scaffold sealed-lineage

Generates a sealed-lineage append-only evidence chain demo: `genesis → append → validate-chain`.
Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--subject` | `MyLineage` | Lineage subject label |
| `--out` | `./kii-covenants-out/my-sealed-lineage` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold sealed-lineage \
  --subject "MyKycLineage" \
  --workspace-path $PWD \
  --out /tmp/my-sl
cd /tmp/my-sl && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-sealed-lineage`; `publish = false`
- `src/main.rs` — GENESIS + APPEND + validate-chain demo
- `tests/lineage_smoke.rs` — L-1..L-4 invariant tests
- `README.md` — before-live-use callouts

## Before live use

Replace synthetic blinds with CSPRNG-derived bytes. `t_bucket` must come from a reliable clock.
