//! CSCI redeem-script assembly — thin wrapper over `kcp_pq_anchor`.
//!
//! Provides [`build_csci_redeem`] (the on-chain KIP-16 tag-0x21 redeem script)
//! and [`csci_proof_fields`] (convenience constructor for [`PqAnchorScriptFields`]).
//!
//! **Pre-production, unaudited, testnet-only.**

use kcp_pq_anchor::anchor_script::{build_pq_anchor_redeem, PqAnchorError, PqAnchorScriptFields};

use crate::state::CsciStateTransition;

/// Assemble a KIP-16 tag-0x21 ZK-proof redeem script for a CSCI settlement.
///
/// The returned redeem script embeds the STARK proof fields and runs
/// `OpZkPrecompile` on-chain. The CSCI state continuity enforcement
/// (seq monotonicity, new_state_hash binding) is enforced off-chain by the
/// vProg until silverscript adds a ZK verification builtin.
///
/// `image_id` must be the RISC Zero image ID of the CSCI vProg guest binary.
pub fn build_csci_redeem(fields: &PqAnchorScriptFields) -> Result<Vec<u8>, PqAnchorError> {
    build_pq_anchor_redeem(fields)
}

/// Convenience: construct a `PqAnchorScriptFields` from a `CsciStateTransition`
/// and the STARK proof output fields from the RISC Zero prover.
///
/// The `journal` field is automatically derived from `transition.journal_hash()`.
pub fn csci_proof_fields(
    transition: &CsciStateTransition,
    claim: Vec<u8>,
    control_index: u32,
    control_digests: Vec<u8>,
    seal: Vec<u8>,
    image_id: [u8; 32],
    control_id: [u8; 32],
) -> PqAnchorScriptFields {
    PqAnchorScriptFields {
        claim,
        control_index,
        control_digests,
        seal,
        journal: transition.journal_hash(),
        image_id,
        control_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CsciState, CsciStateTransition};

    fn dummy_transition() -> CsciStateTransition {
        let covenant_id = [0u8; 32];
        let rule_hash = [1u8; 32];
        let prev = CsciState::new_genesis([2u8; 32], 1000, rule_hash, covenant_id);
        CsciStateTransition::transfer(&prev, [3u8; 32], 500, rule_hash).unwrap()
    }

    #[test]
    fn build_csci_redeem_produces_nonempty_script() {
        let transition = dummy_transition();
        let fields = csci_proof_fields(
            &transition,
            vec![0u8; 32], // claim
            0,             // control_index
            vec![0u8; 32], // control_digests (multiple of 32)
            vec![0u8; 64], // seal
            [0u8; 32],     // image_id
            [0u8; 32],     // control_id
        );
        let script = build_csci_redeem(&fields).unwrap();
        assert!(
            script.len() > 10,
            "redeem script should be non-trivial, got {} bytes",
            script.len()
        );
    }

    #[test]
    fn csci_proof_fields_sets_journal_from_transition() {
        let transition = dummy_transition();
        let fields = csci_proof_fields(
            &transition,
            vec![],
            0,
            vec![0u8; 32],
            vec![],
            [0u8; 32],
            [0u8; 32],
        );
        assert_eq!(fields.journal, transition.journal_hash());
    }
}
