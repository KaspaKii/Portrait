use crate::error::GovernanceError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Lifecycle status of a [`GovernanceProposal`].
///
/// Transitions (enforced by [`GovernanceProposal::advance`]):
/// ```text
/// Pending → Active (current_height >= proposed_at_height)
/// Active  → Passed   (voting_deadline reached + quorum met)
/// Active  → Rejected (voting_deadline reached + quorum not met)
/// Passed  → Executed (execution guard cleared)
/// *       → Cancelled (explicit cancel)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalStatus {
    /// Created; voting window not yet open.
    Pending,
    /// Voting window is open (`current_height ≥ proposed_at_height`,
    /// `current_height < voting_deadline`).
    Active,
    /// Voting window closed with quorum met — awaiting execution.
    Passed,
    /// Voting window closed without quorum — proposal is dead.
    Rejected,
    /// Action has been executed; proposal is final.
    Executed,
    /// Proposal was cancelled before execution.
    Cancelled,
}

/// A DAG-native governance proposal.
///
/// Identifies a proposed action by content hash, records the DAA-height window
/// during which votes may be cast, and tracks lifecycle status.
///
/// **DAG-height clock:** Kaspa does not have globally-sequential block numbers.
/// `proposed_at_height` and `voting_deadline` are DAA (block-DAG) heights —
/// monotonically non-decreasing per node but not globally total-ordered across
/// concurrent blocks. Use them as an *approximate* elapsed-time signal; do not
/// rely on exact equality in time-critical applications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceProposal {
    /// SHA-256 of the canonical proposal content (description + payload bytes).
    pub id: [u8; 32],
    /// Human-readable description of the proposed action.
    pub description: String,
    /// DAA height at which this proposal was submitted.
    pub proposed_at_height: u64,
    /// DAA height by which all votes must be cast (`voting_deadline > proposed_at_height`).
    pub voting_deadline: u64,
}

impl GovernanceProposal {
    /// Create a new proposal and derive its `id` from the description.
    ///
    /// `voting_deadline` must be strictly greater than `proposed_at_height`.
    pub fn new(
        description: impl Into<String>,
        proposed_at_height: u64,
        voting_deadline: u64,
    ) -> Result<Self, GovernanceError> {
        let description = description.into();
        if voting_deadline <= proposed_at_height {
            return Err(GovernanceError::InvalidDeadline {
                proposed_at: proposed_at_height,
                deadline: voting_deadline,
            });
        }
        let id = proposal_id(description.as_bytes(), proposed_at_height);
        Ok(Self {
            id,
            description,
            proposed_at_height,
            voting_deadline,
        })
    }

    /// Compute the expected [`ProposalStatus`] given the current DAA height
    /// and whether quorum has been reached.
    ///
    /// Does not mutate state — callers integrate this into their own state
    /// machine.
    pub fn status_at(&self, current_height: u64, quorum_met: bool) -> ProposalStatus {
        if current_height < self.proposed_at_height {
            return ProposalStatus::Pending;
        }
        if current_height < self.voting_deadline {
            return ProposalStatus::Active;
        }
        // Voting window has closed.
        if quorum_met {
            ProposalStatus::Passed
        } else {
            ProposalStatus::Rejected
        }
    }
}

/// Derive a deterministic proposal id: `SHA-256(description || proposed_at_height_le64)`.
pub fn proposal_id(description_bytes: &[u8], proposed_at_height: u64) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(description_bytes);
    hasher.update(proposed_at_height.to_le_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_proposal_derives_id_deterministically() {
        let p1 = GovernanceProposal::new("upgrade kcp-vault to v2", 1_000, 2_000).unwrap();
        let p2 = GovernanceProposal::new("upgrade kcp-vault to v2", 1_000, 2_000).unwrap();
        assert_eq!(p1.id, p2.id);
    }

    #[test]
    fn different_descriptions_produce_different_ids() {
        let p1 = GovernanceProposal::new("proposal A", 1_000, 2_000).unwrap();
        let p2 = GovernanceProposal::new("proposal B", 1_000, 2_000).unwrap();
        assert_ne!(p1.id, p2.id);
    }

    #[test]
    fn different_heights_produce_different_ids() {
        let p1 = GovernanceProposal::new("same description", 1_000, 2_000).unwrap();
        let p2 = GovernanceProposal::new("same description", 1_001, 2_000).unwrap();
        assert_ne!(p1.id, p2.id);
    }

    #[test]
    fn invalid_deadline_rejected() {
        assert!(GovernanceProposal::new("x", 1_000, 1_000).is_err());
        assert!(GovernanceProposal::new("x", 1_000, 999).is_err());
    }

    #[test]
    fn status_at_lifecycle() {
        let p = GovernanceProposal::new("x", 100, 200).unwrap();
        assert_eq!(p.status_at(99, false), ProposalStatus::Pending);
        assert_eq!(p.status_at(100, false), ProposalStatus::Active);
        assert_eq!(p.status_at(199, false), ProposalStatus::Active);
        assert_eq!(p.status_at(200, true), ProposalStatus::Passed);
        assert_eq!(p.status_at(200, false), ProposalStatus::Rejected);
        assert_eq!(p.status_at(999, true), ProposalStatus::Passed);
    }

    #[test]
    fn serde_round_trip() {
        let p = GovernanceProposal::new("test governance action", 500, 1_000).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let p2: GovernanceProposal = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }
}
