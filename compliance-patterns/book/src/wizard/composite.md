# kcp scaffold composite

Generates a composite `All([TimelockHeight, MultiSig])` covenant project:
a DAA-height CLTV deadline AND k-of-n multisig must both be satisfied.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--deadline` | `1000000` | DAA height (height-based CLTV) |
| `--threshold` | `2` | Signatures required |
| `--n` | `3` | Total number of multisig keys |
| `--out` | `./kii-covenants-out/my-composite` | Output directory |
| `--workspace-path` | *(required)* | Path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold composite \
  --deadline 5000000 --threshold 2 --n 3 \
  --workspace-path $PWD \
  --out /tmp/my-composite
cd /tmp/my-composite && cargo test
```

## What is generated

- `Cargo.toml` — standalone crate with path deps on `kcp-vault` and `kcp-common`
- `src/main.rs` — build composite redeem, sign, verify offline
- `tests/composite_smoke.rs` — positive + deadline-not-met negative tests
- `README.md` — pattern description

Uses `compile_condition_p2sh` for the CLTV leaf. Satisfier ordering: multisig
sigs deepest in stack, controller sig on top. Replace `test_keypair(0xNN)`
with real keypairs before live use.
