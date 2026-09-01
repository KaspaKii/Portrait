//! `kcp-yield-vault` — ERC4626-equivalent yield vault profile.
//!
//! EVM equivalent: `ERC4626` (EIP-4626 Tokenized Vault Standard).
//!
//! Provides shares/assets accounting for pooled-asset vaults on Kaspa. The
//! vault tracks a `total_assets` balance and a `total_shares` supply. As
//! yield accrues (increasing `total_assets` without minting shares), each
//! share becomes redeemable for more assets — the same mechanism as ERC4626.
//!
//! **Pre-production, unaudited, testnet-only.**
//!
//! # Differences from ERC4626
//!
//! - No ERC20 token for shares — the vault profile is a pure accounting
//!   primitive. Callers may represent shares using `kcp-ktt-token` if needed.
//! - Assets and shares are `u64` (sompi); no floating-point.
//! - No rounding mode parameter — uses floor division throughout (rounds in
//!   favour of the vault, not the depositor).
//!
//! # First-depositor inflation attack
//!
//! An attacker who is first into an empty vault can mint 1 share for 1 sompi,
//! donate a large amount directly to `total_assets` (here, via [`accrue`]), and
//! so inflate the price per share that a later depositor's shares round down —
//! in the worst case to zero, losing the whole deposit to the attacker.
//!
//! This crate uses the OpenZeppelin v5 mitigation: **virtual assets and shares**
//! with `decimalsOffset = 0`, i.e. conversions run over `total_shares + 1` and
//! `total_assets + 1`. That bounds the **rounding** loss — the attacker loses
//! more on the donation than they can redeem — and it does so without the
//! zero-guard the naive formula needed.
//!
//! On top of the formula, [`deposit`] **rejects** any deposit that would round
//! down to zero shares, returning [`ZeroSharesMinted`] and leaving the profile
//! unchanged. Without that guard a depositor could hand assets to the vault and
//! receive nothing at all — the residual-dust state left by a full redeem
//! (`total_shares == 0`, `total_assets > 0`) makes every deposit at or below the
//! dust amount round to zero. **No depositor can lose their whole deposit to
//! rounding.**
//!
//! **This bounds the attack; it does not eliminate it.** With an offset of 0 a
//! victim depositing into a freshly inflated vault still takes a real loss on
//! the shares they do get — the attacker simply cannot profit from it.
//! Deployments that need the attack closed must also mitigate operationally:
//! seed the vault with a non-trivial initial deposit that is never withdrawn,
//! or enforce a minimum initial deposit. This crate is **unaudited**.
//!
//! [`accrue`]: profile::YieldVaultProfile::accrue
//! [`deposit`]: profile::YieldVaultProfile::deposit
//! [`ZeroSharesMinted`]: error::VaultError::ZeroSharesMinted

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod profile;
