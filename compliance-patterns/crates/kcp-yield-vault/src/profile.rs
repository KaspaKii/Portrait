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
/// Conversion formula — **virtual assets/shares** with `decimalsOffset = 0`
/// (the OpenZeppelin v5 ERC4626 mitigation), floor division throughout so
/// rounding favours the vault:
/// - `shares = assets × (total_shares + 1) / (total_assets + 1)`
/// - `assets = shares × (total_assets + 1) / (total_shares + 1)`
/// - First deposit (empty vault): `shares = assets` (1 : 1 initialisation)
///
/// The `+ 1` terms are a virtual share and a virtual asset held by the vault
/// itself. They **bound** the first-depositor inflation attack — a donation
/// can no longer round a later depositor's shares to zero — but they do not
/// eliminate it; see the crate docs.
///
/// # State relationship
///
/// A vault that has never been used has `total_assets == 0` and
/// `total_shares == 0`. After a full redeem, `total_shares` returns to 0 while
/// `total_assets` may retain floor-division dust, so the two are **not**
/// required to be zero together.
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

    /// Convert `assets` to shares at the current exchange rate, using the
    /// virtual share/asset offsets: `assets × (total_shares + 1) / (total_assets + 1)`.
    ///
    /// On an empty vault this is the identity (1 : 1 initialisation).
    /// # Saturation
    ///
    /// The quotient is computed in `u128` and **saturates** at [`u64::MAX`]
    /// rather than truncating. It can genuinely exceed `u64` — a vault with
    /// `total_assets == 0` and `total_shares > 0` (reachable by redeeming into
    /// floor-division dust) divides by the virtual asset alone. Truncating
    /// there would wrap and mint an arbitrary share count.
    pub fn convert_to_shares(&self, assets: u64) -> u64 {
        let numerator = (assets as u128).saturating_mul(self.total_shares as u128 + 1);
        u64::try_from(numerator / (self.total_assets as u128 + 1)).unwrap_or(u64::MAX)
    }

    /// Convert `shares` to assets at the current exchange rate, using the
    /// virtual share/asset offsets: `shares × (total_assets + 1) / (total_shares + 1)`.
    ///
    /// # When `total_shares == 0`
    ///
    /// No shares exist, so the divisor is the virtual share alone and the
    /// result is `shares × (total_assets + 1)` — an **extrapolation from the
    /// virtual offsets, not assets the vault can pay out**. It can exceed
    /// `total_assets` by an arbitrary factor. [`Self::redeem`] rejects
    /// `shares > total_shares`, so this figure is unreachable through the
    /// deposit/redeem flow; callers using this method (or
    /// [`Self::preview_redeem`]) directly must not treat it as redeemable
    /// value. This matches the OpenZeppelin v5 formula, which behaves the same
    /// way for a share supply that cannot exist. Like
    /// [`Self::convert_to_shares`], the result **saturates** at [`u64::MAX`]
    /// rather than truncating.
    pub fn convert_to_assets(&self, shares: u64) -> u64 {
        let numerator = (shares as u128).saturating_mul(self.total_assets as u128 + 1);
        u64::try_from(numerator / (self.total_shares as u128 + 1)).unwrap_or(u64::MAX)
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
    ///
    /// When `total_shares == 0` the returned figure is a virtual-offset
    /// extrapolation for a share supply that cannot exist, **not** redeemable
    /// value — see [`Self::convert_to_assets`].
    pub fn preview_redeem(&self, shares: u64) -> u64 {
        self.convert_to_assets(shares)
    }

    /// Deposit `assets` into the vault. Returns `(updated_profile, shares_minted)`.
    ///
    /// EVM equivalent: `ERC4626.deposit`.
    ///
    /// Returns `Err(VaultError::ZeroDeposit)` if `assets == 0`.
    ///
    /// Returns `Err(VaultError::ZeroSharesMinted)` if the deposit is too small
    /// to mint a whole share at the current rate — accepting it would add the
    /// assets to the vault and hand the depositor nothing back. The profile is
    /// unchanged on either error.
    pub fn deposit(&self, assets: u64) -> Result<(Self, u64)> {
        if assets == 0 {
            return Err(VaultError::ZeroDeposit);
        }
        let shares = self.convert_to_shares(assets);
        if shares == 0 {
            return Err(VaultError::ZeroSharesMinted);
        }
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
