# kcp-yield-vault

> **Pre-production, unaudited, testnet-only.**

ERC4626-equivalent yield vault profile for the Kaspa BlockDAG.

EVM equivalent: `ERC4626` (EIP-4626 Tokenized Vault Standard).

Tracks a `total_assets` / `total_shares` accounting pool. As yield accrues,
each share becomes redeemable for more assets — the same mechanism as ERC4626.

## Quick start

```rust
use kcp_yield_vault::profile::YieldVaultProfile;

let vault = YieldVaultProfile::new();

// First deposit: 1,000,000 sompi → 1,000,000 shares (1:1)
let (vault, shares) = vault.deposit(1_000_000)?;

// Yield accrues: 100,000 sompi added without minting shares
let vault = vault.accrue(100_000);

// Second depositor gets fewer shares (rate now 1.1 assets per share)
let (vault, shares2) = vault.deposit(500_000)?;

// Redemption
let (vault, assets_returned) = vault.redeem(shares)?;
```

## Differences from ERC4626

- **No share token** — `kcp-yield-vault` is a pure accounting primitive.
  Represent shares externally via `kcp-ktt-token` if on-chain share balances
  are required.
- **Floor division** — rounds in favour of the vault (same as ERC4626 v5
  default). No rounding mode parameter.
- **`u64` arithmetic** — assets and shares are sompi (`u64`). Maximum supply:
  ~29 billion KAS in sompi.

## Licence

MIT — Stichting Kii Foundation
