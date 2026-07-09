# kcp-governance

`kcp-governance` provides DAG-native governance primitives: a content-addressed
proposal, k-of-n approval tracking, a post-pass timelock, and a combined
`GovernorState` lifecycle. It is the `Governor` + `TimelockController` equivalent
for the Kaspa BlockDAG.

DAA heights serve as the on-chain clock. **Kaspa's DAG does not have
globally-sequential block numbers.** Use DAA heights as *approximate* clocks.

## Running a governance cycle

```rust
use kcp_governance::{
    action::TimelockAction, governor::GovernorState,
    proposal::GovernanceProposal, vote::MultiSigVote,
};

// 1. Create vote and apply approvals before building GovernorState
let mut vote = MultiSigVote::new(vec![key_a, key_b, key_c], 2)?;
vote.approve(key_a)?;
vote.approve(key_b)?; // quorum reached

// 2. Build GovernorState with quorum already met
let proposal = GovernanceProposal::new("fund auditor", current_height, voting_deadline)?;
let action   = TimelockAction::new(500)?;
let mut gov  = GovernorState::new(proposal, vote, action, current_height);

// 3. Advance past deadline → Passed
gov.refresh_status(voting_deadline + 1);

// 4. Schedule and execute
gov.schedule_action(voting_deadline + 1)?;
gov.execute(voting_deadline + 502)?; // after 500-DAA delay
```

## A note on no token-weighted voting

`MultiSigVote` uses a fixed signatory set (k-of-n). Token-weighted voting is
**deferred** until a KRC20-equivalent with snapshotted balances exists on Kaspa
mainnet. This is an intentional honest limitation.

**Verify Schnorr signatures** before calling `MultiSigVote::approve()` — the vote
tracker records approvals by key but does NOT verify cryptographic signatures.

## Extensions

- **Vault integration** — use the same key set as the governance signatories in a `kcp-vault` P2SH. See `examples/governance-demo`.
- **On-chain continuity** — anchor `GovernorState` serialised snapshots to a `kcp-sealed-lineage` lineage for an immutable governance audit trail.

→ API reference: [`GovernanceProposal`], [`MultiSigVote`], [`TimelockAction`], [`GovernorState`], [`proposal_id`]
