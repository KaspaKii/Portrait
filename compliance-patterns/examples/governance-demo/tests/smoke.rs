use kcp_governance::{
    action::TimelockAction,
    governor::GovernorState,
    proposal::GovernanceProposal,
    vote::MultiSigVote,
};

fn run_governance_cycle(proposed_at: u64, voting_deadline: u64, timelock_delay: u64) {
    let key_a = [0xA1u8; 32];
    let key_b = [0xA2u8; 32];
    let key_c = [0xA3u8; 32];

    let proposal = GovernanceProposal::new("test proposal", proposed_at, voting_deadline)
        .expect("proposal must be valid");

    let mut vote = MultiSigVote::new(vec![key_a, key_b, key_c], 2)
        .expect("vote must initialise");
    vote.approve(key_a).expect("key_a approval");
    vote.approve(key_b).expect("key_b approval");
    assert!(vote.quorum_met());

    let action = TimelockAction::new(timelock_delay).expect("timelock must initialise");
    let mut governor = GovernorState::new(proposal, vote, action, proposed_at);

    let post_vote = voting_deadline + 1;
    governor.refresh_status(post_vote);
    governor.schedule_action(post_vote).expect("schedule must succeed");

    let earliest = governor.action.earliest_execution_height().unwrap();
    governor.execute(earliest + 1).expect("execute must succeed after delay");
}

#[test]
fn full_governance_cycle_executes() {
    run_governance_cycle(500_000, 501_000, 500);
}

#[test]
fn proposal_id_is_deterministic() {
    let p1 = GovernanceProposal::new("desc", 100, 200).unwrap();
    let p2 = GovernanceProposal::new("desc", 100, 200).unwrap();
    assert_eq!(p1.id, p2.id);
}

#[test]
fn quorum_requires_threshold() {
    let key_a = [0xA1u8; 32];
    let key_b = [0xA2u8; 32];
    let key_c = [0xA3u8; 32];
    let mut vote = MultiSigVote::new(vec![key_a, key_b, key_c], 2).unwrap();
    assert!(!vote.quorum_met(), "quorum not met with zero approvals");
    vote.approve(key_a).unwrap();
    assert!(!vote.quorum_met(), "quorum not met with one approval");
    vote.approve(key_b).unwrap();
    assert!(vote.quorum_met(), "quorum met with two approvals");
}

#[test]
fn execute_fails_before_delay_elapses() {
    let key_a = [0xA1u8; 32];
    let key_b = [0xA2u8; 32];
    let key_c = [0xA3u8; 32];
    let mut vote = MultiSigVote::new(vec![key_a, key_b, key_c], 2).unwrap();
    vote.approve(key_a).unwrap();
    vote.approve(key_b).unwrap();
    let proposal = GovernanceProposal::new("test", 100, 200).unwrap();
    let action = TimelockAction::new(100).unwrap();
    let mut gov = GovernorState::new(proposal, vote, action, 100);
    gov.refresh_status(201);
    gov.schedule_action(201).unwrap();
    // Try to execute before the 100-block delay elapses
    assert!(gov.execute(250).is_err(), "must fail before delay elapses");
    // Execute after delay
    assert!(gov.execute(302).is_ok(), "must succeed after delay elapses");
}

#[test]
fn vote_rejected_when_proposal_not_active() {
    // Approvals cannot be cast when the proposal is in Pending status.
    // (GovernorState::approve guards on Active status.)
    use kcp_governance::governor::GovernorState;
    use kcp_governance::proposal::GovernanceProposal;
    use kcp_governance::vote::MultiSigVote;
    use kcp_governance::action::TimelockAction;

    let key_a = [0xA1u8; 32];
    let key_b = [0xA2u8; 32];
    let key_c = [0xA3u8; 32];

    // Proposal window: height 1000..2000
    let proposal = GovernanceProposal::new("test", 1_000, 2_000).unwrap();
    let vote = MultiSigVote::new(vec![key_a, key_b, key_c], 2).unwrap();
    let action = TimelockAction::new(100).unwrap();

    // Create governor at height 500 — proposal is Pending (voting window not yet open)
    let mut gov = GovernorState::new(proposal, vote, action, 500);

    // Attempt to approve before the window opens — must fail
    assert!(
        gov.approve(key_a, 500).is_err(),
        "approve must fail when proposal is Pending"
    );

    // After deadline with quorum not met → Rejected; approve must also fail
    gov.refresh_status(2_001);
    assert!(
        gov.approve(key_b, 2_001).is_err(),
        "approve must fail when proposal is Rejected"
    );
}
