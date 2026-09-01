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

## First-depositor inflation attack

An attacker who is first into an empty vault can mint 1 share for 1 sompi,
donate a large amount straight into `total_assets` (here via `accrue`), and so
inflate the price per share that the next depositor's shares round down — in the
worst case to zero, losing the whole deposit.

`kcp-yield-vault` applies the OpenZeppelin v5 mitigation: **virtual assets and
shares** with `decimalsOffset = 0`. Conversions run over `total_shares + 1` and
`total_assets + 1`:

```text
shares = assets × (total_shares + 1) / (total_assets + 1)
assets = shares × (total_assets + 1) / (total_shares + 1)
```

On top of the formula, **`deposit()` rejects any deposit that would round down
to zero shares** (`VaultError::ZeroSharesMinted`), leaving the profile
unchanged. This matters because a full redeem can leave residual dust
(`total_shares == 0`, `total_assets > 0`), and in that state every deposit at or
below the dust amount rounds to zero — without the guard those assets would join
the pool and the depositor would get nothing. No depositor can lose a whole
deposit to rounding.

**This bounds the attack; it does not eliminate it.** A depositor entering a
freshly inflated vault still takes a real loss on the shares they do get — they
can no longer be rounded down to zero, and the attacker cannot profit (they lose
more on the donation than they can redeem), but the victim is not made whole.
Deployments that need the attack closed must mitigate on the deployment side
too: seed the vault with a non-trivial initial deposit that is never withdrawn,
or enforce a minimum initial deposit.

This crate is **unaudited**. See `tests/smoke.rs::first_depositor_inflation_attack_is_bounded`
and `::deposit_that_would_mint_zero_shares_is_rejected`.

### Previews when no shares exist

With `total_shares == 0` the divisor in `convert_to_assets` is the virtual share
alone, so `convert_to_assets` / `preview_redeem` return
`shares × (total_assets + 1)` — an extrapolation from the virtual offsets that
can far exceed everything the vault holds. `redeem()` rejects
`shares > total_shares`, so this is unreachable through the deposit/redeem flow,
but **callers must not read those two methods as redeemable value in that
state**. The formula is left faithful to OpenZeppelin v5 rather than special-cased.

## Threat model

> **Pre-production, unaudited, testnet-only.** This section distils what the
> crate documents. It is **not a security audit** and not an assurance that the
> properties below hold.

**Assets** — the pooled assets the accounting stands for, and each depositor's
claim on them through shares.

**Attacker capabilities (assumed)** — the first depositor into an empty (or
dust-only) vault, who mints 1 share for 1 sompi and then donates straight into
`total_assets` (here via `accrue`) to inflate the price per share before the
next depositor arrives. More generally: whoever can call `accrue` moves the
exchange rate at will, and whoever reads a preview API can be misled in states
where it does not mean what it looks like. On a UTXO chain the spender is
normally the adversary — but no UTXO is involved here, so the adversary is the
caller.

**What consensus enforces** — **nothing.** Like `kcp-vesting`, this is a pure
`u64` accounting value type: no script, no covenant, no UTXO, no share token,
no access control. It holds no funds and authorises no one; a "deposit" is an
entry in a struct the caller persists.

**What this assumes / trusts off-chain** — that the caller holds the real assets
and keeps them consistent with `total_assets`; that whoever calls `accrue` is
reporting real yield rather than staging a donation aimed at a victim; that the
caller enforces who may deposit or redeem, since the profile has no owner and no
authorisation check of its own.

**Known limits and non-goals** — the first-depositor inflation attack is
**bounded, not eliminated**. Virtual assets and shares (`decimalsOffset = 0`)
plus `VaultError::ZeroSharesMinted` mean no depositor can lose a whole deposit
to rounding, and the attacker cannot profit; a depositor entering a freshly
inflated vault still takes a real loss and is not made whole. Closing the gap
needs a deployment-side mitigation: seed the vault with a non-trivial initial
deposit that is never withdrawn, or enforce a minimum initial deposit. With
`total_shares == 0`, `convert_to_assets` / `preview_redeem` return a
virtual-offset extrapolation (`shares × (total_assets + 1)`) that can far exceed
everything the vault holds — **not** payable value; `redeem()` rejects
`shares > total_shares` so the deposit/redeem flow cannot reach it, but a caller
reading those methods directly can. Floor division always rounds in the vault's
favour, and a full redeem can leave dust (`total_shares == 0`,
`total_assets > 0`).

**There is no loss path.** `accrue` only adds; nothing marks the pool *down*. A
vault whose underlying loses value keeps quoting the pre-loss exchange rate and
pays early redeemers in full until the assets run out, so the loss lands
entirely on whoever redeems last — first-mover-wins, with no circuit breaker. A
deployment that can lose value needs a mark-down step this profile does not
provide.

Both converters compute in `u128` and **saturate** at `u64::MAX` rather than
truncating (a `total_assets == 0, total_shares > 0` vault is reachable by
redeeming into dust, and there the share conversion divides by the virtual asset
alone and can exceed `u64`); saturation is a guard against a silent wrap, not a
meaningful share count. No fee model, no withdrawal queue, no pause, no yield
source. Unaudited; not for mainnet value.

## Licence

MIT — Stichting Kii Foundation
