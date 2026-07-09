use kcp_yield_vault::error::VaultError;
use kcp_yield_vault::profile::YieldVaultProfile;

#[test]
fn empty_vault_first_deposit_is_one_to_one() {
    let vault = YieldVaultProfile::new();
    let (v2, shares) = vault.deposit(1_000_000).unwrap();
    assert_eq!(shares, 1_000_000);
    assert_eq!(v2.total_assets, 1_000_000);
    assert_eq!(v2.total_shares, 1_000_000);
}

#[test]
fn second_deposit_at_one_to_one_rate() {
    let vault = YieldVaultProfile::new();
    let (v2, _) = vault.deposit(1_000_000).unwrap();
    let (v3, shares2) = v2.deposit(500_000).unwrap();
    assert_eq!(shares2, 500_000);
    assert_eq!(v3.total_assets, 1_500_000);
    assert_eq!(v3.total_shares, 1_500_000);
}

#[test]
fn yield_accrual_increases_assets_per_share() {
    let vault = YieldVaultProfile::new();
    let (v2, _) = vault.deposit(1_000_000).unwrap();
    // Accrue 100_000 yield → total_assets = 1_100_000, total_shares = 1_000_000
    let v3 = v2.accrue(100_000);
    // Second deposit: 1_000_000 assets → 1_000_000 * 1_000_000 / 1_100_000 = 909_090 shares
    let shares = v3.preview_deposit(1_000_000);
    assert_eq!(shares, 909_090);
}

#[test]
fn redeem_returns_proportional_assets() {
    let vault = YieldVaultProfile::new();
    let (v2, _) = vault.deposit(1_000_000).unwrap();
    let v3 = v2.accrue(1_000_000); // 2x yield — 1 share now worth 2 assets
    let assets_preview = v3.preview_redeem(500_000);
    assert_eq!(assets_preview, 1_000_000); // 500_000 shares * 2_000_000 / 1_000_000
    let (v4, assets) = v3.redeem(500_000).unwrap();
    assert_eq!(assets, 1_000_000);
    assert_eq!(v4.total_shares, 500_000);
    assert_eq!(v4.total_assets, 1_000_000);
}

#[test]
fn zero_deposit_rejected() {
    let vault = YieldVaultProfile::new();
    assert_eq!(vault.deposit(0).unwrap_err(), VaultError::ZeroDeposit);
}

#[test]
fn zero_redeem_rejected() {
    let vault = YieldVaultProfile::new();
    let (v2, _) = vault.deposit(1_000_000).unwrap();
    assert_eq!(v2.redeem(0).unwrap_err(), VaultError::ZeroWithdraw);
}

#[test]
fn redeem_more_than_supply_rejected() {
    let vault = YieldVaultProfile::new();
    let (v2, shares) = vault.deposit(1_000_000).unwrap();
    assert_eq!(
        v2.redeem(shares + 1).unwrap_err(),
        VaultError::InsufficientShares
    );
}

#[test]
fn serde_round_trip() {
    let vault = YieldVaultProfile {
        total_assets: 2_000_000,
        total_shares: 1_500_000,
    };
    let json = serde_json::to_string(&vault).unwrap();
    let back: YieldVaultProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(vault, back);
}

#[test]
fn convert_to_shares_empty_vault_is_identity() {
    let vault = YieldVaultProfile::new();
    assert_eq!(vault.convert_to_shares(12345), 12345);
}

#[test]
fn convert_to_assets_empty_vault_is_zero() {
    let vault = YieldVaultProfile::new();
    assert_eq!(vault.convert_to_assets(12345), 0);
}
