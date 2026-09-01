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
    // Second deposit: 1_000_000 assets → 1_000_000 * 1_000_001 / 1_100_001 = 909_090 shares
    let shares = v3.preview_deposit(1_000_000);
    assert_eq!(shares, 909_090);
}

#[test]
fn redeem_returns_proportional_assets() {
    let vault = YieldVaultProfile::new();
    let (v2, _) = vault.deposit(1_000_000).unwrap();
    let v3 = v2.accrue(1_000_000); // 2x yield — 1 share now worth 2 assets
    let assets_preview = v3.preview_redeem(500_000);
    assert_eq!(assets_preview, 999_999); // 500_000 shares * 2_000_001 / 1_000_001
    let (v4, assets) = v3.redeem(500_000).unwrap();
    assert_eq!(assets, 999_999);
    assert_eq!(v4.total_shares, 500_000);
    assert_eq!(v4.total_assets, 1_000_001);
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
fn convert_to_assets_empty_vault_returns_virtual_identity() {
    // With virtual assets/shares the empty vault converts 1:1 rather than to 0.
    // `redeem()` still rejects `shares > total_shares`, so this conversion is
    // unreachable from the deposit/redeem flow — the same property as the
    // OpenZeppelin v5 implementation.
    let vault = YieldVaultProfile::new();
    assert_eq!(vault.convert_to_assets(12345), 12345);
}

#[test]
fn first_depositor_inflation_attack_is_bounded() {
    // Attacker seeds the vault with 1 sompi, then donates 1_000_000 via accrue.
    let vault = YieldVaultProfile::new();
    let (v2, attacker_shares) = vault.deposit(1).unwrap();
    let v3 = v2.accrue(1_000_000);

    // Under the pre-mitigation formula the victim would have minted 0 shares
    // (1_000_000 * 1 / 1_000_001 = 0) and lost the whole deposit.
    let (v4, victim_shares) = v3.deposit(1_000_000).unwrap();
    assert!(victim_shares > 0);

    // The attacker cannot profit: redeeming their share returns less than the
    // 1_000_001 sompi they put in. The attack is BOUNDED, not eliminated —
    // the victim still takes a haircut.
    let attacker_outlay = 1 + 1_000_000;
    let attacker_redemption = v4.preview_redeem(attacker_shares);
    assert!(attacker_redemption < attacker_outlay);
}

/// Drives the vault to the residual-dust state (`total_shares == 0`,
/// `total_assets > 0`) that a full redeem can leave behind, and pins the
/// behaviour there: a deposit too small to mint a share is rejected, not
/// silently absorbed.
fn dust_state() -> YieldVaultProfile {
    let (v2, attacker_shares) = YieldVaultProfile::new().deposit(1).unwrap();
    let v3 = v2.accrue(1_000_000);
    let (v4, victim_shares) = v3.deposit(1_000_000).unwrap();
    let (v5, _) = v4.redeem(victim_shares).unwrap();
    let (v6, _) = v5.redeem(attacker_shares).unwrap();
    v6
}

#[test]
fn deposit_that_would_mint_zero_shares_is_rejected() {
    let dust = dust_state();
    assert_eq!(dust.total_assets, 666_667);
    assert_eq!(dust.total_shares, 0);

    // 1_000 * (0 + 1) / (666_667 + 1) = 0 shares — the depositor would receive
    // nothing while the vault kept the assets.
    assert_eq!(dust.preview_deposit(1_000), 0);
    assert_eq!(
        dust.deposit(1_000).unwrap_err(),
        VaultError::ZeroSharesMinted
    );
    assert_eq!(dust.total_assets, 666_667);
    assert_eq!(dust.total_shares, 0);
}

#[test]
fn deposit_above_the_dust_threshold_succeeds() {
    let dust = dust_state();
    let (v2, shares) = dust.deposit(666_668).unwrap();
    assert_eq!(shares, 1);
    assert_eq!(v2.total_assets, 1_333_335);
    assert_eq!(v2.total_shares, 1);
}

/// `total_assets == 0` with `total_shares > 0` is reachable, and there the
/// share conversion divides by the virtual asset alone — the numerator can
/// exceed `u64`. It must saturate, never wrap into a small share count.
#[test]
fn conversion_saturates_instead_of_wrapping_when_assets_are_zero() {
    let empty_of_assets = YieldVaultProfile {
        total_assets: 0,
        total_shares: u64::MAX,
    };

    // assets × (u64::MAX + 1) / 1 — far beyond u64. A truncating `as u64`
    // returns 0 here (the low 64 bits), which would mint nothing for a real
    // deposit and silently corrupt the accounting.
    assert_eq!(empty_of_assets.convert_to_shares(2), u64::MAX);
    assert_eq!(empty_of_assets.preview_deposit(u64::MAX), u64::MAX);
}

/// The mirror case for the assets direction: no share supply, a large balance.
#[test]
fn asset_conversion_saturates_instead_of_wrapping() {
    let no_shares = YieldVaultProfile {
        total_assets: u64::MAX,
        total_shares: 0,
    };
    assert_eq!(no_shares.convert_to_assets(2), u64::MAX);
}
