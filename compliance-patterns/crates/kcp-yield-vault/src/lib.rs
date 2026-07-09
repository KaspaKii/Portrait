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

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod profile;
