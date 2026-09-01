use sha2::{Digest, Sha256};

/// Specifies how to derive the 32-byte journal hash for each pattern.
///
/// The `journal` field in a KIP-16 tag-0x21 script is `sha256(journal_bytes)`.
/// Each pattern has a defined encoding for `journal_bytes`.
///
/// # Two-path design (rusty-kaspa v2.0.0 constraint)
///
/// `OpZkPrecompile` (0xa6) pops all eight proof fields and pushes `true`/`false`.
/// It does NOT re-push the journal hash to the script stack. Covenant scripts that
/// need to inspect the journal must receive `journal_bytes` as a separate clear-text
/// push in the unlocking script and independently compute `sha256(journal_bytes)` via
/// `OP_SHA256` to bind it to the proof.
///
/// `journal_hash()` on any variant returns the 32-byte value to use as the `journal`
/// argument in `build_pq_anchor_redeem` — the same value the covenant's `OP_SHA256`
/// step must produce over the corresponding `journal_bytes()` output.
pub enum JournalSpec {
    /// `kcp-paired-attestation`: journal_bytes = attestation_id (32) || spend_outpoint (36)
    PairedAttestation {
        attestation_id: [u8; 32],
        /// The outpoint being spent — 32-byte txid + 4-byte output index (LE).
        spend_outpoint: [u8; 36],
    },
    /// `kcp-sealed-lineage`: journal_bytes = lineage_id (32) || seq (8 LE) || t_bucket (8 LE)
    SealedLineage {
        lineage_id: [u8; 32],
        seq: u64,
        t_bucket: u64,
    },
    /// `kcp-transferable-record`: journal_bytes = record_id (32) || new_controller_xonly (32)
    TransferableRecord {
        record_id: [u8; 32],
        new_controller: [u8; 32],
    },
    /// Generalised vProg state transition (Varnish bridge).
    ///
    /// journal_bytes = covenant_id (32) || prev_state_hash (32) || next_state_hash (32) ||
    ///                 vprog_image_id (32) || seq (8 LE)  → total 136 bytes
    ///
    /// This models any vProg computation that transitions from one L1-committed state
    /// to another. The covenant binds to `next_state_hash` and `seq` monotonicity.
    /// `vprog_image_id` is the RISC Zero content-addressed hash of the guest ELF.
    VProgStateTransition {
        covenant_id: [u8; 32],
        prev_state_hash: [u8; 32],
        next_state_hash: [u8; 32],
        vprog_image_id: [u8; 32],
        seq: u64,
    },
    /// CSCI (Covenant-Settled Compliance Instrument) transfer.
    ///
    /// journal_bytes = covenant_id (32) || new_state_hash (32) || rule_hash (32) ||
    ///                 seq (8 LE)  → total 104 bytes
    ///
    /// See `docs/FLAGSHIP-DESIGN.md` §3.2 for the full field schema.
    /// `new_state_hash` = sha256 of the 50-byte KTT state extension (KttState + seq).
    CsciTransition {
        covenant_id: [u8; 32],
        new_state_hash: [u8; 32],
        rule_hash: [u8; 32],
        seq: u64,
    },
    /// Custom: caller provides the pre-hashed 32-byte journal directly.
    Custom([u8; 32]),
}

impl JournalSpec {
    /// Return the raw `journal_bytes` that are pushed in the clear in the unlocking
    /// script so the covenant locking script can inspect individual fields and
    /// independently compute `sha256(journal_bytes)` to bind them to the proof.
    ///
    /// Returns `None` for `Custom` (caller owns the encoding) and for variants
    /// where the journal_bytes length exceeds a single data push (not applicable).
    pub fn journal_bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::PairedAttestation {
                attestation_id,
                spend_outpoint,
            } => {
                let mut v = Vec::with_capacity(68);
                v.extend_from_slice(attestation_id);
                v.extend_from_slice(spend_outpoint);
                Some(v)
            }
            Self::SealedLineage {
                lineage_id,
                seq,
                t_bucket,
            } => {
                let mut v = Vec::with_capacity(48);
                v.extend_from_slice(lineage_id);
                v.extend_from_slice(&seq.to_le_bytes());
                v.extend_from_slice(&t_bucket.to_le_bytes());
                Some(v)
            }
            Self::TransferableRecord {
                record_id,
                new_controller,
            } => {
                let mut v = Vec::with_capacity(64);
                v.extend_from_slice(record_id);
                v.extend_from_slice(new_controller);
                Some(v)
            }
            Self::VProgStateTransition {
                covenant_id,
                prev_state_hash,
                next_state_hash,
                vprog_image_id,
                seq,
            } => {
                let mut v = Vec::with_capacity(136);
                v.extend_from_slice(covenant_id);
                v.extend_from_slice(prev_state_hash);
                v.extend_from_slice(next_state_hash);
                v.extend_from_slice(vprog_image_id);
                v.extend_from_slice(&seq.to_le_bytes());
                Some(v)
            }
            Self::CsciTransition {
                covenant_id,
                new_state_hash,
                rule_hash,
                seq,
            } => {
                let mut v = Vec::with_capacity(104);
                v.extend_from_slice(covenant_id);
                v.extend_from_slice(new_state_hash);
                v.extend_from_slice(rule_hash);
                v.extend_from_slice(&seq.to_le_bytes());
                Some(v)
            }
            Self::Custom(_) => None,
        }
    }

    /// Derive the `journal` hash (`sha256(journal_bytes)`) for this spec.
    ///
    /// This is the value passed as the `journal` argument to `build_pq_anchor_redeem`.
    /// It must equal `sha256(journal_bytes())` — the same computation the covenant
    /// locking script performs via `OP_SHA256` on the clear-text push.
    pub fn journal_hash(&self) -> [u8; 32] {
        match self {
            Self::Custom(hash) => *hash,
            other => {
                let bytes = other
                    .journal_bytes()
                    .expect("all non-Custom variants return Some");
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                hasher.finalize().into()
            }
        }
    }
}
