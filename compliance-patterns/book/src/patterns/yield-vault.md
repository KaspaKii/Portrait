# kcp-yield-vault

> **Pre-production, unaudited, testnet-only.**

`kcp-yield-vault` provides shares/assets accounting for pooled-asset vaults on
the Kaspa BlockDAG — the ERC4626 (Tokenized Vault Standard) equivalent.

The vault tracks a `total_assets` balance and a `total_shares` supply. As yield
accrues (increasing `total_assets` without minting new shares), each share
becomes redeemable for more assets. This is the same mechanism as ERC4626.

## Constructing a yield vault

```rust
use kcp_yield_vault::profile::YieldVaultProfile;

// Create an empty vault
let vault = YieldVaultProfile::new();

// First deposit: 1:1 initialisation (no rate yet)
let (vault, shares_a) = vault.deposit(1_000_000)?;
// vault: total_assets=1_000_000, total_shares=1_000_000

// Yield accrues — increases assets per share
let vault = vault.accrue(100_000);
// vault: total_assets=1_100_000, total_shares=1_000_000
// Rate: 1 share = 1.1 assets

// Second depositor gets fewer shares (rate now 1.1 assets/share)
let shares_b = vault.preview_deposit(550_000); // 500_000 shares
let (vault, shares_b) = vault.deposit(550_000)?;

// Redemption: shares → assets at current rate
let (vault, assets_out) = vault.redeem(shares_a)?;
```

## A note on rounding: floor division

`kcp-yield-vault` uses floor (truncating) integer division throughout — rounding
in favour of the vault, not the depositor. This matches ERC4626 v5's default.

**It is critical** that callers use `preview_deposit` / `preview_redeem` to
check the expected output before executing, and confirm the returned amount
matches expectations before persisting state.

## A note on share token representation

`kcp-yield-vault` is a pure accounting primitive — it tracks totals but does
not issue an on-chain share token. Represent shares externally using
`kcp-ktt-token` if on-chain, transferable share balances are required. The
vault's `total_shares` must always equal the sum of all outstanding share balances.

## Extensions

- **Yield source** — call `accrue(yield_amount)` when yield is credited from an
  external source (e.g. staking rewards, lending interest).
- **Governed yield vault** — gate `accrue()` behind a `kcp-governance` vote so
  the committee controls yield reporting.
- **Share token** — pair with `kcp-ktt-token` to give each share holder an
  on-chain, transferable claim. `total_shares` must equal `kcp-ktt-token`
  total supply.
- **Custody** — lock the underlying assets in a `kcp-vault` P2SH covenant;
  the yield vault profile tracks the accounting while the vault covenant
  enforces custody.

→ API reference: [`YieldVaultProfile`], [`deposit`], [`redeem`], [`accrue`], [`convert_to_shares`], [`convert_to_assets`], [`preview_deposit`], [`preview_redeem`]
