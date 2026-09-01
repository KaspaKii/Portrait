# kcp scaffold timelock

Generates a single-key P2SH DAA-height timelock covenant project.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--deadline` | `1000000` | DAA height at or after which the controller may spend |
| `--out` | `./kii-covenants-out/my-timelock` | Output directory |
| `--workspace-path` | *(required)* | Path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold timelock \
  --deadline 5000000 \
  --workspace-path $PWD \
  --out /tmp/my-timelock
cd /tmp/my-timelock && cargo test
```

## What is generated

- `Cargo.toml` — standalone crate with path dep on `kcp-vault`
- `src/main.rs` — build CLTV redeem, sign, verify offline
- `tests/timelock_smoke.rs` — positive + spend-before-deadline negative tests
- `README.md` — pattern description

Uses `compile_condition_p2sh` (no `OP_DROP` after CLTV) — correct for Kaspa's
`OP_CHECKLOCKTIMEVERIFY`, which pops the deadline from the stack.
Replace `test_keypair(0xA1)` with your real controller keypair before live use.
