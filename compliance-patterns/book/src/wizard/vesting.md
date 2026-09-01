# kcp scaffold vesting

Generates a linear DAA-height vesting schedule demo (`VestingWallet` equivalent). Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--start` | `100000` | DAA height at which vesting begins |
| `--duration` | `86400` | Vesting duration in DAA units (~1 day at 1 BPS) |
| `--total-amount` | `1000000` | Total sompi to vest |
| `--out` | `./kii-covenants-out/my-vesting` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold vesting \
  --start 100000 --duration 86400 --total-amount 1000000 \
  --workspace-path $PWD \
  --out /tmp/my-vest
cd /tmp/my-vest && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-vesting`; `publish = false`
- `src/main.rs` — VestingSchedule with before-start, halfway, and full-vest release points
- `tests/vesting_smoke.rs` — nothing-before-start, fully-releasable-after-end, release-decrements-remaining
- `README.md` — before-live-use callouts

## Before live use

Replace `[0x01u8; 32]` with a real Schnorr x-only public key (the beneficiary).
Use real DAA heights from a connected `kaspad` node — Kaspa's blue score advances at
approximately 1 unit per second at 1 BPS target, but this varies.
Persist `VestingSchedule` after every `release()` call; the returned struct is the new state.
