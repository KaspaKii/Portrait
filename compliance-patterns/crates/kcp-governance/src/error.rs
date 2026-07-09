use thiserror::Error;

/// Errors returned by `kcp-governance` primitives.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GovernanceError {
    /// Voting deadline must be strictly after the proposal height.
    #[error("voting_deadline ({deadline}) must be > proposed_at_height ({proposed_at})")]
    InvalidDeadline { proposed_at: u64, deadline: u64 },

    /// Threshold must satisfy 1 ≤ threshold ≤ signatories.len().
    #[error("threshold {threshold} must be ≥ 1 and ≤ signatory count {count}")]
    InvalidThreshold { threshold: u8, count: usize },

    /// Signatory list is empty.
    #[error("signatory list must not be empty")]
    EmptySignatories,

    /// Duplicate signatory key.
    #[error("duplicate signatory key at index {index}")]
    DuplicateSignatory { index: usize },

    /// Signatory is not authorized to vote.
    #[error("key is not a registered signatory")]
    UnauthorizedSignatory,

    /// A key has already cast an approval.
    #[error("signatory has already approved this proposal")]
    AlreadyApproved,

    /// Action cannot be executed: proposal has not passed.
    #[error("proposal must be in Passed status to schedule execution")]
    ProposalNotPassed,

    /// Action cannot be executed: timelock delay has not elapsed.
    #[error("timelock delay not elapsed: need height >= {required}, current {current}")]
    TimelockNotElapsed { required: u64, current: u64 },

    /// minimum_delay must be ≥ 1 DAA height.
    #[error("minimum_delay must be ≥ 1")]
    InvalidMinimumDelay,
}
