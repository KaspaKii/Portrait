# kii-solidity-compat

> **Pre-production, unaudited, testnet-only.**

Solidity-pattern-shaped facade over `kcp-ktt-token`. Lets an Ethereum developer
read KTT token operations in familiar ERC20 vocabulary.

## Quick start

```rust
use kii_solidity_compat::erc20::Token;

let token = Token::new("My Token", "MYT", 8, minter_key);

// ERC20 _mint → initial_mint
let (alice_state, minter_state) = token.initial_mint(alice_key, 1_000_000)?;

// ERC20 transfer → token.transfer (takes the UTXO, not the address)
let (to_bob, change) = token.transfer(&alice_state, bob_key, 300_000)?;

// ERC20 _burn → token.burn
let remaining = token.burn(&alice_state, 100_000)?;
```

## What is different from ERC20

| ERC20 | Here | Reason |
|---|---|---|
| `balanceOf(addr)` | No direct equivalent | Sum KTT UTXOs from the node |
| `approve` / `allowance` / `transferFrom` | Not present | Model via covenant spend condition |
| `totalSupply` | No direct equivalent | Sum all KTT UTXOs on-chain |

## Crate scope

This crate handles **state-machine validation only** — it does not broadcast
transactions. To deploy on TN10, encode the resulting `KttState` values into a
`kcp-ktt-token` transaction and submit via the `wrpc` feature.
