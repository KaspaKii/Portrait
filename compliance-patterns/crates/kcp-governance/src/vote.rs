use crate::error::GovernanceError;
use serde::{Deserialize, Serialize};

/// k-of-n approval tracking for a [`GovernanceProposal`].
///
/// Tracks which signatories have approved. Does not hold cryptographic
/// signatures — it records the *keys* of signatories who have approved.
/// Actual Schnorr signature verification is the caller's responsibility
/// (use `kcp-common::p2sh::schnorr_satisfier_sig` or similar).
///
/// **Ordering:** keys in `signatories` define the canonical key order for
/// multisig script compilation. `approvals` records keys (not positions) so
/// the set is order-independent for quorum checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiSigVote {
    /// The ordered set of authorized signatories (x-only Schnorr public keys).
    pub signatories: Vec<[u8; 32]>,
    /// Minimum number of approvals required for the proposal to pass.
    pub threshold: u8,
    /// Keys that have cast an approval (subset of `signatories`).
    pub approvals: Vec<[u8; 32]>,
}

impl MultiSigVote {
    /// Create a new vote tracker for the given signatory set and threshold.
    ///
    /// Rules:
    /// - `signatories` must be non-empty, ≤ 16 keys, no duplicates.
    /// - `1 ≤ threshold ≤ signatories.len()`.
    pub fn new(signatories: Vec<[u8; 32]>, threshold: u8) -> Result<Self, GovernanceError> {
        if signatories.is_empty() {
            return Err(GovernanceError::EmptySignatories);
        }
        let t = threshold as usize;
        if t == 0 || t > signatories.len() {
            return Err(GovernanceError::InvalidThreshold {
                threshold,
                count: signatories.len(),
            });
        }
        // Detect duplicates (O(n²) — n ≤ 16 per MAX_MULTISIG_KEYS).
        for (i, k) in signatories.iter().enumerate() {
            if signatories[..i].contains(k) {
                return Err(GovernanceError::DuplicateSignatory { index: i });
            }
        }
        Ok(Self {
            signatories,
            threshold,
            approvals: Vec::new(),
        })
    }

    /// Record an approval from `key`.
    ///
    /// Returns `Err` if `key` is not a registered signatory or has already
    /// approved.
    pub fn approve(&mut self, key: [u8; 32]) -> Result<(), GovernanceError> {
        if !self.signatories.contains(&key) {
            return Err(GovernanceError::UnauthorizedSignatory);
        }
        if self.approvals.contains(&key) {
            return Err(GovernanceError::AlreadyApproved);
        }
        self.approvals.push(key);
        Ok(())
    }

    /// Returns `true` if the approval count meets the threshold.
    pub fn quorum_met(&self) -> bool {
        self.approvals.len() >= self.threshold as usize
    }

    /// Number of approvals still needed to reach quorum.
    pub fn approvals_needed(&self) -> usize {
        let have = self.approvals.len();
        let need = self.threshold as usize;
        need.saturating_sub(have)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn valid_2_of_3_vote() {
        let mut v = MultiSigVote::new(vec![key(1), key(2), key(3)], 2).unwrap();
        assert!(!v.quorum_met());
        assert_eq!(v.approvals_needed(), 2);
        v.approve(key(1)).unwrap();
        assert!(!v.quorum_met());
        assert_eq!(v.approvals_needed(), 1);
        v.approve(key(3)).unwrap();
        assert!(v.quorum_met());
        assert_eq!(v.approvals_needed(), 0);
    }

    #[test]
    fn unauthorized_signatory_rejected() {
        let mut v = MultiSigVote::new(vec![key(1), key(2)], 1).unwrap();
        assert_eq!(
            v.approve(key(99)),
            Err(GovernanceError::UnauthorizedSignatory)
        );
    }

    #[test]
    fn double_approval_rejected() {
        let mut v = MultiSigVote::new(vec![key(1), key(2)], 2).unwrap();
        v.approve(key(1)).unwrap();
        assert_eq!(v.approve(key(1)), Err(GovernanceError::AlreadyApproved));
    }

    #[test]
    fn duplicate_signatories_rejected() {
        let err = MultiSigVote::new(vec![key(1), key(1)], 1).unwrap_err();
        assert_eq!(err, GovernanceError::DuplicateSignatory { index: 1 });
    }

    #[test]
    fn empty_signatories_rejected() {
        assert_eq!(
            MultiSigVote::new(vec![], 1).unwrap_err(),
            GovernanceError::EmptySignatories
        );
    }

    #[test]
    fn threshold_zero_rejected() {
        let err = MultiSigVote::new(vec![key(1)], 0).unwrap_err();
        assert!(matches!(err, GovernanceError::InvalidThreshold { .. }));
    }

    #[test]
    fn threshold_exceeds_signatories_rejected() {
        let err = MultiSigVote::new(vec![key(1), key(2)], 3).unwrap_err();
        assert!(matches!(err, GovernanceError::InvalidThreshold { .. }));
    }

    #[test]
    fn serde_round_trip() {
        let mut v = MultiSigVote::new(vec![key(0xAA), key(0xBB), key(0xCC)], 2).unwrap();
        v.approve(key(0xAA)).unwrap();
        let json = serde_json::to_string(&v).unwrap();
        let v2: MultiSigVote = serde_json::from_str(&json).unwrap();
        assert_eq!(v, v2);
    }
}
