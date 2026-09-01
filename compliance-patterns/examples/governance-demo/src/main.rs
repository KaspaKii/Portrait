//! Foundation treasury governance cycle — reference implementation.
//!
//! Chains kcp-vault (treasury) + kcp-governance (DAG-native governance):
//!   GovernanceProposal → MultiSigVote (quorum) → TimelockAction → execute
//!
//! DAA heights serve as the on-chain clock — no globally-sequential block numbers
//! exist in Kaspa's DAG.
//!
//! Pre-production, unaudited, testnet-only.

use kcp_governance::{
    action::TimelockAction,
    governor::GovernorState,
    proposal::GovernanceProposal,
    vote::MultiSigVote,
};
use kcp_vault::{
    condition::SpendCondition,
    onchain::compile_condition_p2sh,
    script::vault_script_digest,
};

fn main() {
    println!("kaspa-compliance-patterns — Foundation Treasury Governance Cycle");
    println!("==================================================================");
    println!("Pre-production, unaudited, testnet-only.\n");

    let key_a: [u8; 32] = [0xA1u8; 32];
    let key_b: [u8; 32] = [0xA2u8; 32];
    let key_c: [u8; 32] = [0xA3u8; 32];

    let current_height:  u64 = 500_000;
    let voting_deadline: u64 = current_height + 1_000;
    let timelock_delay:  u64 = 500;

    // ── [1] Treasury vault ────────────────────────────────────────────────────
    println!("[1] Treasury vault established — 2-of-3 multisig");

    let condition = SpendCondition::MultiSig {
        threshold: 2,
        xonly_keys: vec![key_a, key_b, key_c],
    };
    let redeem    = compile_condition_p2sh(&condition).expect("vault redeem script must compile");
    let p2sh_hash = vault_script_digest(&redeem);
    println!("    ✓ 2-of-3 vault — redeem {} bytes, P2SH hash: {}", redeem.len(), hex::encode(&p2sh_hash[..4]));

    // ── [2] Governance proposal ───────────────────────────────────────────────
    println!("[2] Governance proposal raised");

    let proposal = GovernanceProposal::new(
        "Release 100 KAS from treasury to fund auditor engagement",
        current_height,
        voting_deadline,
    ).expect("proposal must be valid");
    println!("    ✓ Proposal id: {}", hex::encode(&proposal.id[..8]));
    println!("    ✓ Voting window: DAA {} → {}", current_height, voting_deadline);

    // ── [3] Committee votes ───────────────────────────────────────────────────
    println!("[3] Committee members cast approvals");

    let mut vote = MultiSigVote::new(vec![key_a, key_b, key_c], 2)
        .expect("2-of-3 vote tracker must initialise");
    vote.approve(key_a).expect("key_a approval");
    vote.approve(key_b).expect("key_b approval");
    assert!(vote.quorum_met(), "quorum must be met after 2 approvals");
    println!("    ✓ key_a approved");
    println!("    ✓ key_b approved — quorum reached (2 of 3 required)");

    // ── [4] GovernorState lifecycle ───────────────────────────────────────────
    println!("[4] Governor lifecycle: vote applied → refresh to Passed");

    let action     = TimelockAction::new(timelock_delay).expect("timelock must initialise");
    let mut governor = GovernorState::new(proposal, vote, action, current_height);

    // Advance past the voting deadline — quorum already met → status becomes Passed.
    let post_vote_height = voting_deadline + 1;
    governor.refresh_status(post_vote_height);
    println!("    ✓ Proposal status: {:?}", governor.status);

    // ── [5] Schedule timelock ─────────────────────────────────────────────────
    println!("[5] Scheduling timelock action ({}-DAA delay)", timelock_delay);

    governor.schedule_action(post_vote_height)
        .expect("schedule must succeed when proposal has passed");
    let earliest = governor.action.earliest_execution_height()
        .expect("execution height must be set after scheduling");
    println!("    ✓ Timelock scheduled — earliest execution: DAA {}", earliest);

    // ── [6] Execute ───────────────────────────────────────────────────────────
    println!("[6] Executing — DAA height past timelock");

    let execution_height = earliest + 1;
    governor.execute(execution_height)
        .expect("execute must succeed after delay elapsed");
    println!("    ✓ Governance action executed — status: {:?}", governor.status);

    // ── [7] Summary ───────────────────────────────────────────────────────────
    println!("\n── Summary ──────────────────────────────────────────────────────────");
    println!("vault redeem    : {} bytes (2-of-3 multisig P2SH)", redeem.len());
    println!("proposal_id     : {}", hex::encode(governor.proposal.id));
    println!("quorum met      : {}", governor.vote.quorum_met());
    println!("executed at DAA : {}", execution_height);
    println!();
    println!("DAA heights are approximate clocks — not globally sequential block numbers.");
    println!("Before live use: replace [0xANu8; 32] keys with real Schnorr x-only pubkeys.");
}
