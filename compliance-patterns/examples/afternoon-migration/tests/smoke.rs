//! Smoke tests for the afternoon-migration composite demo.
//! Verifies the Solidity-pattern-shaped Kaspa facades compose correctly.

use kcp_vault::{condition::SpendCondition, evaluator::EvalContext};
use kii_solidity_compat::{Error, OwnershipRecord, TimelockController, Vault};

fn key(b: u8) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0] = b;
    k
}

const OWNER: [u8; 32] = [0x01u8; 32];
const PROPOSER: [u8; 32] = [0x02u8; 32];
const EXECUTOR: [u8; 32] = [0x03u8; 32];
const BENEFICIARY: [u8; 32] = [0x04u8; 32];

// ── Ownable ──────────────────────────────────────────────────────────────────

#[test]
fn ownable_verify_owner_passes() {
    let rec = OwnershipRecord::new(OWNER);
    assert!(rec.verify_owner(OWNER).is_ok());
}

#[test]
fn ownable_verify_other_fails() {
    let rec = OwnershipRecord::new(OWNER);
    assert!(matches!(rec.verify_owner(key(0xff)), Err(Error::NotOwner)));
}

#[test]
fn ownable_transfer_and_renounce() {
    let new_owner = key(0x20);
    let rec = OwnershipRecord::new(OWNER)
        .transfer_ownership(new_owner)
        .renounce_ownership();
    assert!(rec.is_renounced());
}

// ── TimelockController ───────────────────────────────────────────────────────

#[test]
fn timelock_happy_path() {
    let mut ctrl = TimelockController::new(1_000, PROPOSER, EXECUTOR).unwrap();
    ctrl.schedule(PROPOSER, 500_000).unwrap();
    assert!(!ctrl.is_ready(500_999));
    assert!(ctrl.is_ready(501_000));
    assert!(ctrl.execute(EXECUTOR, 501_000).is_ok());
}

#[test]
fn timelock_cancel_blocks_schedule() {
    let mut ctrl = TimelockController::new(100, PROPOSER, EXECUTOR).unwrap();
    ctrl.cancel();
    assert!(matches!(ctrl.schedule(PROPOSER, 1), Err(Error::AlreadyCancelled)));
}

// ── Vault ─────────────────────────────────────────────────────────────────────

#[test]
fn vault_timelock_evaluate() {
    let deadline: u64 = 2_000_000;
    let vault = Vault::new(SpendCondition::TimelockHeight {
        deadline,
        controller_xonly: BENEFICIARY,
    })
    .unwrap();
    let before = EvalContext { daa_score: deadline - 1, unix_seconds: 0, signers_present: vec![] };
    let at = EvalContext { daa_score: deadline, unix_seconds: 0, signers_present: vec![] };
    assert!(!vault.evaluate(&before));
    assert!(vault.evaluate(&at));
}

#[test]
fn vault_deposit_returns_descriptor() {
    let vault = Vault::new(SpendCondition::TimelockHeight {
        deadline: 1_000_000,
        controller_xonly: BENEFICIARY,
    })
    .unwrap();
    let desc = vault.deposit(1_000_000_000).unwrap();
    assert_eq!(desc.amount_sompi, 1_000_000_000);
}

// ── Composite: all three together ─────────────────────────────────────────────

#[test]
fn composite_owner_governs_vault_creation() {
    // Step 1: ownership check
    let rec = OwnershipRecord::new(OWNER);
    rec.verify_owner(OWNER).unwrap(); // gate passed

    // Step 2: governance delay
    let mut ctrl = TimelockController::new(1_000, PROPOSER, EXECUTOR).unwrap();
    ctrl.schedule(PROPOSER, 100_000).unwrap();
    assert!(ctrl.execute(EXECUTOR, 101_000).is_ok());

    // Step 3: vault created after governance pass
    let vault = Vault::new(SpendCondition::TimelockHeight {
        deadline: 111_000,
        controller_xonly: BENEFICIARY,
    })
    .unwrap();
    let desc = vault.deposit(500_000_000).unwrap();
    assert_eq!(desc.amount_sompi, 500_000_000);

    // Vault not yet spendable at governance pass time
    let ctx_early = EvalContext { daa_score: 101_000, unix_seconds: 0, signers_present: vec![] };
    assert!(!vault.evaluate(&ctx_early));

    // Vault spendable at deadline
    let ctx_ready = EvalContext { daa_score: 111_000, unix_seconds: 0, signers_present: vec![] };
    assert!(vault.evaluate(&ctx_ready));
}
