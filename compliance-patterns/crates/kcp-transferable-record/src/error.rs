//! Error types for the transferable-record crate.

use thiserror::Error;

use kcp_common::canonical::CanonicalError;

/// Errors from the transferable-record pattern.
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

    /// Payload magic bytes are not `b"KCPTR"`.
    #[error("payload bad magic: not a KCPTR transferable-record payload")]
    PayloadBadMagic,

    /// Payload version byte is not `0x01`.
    #[error("payload bad version: got {0:#04x}, expected 0x01")]
    PayloadBadVersion(u8),

    /// TR-1: sequence number is not the expected next value.
    #[error("lineage TR-1 seq gap: expected {expected}, got {got}")]
    LineageSeqGap {
        /// The value that was expected.
        expected: u64,
        /// The value found in the event.
        got: u64,
    },

    /// TR-2: a transfer event carries a different `record_id` than the first event.
    #[error("lineage TR-2 record_id mismatch at event index {index}")]
    LineageRecordIdMismatch {
        /// Zero-based index of the offending event.
        index: usize,
    },

    /// TR-3: a transfer event's `commitment` is all-zero bytes.
    #[error("lineage TR-3 empty commitment at event index {index}")]
    LineageEmptyCommitment {
        /// Zero-based index of the offending event.
        index: usize,
    },

    /// Canonical hashing failed (serialisation error).
    #[error("canonical: {0}")]
    Canonical(#[from] CanonicalError),

    /// RPC transport or node-side failure.
    #[error("rpc: {0}")]
    Rpc(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
