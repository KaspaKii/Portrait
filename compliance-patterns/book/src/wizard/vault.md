# kcp scaffold vault

Generates a P2SH multisig vault covenant project.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--threshold` | `2` | Signatures required (≤ number of keys) |
| `--keys` | `KEY1,KEY2` | Comma-separated key labels (display only in v0) |
| `--out` | `./kii-covenants-out/my-vault` | Output directory |
| `--workspace-path` | *(required)* | Path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold vault \
  --threshold 2 --keys KEY1,KEY2,KEY3 \
  --workspace-path $PWD \
  --out /tmp/my-vault
cd /tmp/my-vault && cargo test
```

## What is generated

- `Cargo.toml` — standalone crate with path dep on `kcp-vault`
- `src/main.rs` — build lock script, sign with both keys, verify offline
- `tests/vault_smoke.rs` — positive + threshold-not-met negative tests
- `README.md` — pattern description

Replace `test_keypair(0xNN)` with real Schnorr keypairs before live use.
