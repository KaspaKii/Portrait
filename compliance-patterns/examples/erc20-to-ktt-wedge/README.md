# ERC20 → KTT Wedge Demo

> **Pre-production, unaudited, testnet-only.**

Demonstrates the Kii Rosetta on-ramp: familiar ERC20-shaped API
(`kii-solidity-compat`) → KTT state transitions → carrier-anchored on TN10.

## What this shows

| ERC20 concept | This demo |
|---|---|
| `ERC20(name, symbol)` | `Token::new("HelloKTT", "HKT", 8, minter_key)` |
| `_mint(to, amount)` | `token.initial_mint(issuer_key, 1_000_000)` |
| `transfer(to, amount)` | `token.transfer(&holder_state, recipient_key, 400_000)` |
| `_burn(from, amount)` | `token.burn(&change_state, 100_000)` |
| `balanceOf` | *(absent — sum KTT UTXOs from kaspad)* |

## Prerequisites

1. A TN10 node (local or public) — set `KCP_NODE_URL`.
2. A wallet key file: 64-char hex private key or BIP-39 mnemonic on one line.
3. Fund the issuer address from the TN10 faucet (≥ 0.5 KAS for 3 anchor txs).

## Run

```sh
KCP_NODE_URL=ws://localhost:17210 \
KCP_KEY_FILE=/path/to/wallet.key \
cargo run --manifest-path examples/erc20-to-ktt-wedge/Cargo.toml
```

Optional: `KCP_NET_SUFFIX=10` (default).

## Verify a txid

```sh
# Independently fetch the transaction to confirm it was accepted:
curl -s "https://api.kaspa.org/transactions/<txid>" | jq '.is_accepted'
# → true
```

## What is NOT demonstrated here

- `approve` / `transferFrom` — no allowance object in the UTXO model.
  Express delegated spending as a covenant spend condition.
- `balanceOf` — query kaspad for KTT UTXOs held by an address.
- On-chain covenant binding — the UTXO output is plain pay-to-address (v0).
  KCC20 `validateOutputStateWithTemplate` is the next step.
