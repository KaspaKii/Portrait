use crate::{
    action::TimelockAction,
    error::GovernanceError,
    proposal::{GovernanceProposal, ProposalStatus},
    vote::MultiSigVote,
};
use serde::{Deserialize, Serialize};

/// Combined governance session: proposal + vote tracker + timelock action.
///
/// `GovernorState` is the top-level DAG-native governance primitive. It
/// sequences a proposal through its lifecycle using DAA heights as the clock
/// and k-of-n multisig as the voting mechanism.
///
/// ## Lifecycle
///
/// ```text
/// new() → [Pending] → approve_by() calls (voting window) → advance() →
///   [Active] → advance() at deadline →
///     [Passed] → schedule_action() → can_execute() → execute() → [Executed]
///     [Rejected] (quorum not met)
///   * → cancel() → [Cancelled]
/// ```
///
/// ## DAG-height note
///
/// `current_height` arguments are the caller's view of the current DAA height.
/// They are not verified against any external source — this is a pure value
/// type. Callers must supply the authoritative height from their node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernorState {
    /// The proposal being governed.
    pub proposal: GovernanceProposal,
    /// Vote tracker for the proposal.
    pub vote: MultiSigVote,
    /// Timelock for the post-pass execution delay.
    pub action: TimelockAction,
    /// Current lifecycle status.
    pub status: ProposalStatus,
}

impl GovernorState {
    /// Create a new governance session.
    ///
    /// `current_height` is used to determine the initial status (Pending vs
    /// Active if the window has already opened).
    pub fn new(
        proposal: GovernanceProposal,
        vote: MultiSigVote,
        action: TimelockAction,
        current_height: u64,
    ) -> Self {
        let status = proposal.status_at(current_height, false);
        Self {
            proposal,
            vote,
            action,
            status,
        }
    }

    /// Record an approval from `key` and refresh the status.
    ///
    /// Fails if:
    /// - the proposal is not `Active`,
    /// - `key` is not a registered signatory, or
    /// - `key` has already approved.
    pub fn approve(&mut self, key: [u8; 32], current_height: u64) -> Result<(), GovernanceError> {
        self.refresh_status(current_height);
        if self.status != ProposalStatus::Active {
            return Err(GovernanceError::ProposalNotPassed);
        }
        self.vote.approve(key)?;
        Ok(())
    }

    /// Advance the lifecycle to reflect the current height and vote state.
    ///
    /// Should be called after each `approve()` and before any read of
    /// `self.status`.
    pub fn refresh_status(&mut self, current_height: u64) {
        let quorum = self.vote.quorum_met();
        let derived = self.proposal.status_at(current_height, quorum);
        // Only advance — never regress. Terminal states are sticky.
        match self.status {
            ProposalStatus::Cancelled | ProposalStatus::Executed => {}
            _ => self.status = derived,
        }
    }

    /// Schedule the post-pass timelock action.
    ///
    /// Fails if the proposal has not yet `Passed`.
    pub fn schedule_action(&mut self, current_height: u64) -> Result<(), GovernanceError> {
        self.refresh_status(current_height);
        if self.status != ProposalStatus::Passed {
            return Err(GovernanceError::ProposalNotPassed);
        }
        self.action.schedule(current_height)
    }

    /// Mark the proposal as `Executed` if the timelock has elapsed.
    ///
    /// Fails if:
    /// - the proposal is not in `Passed` status (action not yet scheduled or
    ///   not passed), or
    /// - the timelock delay has not elapsed.
    pub fn execute(&mut self, current_height: u64) -> Result<(), GovernanceError> {
        self.refresh_status(current_height);
        if self.status != ProposalStatus::Passed {
            return Err(GovernanceError::ProposalNotPassed);
        }
        self.action.can_execute(current_height)?;
        self.status = ProposalStatus::Executed;
        Ok(())
    }

