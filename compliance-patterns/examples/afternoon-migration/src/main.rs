//! "I migrated from Ethereum in an afternoon" — composite Kii Rosetta demo.
//!
//! Composes three Solidity-pattern-shaped Kaspa facades in a single workflow:
//!
//!   1. **`Ownable`** — single-key authority over who can create the vault.
//!   2. **`TimelockController`** — queue + delay before the vault is activated.
//!   3. **`Vault`** — covenant-enforced custody of value under a spend condition.
//!
//! This models a common Ethereum pattern — an owner queues a vault creation,
//! waits for the governance delay, then activates the vault — using familiar
//! API names over Kaspa's UTXO covenant model.
//!
//! **Replace synthetic keys and heights with real values before live use.**
//! **Pre-production, unaudited, testnet-only.**

use kcp_vault::{
    condition::SpendCondition,
    evaluator::{evaluate, EvalContext},
};
use kii_solidity_compat::{OwnershipRecord, TimelockController, Vault};

fn key(b: u8) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0] = b;
    k
}

fn main() {
    println!("=== Afternoon Migration — Ownable + TimelockController + Vault ===\n");

    // ── Keys (replace with real 32-byte x-only Schnorr public keys) ──────────
    let owner_key    = key(0x01); // the admin / deployer
    let proposer_key = key(0x02); // the governance proposer (often = owner)
    let executor_key = key(0x03); // the governance executor
    let beneficiary  = key(0x04); // who can spend the vault after the deadline

    // ── Step 1: Ownership ────────────────────────────────────────────────────
    // Ownable(address initialOwner)
    let record = OwnershipRecord::new(owner_key);
    println!("[1] Owner set: {}", hex::encode(record.owner()));

    // Verify this call is by the owner — the onlyOwner modifier analog.
    record.verify_owner(owner_key).expect("caller is owner");
    println!("    verify_owner: OK\n");

    // ── Step 2: TimelockController — queue vault creation ────────────────────
    // TimelockController(minDelay=1_000 DAA heights, proposer, executor)
    // At 1 BPS ≈ 1_000 seconds (~17 min) of mandatory governance delay.
    let min_delay: u64 = 1_000;
    let mut ctrl = TimelockController::new(min_delay, proposer_key, executor_key)
        .expect("valid delay");

    // schedule() — proposer queues the vault activation.
    // Replace 500_000 with the real current DAA height from kaspad.
    let queue_daa: u64 = 500_000;
    ctrl.schedule(proposer_key, queue_daa).expect("proposer schedules");
    println!(
        "[2] Governance: operation queued at DAA {}. Earliest execution: {}",
        queue_daa,
        ctrl.earliest_execution_height().unwrap()
    );

    // isOperationReady — check before the delay elapses
    let too_soon = queue_daa + min_delay - 1;
    let ready_at = queue_daa + min_delay;
    println!("    is_ready at {}: {}", too_soon, ctrl.is_ready(too_soon));
    println!("    is_ready at {}: {}\n", ready_at, ctrl.is_ready(ready_at));

    // execute() — executor activates the vault creation once delay has elapsed.
    ctrl.execute(executor_key, ready_at).expect("executor activates");
    println!("[3] Governance: operation executed at DAA {}\n", ready_at);

    // ── Step 3: Vault — covenant-enforced custody ────────────────────────────
    // Create a timelocked vault: beneficiary can spend after deadline DAA height.
    // This models an ERC4626 vault or Escrow.
    let vault_deadline: u64 = ready_at + 10_000; // vault active 10_000 DAA heights after governance pass
    let condition = SpendCondition::TimelockHeight {
        deadline: vault_deadline,
        controller_xonly: beneficiary,
    };
    let vault = Vault::new(condition).expect("valid condition");

    // deposit(amount) — returns a VaultDescriptor (the UTXO spec to create on-chain)
    let deposit_sompi: u64 = 1_000_000_000; // 1 000 KAS
    let descriptor = vault.deposit(deposit_sompi).expect("non-zero deposit");
    println!(
        "[4] Vault: {} sompi locked under TimelockHeight deadline {}",
        descriptor.amount_sompi, vault_deadline
    );

    // Evaluate spending — not yet available
    let before_ctx = EvalContext {
        daa_score: vault_deadline - 1,
        unix_seconds: 0,
        signers_present: vec![beneficiary],
    };
    let at_ctx = EvalContext {
        daa_score: vault_deadline,
        unix_seconds: 0,
        signers_present: vec![beneficiary],
    };
    println!(
        "    can_spend at DAA {}: {} (before deadline)",
        vault_deadline - 1,
        evaluate(vault.condition(), &before_ctx)
    );
    println!(
        "    can_spend at DAA {}: {} (at deadline)\n",
        vault_deadline,
        evaluate(vault.condition(), &at_ctx)
    );

    // ── What just happened ───────────────────────────────────────────────────
    println!("--- Summary ---");
    println!("  Owner verified authority (Ownable::verify_owner).");
    println!("  Governance delay of {} DAA heights enforced (TimelockController).", min_delay);
    println!("  Vault created: {} sompi, redeemable at DAA {} by beneficiary {}.",
        descriptor.amount_sompi, vault_deadline,
        hex::encode(&beneficiary[..4]));
    println!();
    println!("Kaspa differences from Ethereum:");
    println!("  - No msg.sender: keys are supplied explicitly, enforced by covenant scripts.");
    println!("  - 'deposit' builds a UTXO spec; broadcast it to lock value on-chain.");
    println!("  - Spend conditions are enforced by Toccata consensus, not a contract runtime.");
    println!();
    println!("Pre-production, unaudited, testnet-only.");
}

