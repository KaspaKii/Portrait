//! Error types for the paired-attestation crate.

use thiserror::Error;

use kcp_common::canonical::CanonicalError;

/// Errors from the paired-attestation pattern.
#[derive(Debug, Error)]
pub enum Error {
    /// Payload byte slice has the wrong length.
    #[error("payload bad length: expected {expected}, got {got}")]
    PayloadBadLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        got: usize,
    },

    /// Payload magic bytes are not `b"KCPPA"`.
    #[error("payload bad magic: not a KCPPA paired-attestation payload")]
    PayloadBadMagic,

    /// Payload version byte is not `0x01`.
    #[error("payload bad version: got {0:#04x}, expected 0x01")]
    PayloadBadVersion(u8),

    /// Chain validation failed: payloads slice was empty.
    #[error("invariant: chain must have at least one event (PartyACommit)")]
    InvariantEmptyChain,

    /// PA-1: sequence number is not the expected next value.
    #[error("invariant PA-1 seq gap at index {index}: expected {expected}, got {got}")]
    InvariantSeqGap {
        /// Zero-based index of the offending event.
        index: usize,
        /// The value that was expected.
        expected: u64,
        /// The value found in the payload.
        got: u64,
    },

    /// PA-2: a payload carries a different `attestation_id` than the first event.
    #[error("invariant PA-2 attestation_id mismatch at index {index}")]
    InvariantAttestationIdMismatch {
        /// Zero-based index of the offending event.
        index: usize,
    },

    /// PA-3: PartyACommit event class used at a sequence number other than 0,
    /// or a non-PartyACommit class used at sequence number 0.
    #[error("invariant PA-3 event-class rule violated at index {index}")]
    InvariantClassAtSeqZero {
        /// Zero-based index of the offending event.
        index: usize,
    },

    /// PA-3: an event appears after a Close event.
    #[error("invariant PA-3 event after Close at index {index}")]
    InvariantEventAfterClose {
        /// Zero-based index of the event that follows a Close.
        index: usize,
    },

    /// PA-3: PartyBMate must appear at seq 1.
    #[error("invariant PA-3 PartyBMate not at seq 1 (index {index})")]
    InvariantMateNotAtSeqOne {
        /// Zero-based index of the offending event.
        index: usize,
    },

    /// PA-3: the event class byte is not a known value.
    #[error("invariant PA-3 unknown event class {class:#04x} at index {index}")]
    InvariantUnknownEventClass {
        /// Zero-based index of the offending event.
        index: usize,
        /// The unrecognised class byte.
        class: u8,
    },

    /// PA-4: the mate proof carried in the seq-1 event failed verification.
    #[error("invariant PA-4 mate proof invalid at index {index}: {reason}")]
    InvariantMateProofInvalid {
        /// Zero-based index of the seq-1 event.
        index: usize,
        /// Human-readable failure reason from `verify_mate`.
        reason: String,
    },

    /// Mate proof verification failed: Party A commitment did not match.
    #[error(
        "mate proof: commit_a mismatch — recomputed commitment does not match claimed commit_a"
    )]
    MateCommitAMismatch,

    /// Mate proof verification failed: Party B commitment did not match.
    #[error(
        "mate proof: commit_b mismatch — recomputed commitment does not match claimed commit_b"
    )]
    MateCommitBMismatch,

    /// Mate proof field mismatch: the attestation_id in the proof does not
    /// match the one derived from the record bytes.
    #[error("mate proof: attestation_id mismatch — record bytes do not hash to the claimed attestation_id")]
    MateAttestationIdMismatch,

    /// Canonical hashing failed (serialisation error).
    #[error("canonical: {0}")]
    Canonical(#[from] CanonicalError),

    /// RPC transport or node-side failure.
    #[error("rpc: {0}")]
    Rpc(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
