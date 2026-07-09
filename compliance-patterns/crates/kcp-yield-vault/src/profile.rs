//! Yield vault profile — shares/assets accounting.

use super::error::{Result, VaultError};
use serde::{Deserialize, Serialize};

/// Shares/assets accounting for a yield-bearing vault.
///
/// EVM equivalent: `ERC4626` (EIP-4626 Tokenized Vault Standard)
/// — pre-production, unaudited.
///
/// # Accounting model
///
/// - `total_assets` — total assets under custody (sompi). Increases on deposit
///   and when yield is reported via [`accrue`]. Decreases on withdrawal.
/// - `total_shares` — total shares in circulation. Increases on deposit.
///   Decreases on withdrawal.
///
/// Conversion formula (floor division — rounds in favour of the vault):
/// - `shares = assets × total_shares / total_assets` (when shares exist)
/// - `assets = shares × total_assets / total_shares`
/// - First deposit (no shares yet): `shares = assets` (1 : 1 initialisation)
///
/// # Invariant
///
/// `total_assets == 0` iff `total_shares == 0`. The vault is either empty
/// (both zero) or has both assets and shares.
///
/// # Pure value type
///
/// All operations return a new `YieldVaultProfile`. Callers are responsible
/// for persisting the updated state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YieldVaultProfile {
    /// Total assets under custody (sompi).
    pub total_assets: u64,
    /// Total shares in circulation.
    pub total_shares: u64,
}

impl Default for YieldVaultProfile {
    /// Returns an empty vault (no assets, no shares).
    fn default() -> Self {
        Self {
            total_assets: 0,
            total_shares: 0,
        }
    }
}

impl YieldVaultProfile {
    /// Create a new empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert `assets` to shares at the current exchange rate.
    ///
    /// Returns 0 if `total_assets == 0` (empty vault, no rate yet).
    /// Use [`deposit`] for the full deposit flow, which handles initialisation.
    pub fn convert_to_shares(&self, assets: u64) -> u64 {
        if self.total_assets == 0 || self.total_shares == 0 {
            return assets; // 1:1 for first deposit
        }
        (assets as u128)
            .saturating_mul(self.total_shares as u128)
            .checked_div(self.total_assets as u128)
            .unwrap_or(0) as u64
    }

    /// Convert `shares` to assets at the current exchange rate.
    ///
    /// Returns 0 if `total_shares == 0` (empty vault).
    pub fn convert_to_assets(&self, shares: u64) -> u64 {
        if self.total_shares == 0 {
            return 0;
        }
        (shares as u128)
            .saturating_mul(self.total_assets as u128)
            .checked_div(self.total_shares as u128)
            .unwrap_or(0) as u64
    }

    /// Preview how many shares a deposit of `assets` would mint at the
    /// current exchange rate. Does not mutate state.
    ///
    /// EVM equivalent: `ERC4626.previewDeposit`.
    pub fn preview_deposit(&self, assets: u64) -> u64 {
        self.convert_to_shares(assets)
    }

    /// Preview how many assets redeeming `shares` would return at the
    /// current exchange rate. Does not mutate state.
    ///
    /// EVM equivalent: `ERC4626.previewRedeem`.
    pub fn preview_redeem(&self, shares: u64) -> u64 {
        self.convert_to_assets(shares)
    }

    /// Deposit `assets` into the vault. Returns `(updated_profile, shares_minted)`.
    ///
    /// EVM equivalent: `ERC4626.deposit`.
    ///
    /// Returns `Err(VaultError::ZeroDeposit)` if `assets == 0`.
    pub fn deposit(&self, assets: u64) -> Result<(Self, u64)> {
        if assets == 0 {
            return Err(VaultError::ZeroDeposit);
        }
        let shares = self.convert_to_shares(assets);
        let updated = Self {
            total_assets: self.total_assets.saturating_add(assets),
            total_shares: self.total_shares.saturating_add(shares),
        };
        Ok((updated, shares))
    }

    /// Redeem `shares` from the vault. Returns `(updated_profile, assets_returned)`.
    ///
    /// EVM equivalent: `ERC4626.redeem`.
    ///
    /// Returns `Err(VaultError::ZeroWithdraw)` if `shares == 0`.
    /// Returns `Err(VaultError::InsufficientShares)` if `shares > total_shares`.
    pub fn redeem(&self, shares: u64) -> Result<(Self, u64)> {
        if shares == 0 {
            return Err(VaultError::ZeroWithdraw);
        }
        if shares > self.total_shares {
            return Err(VaultError::InsufficientShares);
        }
        let assets = self.convert_to_assets(shares);
        let updated = Self {
            total_assets: self.total_assets.saturating_sub(assets),
            total_shares: self.total_shares.saturating_sub(shares),
        };
        Ok((updated, assets))
    }

    /// Report yield accrual: increase `total_assets` by `yield_amount` without
    /// minting new shares. This increases the assets-per-share exchange rate.
    ///
    /// Callers must verify the authorising key has the right to report yield
    /// (e.g. the vault manager) before calling this method.
    pub fn accrue(&self, yield_amount: u64) -> Self {
        Self {
            total_assets: self.total_assets.saturating_add(yield_amount),
            total_shares: self.total_shares,
        }
    }
}
