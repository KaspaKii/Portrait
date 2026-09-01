//! KIP-20 cross-layer binding: ties a CSCI vProg journal to a specific covenant program.
//!
//! A KovId uniquely identifies a covenant *program* (bytecode + constructor args).
//! Including it in the CSCI journal makes the STARK proof non-transferable to any
//! other covenant type.

use crate::error::{CsciError, Result};

/// A KIP-20 covenant identity — 32-byte hash of covenant bytecode + constructor args.
///
/// Two UTXOs with the same silverscript program share the same KovId.
/// The ID is stable across UTXO respending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KovId(pub [u8; 32]);

impl KovId {
    /// Construct from a 32-byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(CsciError::InvalidJournal(format!(
                "KovId requires 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ok(KovId(arr))
    }

    /// Raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Cross-layer binding: parsed fields from a 104-byte CSCI journal.
///
/// Journal layout: `covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]`
#[derive(Debug, Clone)]
pub struct CovIdBinding {
    /// KIP-20 covenant identity of the covenant program this proof is bound to.
    pub kov_id: KovId,
    /// SHA-256 of the encoded new state after this transition.
    pub new_state_hash: [u8; 32],
    /// SHA-256 of the entry point name (UTF-8 bytes).
    pub rule_hash: [u8; 32],
    /// Monotonic sequence counter (u64 little-endian).
    pub seq: u64,
}

impl CovIdBinding {
    /// Parse a 104-byte CSCI journal into a CovIdBinding.
    pub fn from_journal(journal: &[u8; 104]) -> Self {
        let mut kov_id = [0u8; 32];
        let mut new_state_hash = [0u8; 32];
        let mut rule_hash = [0u8; 32];
        let mut seq_bytes = [0u8; 8];

        kov_id.copy_from_slice(&journal[0..32]);
        new_state_hash.copy_from_slice(&journal[32..64]);
        rule_hash.copy_from_slice(&journal[64..96]);
        seq_bytes.copy_from_slice(&journal[96..104]);

        CovIdBinding {
            kov_id: KovId(kov_id),
            new_state_hash,
            rule_hash,
            seq: u64::from_le_bytes(seq_bytes),
        }
    }

    /// Verify that this binding's covenant ID matches the expected program.
    pub fn verify_kov_id(&self, expected: &KovId) -> bool {
        self.kov_id == *expected
    }

    /// Verify that this binding's seq strictly advances past the last accepted seq.
    ///
    /// The covenant must enforce `seq > last_seq` to prevent proof replay.
    ///
    /// **Genesis initialisation:** the covenant must initialise `last_seq` to `0`.
    /// A genesis journal should carry `seq = 1` (1 > 0 ✓). Initialising `last_seq`
    /// to `u64::MAX` would reject every valid journal and permanently brick the instance.
    pub fn verify_seq_advance(&self, last_seq: u64) -> bool {
        self.seq > last_seq
    }

    /// Serialise back to 104-byte journal form.
    pub fn to_journal(&self) -> [u8; 104] {
        let mut journal = [0u8; 104];
        journal[0..32].copy_from_slice(&self.kov_id.0);
        journal[32..64].copy_from_slice(&self.new_state_hash);
        journal[64..96].copy_from_slice(&self.rule_hash);
        journal[96..104].copy_from_slice(&self.seq.to_le_bytes());
        journal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_journal() -> [u8; 104] {
        let mut j = [0u8; 104];
        // covenant_id: bytes 0..32 = 0x01 * 32
        j[0..32].fill(0x01);
        // new_state_hash: bytes 32..64 = 0x02 * 32
        j[32..64].fill(0x02);
        // rule_hash: bytes 64..96 = 0x03 * 32
        j[64..96].fill(0x03);
        // seq: bytes 96..104 = 7 LE
        j[96..104].copy_from_slice(&7u64.to_le_bytes());
        j
    }

    #[test]
    fn from_journal_parses_fields() {
        let j = sample_journal();
        let binding = CovIdBinding::from_journal(&j);
        assert_eq!(binding.kov_id.0, [0x01u8; 32]);
        assert_eq!(binding.new_state_hash, [0x02u8; 32]);
        assert_eq!(binding.rule_hash, [0x03u8; 32]);
        assert_eq!(binding.seq, 7);
    }

    #[test]
    fn verify_kov_id_and_seq() {
        let j = sample_journal();
        let binding = CovIdBinding::from_journal(&j);
        let expected = KovId([0x01u8; 32]);
        let wrong = KovId([0xFFu8; 32]);

        assert!(binding.verify_kov_id(&expected));
        assert!(!binding.verify_kov_id(&wrong));
        assert!(binding.verify_seq_advance(6)); // 7 > 6 ✓
        assert!(!binding.verify_seq_advance(7)); // 7 > 7 ✗
        assert!(!binding.verify_seq_advance(8)); // 7 > 8 ✗
    }

    #[test]
    fn roundtrip_journal() {
        let j = sample_journal();
        let binding = CovIdBinding::from_journal(&j);
        let j2 = binding.to_journal();
        assert_eq!(j, j2);
    }
}
