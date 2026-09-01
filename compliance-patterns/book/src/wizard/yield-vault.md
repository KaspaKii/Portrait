# kcp scaffold yield-vault

Generates an ERC4626-equivalent shares/assets vault accounting demo. Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--initial-deposit` | `1000000` | Sompi deposited in the demo |
| `--yield-amount` | `100000` | Sompi accrued as yield |
| `--out` | `./kii-covenants-out/my-yield-vault` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold yield-vault \
  --initial-deposit 1000000 --yield-amount 100000 \
  --workspace-path $PWD \
  --out /tmp/my-yv
cd /tmp/my-yv && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-yield-vault`; `publish = false`
- `src/main.rs` — first deposit (1:1 init), yield accrual, second deposit at improved rate, redemption
- `tests/yield_vault_smoke.rs` — first-deposit-one-to-one, yield-increases-rate, redeem-returns-assets
- `README.md` — before-live-use callouts

## Before live use

**First deposit sets the exchange rate.** An attacker who deposits before the first
legitimate depositor can manipulate the share price. Consider seeding the vault with
a small initial deposit in your deployment flow.
Pair with `kcp-ktt-token` to represent shares as on-chain transferable tokens.
Lock underlying assets in a `kcp-vault` P2SH covenant to prevent unauthorised withdrawals.
