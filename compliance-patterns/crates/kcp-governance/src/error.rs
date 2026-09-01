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

    /// Lineage verification: the anchored-step slice was empty.
    #[error("lineage: chain must have at least one anchored step")]
    LineageEmpty,

    /// Lineage verification: the sealed-lineage chain invariants (L-1..L-4)
    /// rejected the anchored payload sequence.
    #[error("lineage: sealed-lineage chain invalid: {0}")]
    LineageChainInvalid(String),

    /// Lineage verification: a step's config-derived `lineage_id` does not
    /// match the anchored payload `lineage_id` (the immutable governor config
    /// drifted).
    #[error("lineage: identity mismatch at step {index}")]
    LineageIdentityMismatch { index: usize },

    /// Lineage verification: a disclosed `GovernorState` does not reproduce the
    /// anchored commitment (the disclosed state was tampered).
    #[error("lineage: commitment mismatch at step {index}")]
    LineageCommitmentMismatch { index: usize },

    /// Lineage verification: an illegal lifecycle transition, or an event class
    /// that does not match the step's status, at the given step.
    #[error("lineage: illegal lifecycle transition at step {index}")]
    LineageIllegalTransition { index: usize },

    /// Lineage verification: a disclosed state contradicts its own status (e.g.
    /// a `Passed`/`Executed` state without quorum, a `Rejected` state with
    /// quorum, or an `Executed` state whose timelock was never scheduled).
    #[error("lineage: disclosed state is internally inconsistent at step {index}")]
    LineageStateInconsistent { index: usize },

    /// Lineage: canonical serialisation of a governor state (or its identity
    /// body) failed.
    #[error("lineage: serialisation failed: {0}")]
    LineageSerialization(String),
}