    /// Cancel the proposal unconditionally (except if already Executed).
    ///
    /// Cancelled proposals cannot be revived.
    pub fn cancel(&mut self) -> Result<(), GovernanceError> {
        if self.status == ProposalStatus::Executed {
            return Err(GovernanceError::ProposalNotPassed);
        }
        self.status = ProposalStatus::Cancelled;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::GovernanceProposal;

    fn key(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn make_governor() -> GovernorState {
        let proposal = GovernanceProposal::new("upgrade v2", 100, 200).unwrap();
        let vote = MultiSigVote::new(vec![key(1), key(2), key(3)], 2).unwrap();
        let action = TimelockAction::new(50).unwrap();
        GovernorState::new(proposal, vote, action, 90)
    }

    #[test]
    fn initial_status_pending_before_window() {
        let gov = make_governor();
        assert_eq!(gov.status, ProposalStatus::Pending);
    }

    #[test]
    fn initial_status_active_after_window_opens() {
        let proposal = GovernanceProposal::new("test", 100, 200).unwrap();
        let vote = MultiSigVote::new(vec![key(1)], 1).unwrap();
        let action = TimelockAction::new(10).unwrap();
        let gov = GovernorState::new(proposal, vote, action, 150);
        assert_eq!(gov.status, ProposalStatus::Active);
    }

    #[test]
    fn full_happy_path() {
        let mut gov = make_governor();

        // Voting window opens
        gov.refresh_status(100);
        assert_eq!(gov.status, ProposalStatus::Active);

        // Two signatories approve
        gov.approve(key(1), 100).unwrap();
        gov.approve(key(2), 150).unwrap();
        assert!(gov.vote.quorum_met());

        // Past voting deadline → Passed
        gov.refresh_status(200);
        assert_eq!(gov.status, ProposalStatus::Passed);

        // Schedule execution at height 200; earliest = 250
        gov.schedule_action(200).unwrap();
        assert_eq!(gov.action.earliest_execution_height(), Some(250));

        // Can't execute before delay elapses
        assert!(gov.execute(249).is_err());

        // Execute at 250 (earliest eligible height)
        gov.execute(250).unwrap();
        assert_eq!(gov.status, ProposalStatus::Executed);
    }

    #[test]
    fn rejected_when_quorum_not_met_at_deadline() {
        let mut gov = make_governor();
        gov.refresh_status(100); // Active
        gov.vote.approve(key(1)).unwrap(); // only 1 of 2 required
        gov.refresh_status(200); // deadline reached
        assert_eq!(gov.status, ProposalStatus::Rejected);
    }

    #[test]
    fn cancel_works_before_execution() {
        let mut gov = make_governor();
        gov.cancel().unwrap();
        assert_eq!(gov.status, ProposalStatus::Cancelled);
    }

    #[test]
    fn cannot_cancel_after_execution() {
        let mut gov = make_governor();
        gov.refresh_status(100);
        gov.approve(key(1), 100).unwrap();
        gov.approve(key(2), 100).unwrap();
        gov.refresh_status(200);
        gov.schedule_action(200).unwrap();
        gov.execute(250).unwrap();
        assert!(gov.cancel().is_err());
    }

    #[test]
    fn approve_rejected_outside_active_window() {
        let mut gov = make_governor(); // Pending at height 90
        assert!(gov.approve(key(1), 90).is_err()); // not Active yet
    }

    #[test]
    fn schedule_requires_passed_status() {
        let mut gov = make_governor();
        gov.refresh_status(100); // Active but not Passed
        assert!(gov.schedule_action(100).is_err());
    }

    #[test]
    fn serde_round_trip() {
        let mut gov = make_governor();
        gov.refresh_status(100);
        gov.approve(key(1), 100).unwrap();
        let json = serde_json::to_string(&gov).unwrap();
        let g2: GovernorState = serde_json::from_str(&json).unwrap();
        assert_eq!(gov, g2);
    }
}
