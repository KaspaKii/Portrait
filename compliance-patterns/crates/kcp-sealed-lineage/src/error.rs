//! Error types for the sealed-lineage crate.

use thiserror::Error;

use kcp_common::canonical::CanonicalError;

/// Errors from the sealed-lineage pattern.
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

    /// Payload magic bytes are not `b"KCPSL"`.
    #[error("payload bad magic: not a KCPSL sealed-lineage payload")]
    PayloadBadMagic,

    /// Payload version byte is not `0x01`.
    #[error("payload bad version: got {0:#04x}, expected 0x01")]
    PayloadBadVersion(u8),

    /// Chain validation failed: payloads slice was empty.
    #[error("invariant: chain must have at least one event (genesis)")]
    InvariantEmptyChain,

    /// L-1: sequence number is not the expected next value.
    #[error("invariant L-1 seq gap at index {index}: expected {expected}, got {got}")]
    InvariantSeqGap {
        /// Zero-based index of the offending event.
        index: usize,
        /// The value that was expected.
        expected: u64,
        /// The value found in the payload.
        got: u64,
    },

    /// L-2: a payload carries a different `lineage_id` than the first event.
    #[error("invariant L-2 lineage_id mismatch at index {index}")]
    InvariantLineageIdMismatch {
        /// Zero-based index of the offending event.
        index: usize,
    },

    /// L-3: Genesis event class used at a sequence number other than 0, or a
    /// non-Genesis class used at sequence number 0.
    #[error("invariant L-3 genesis-class rule violated at index {index}")]
    InvariantGenesisNotAtSeqZero {
        /// Zero-based index of the offending event.
        index: usize,
    },

    /// L-3: an event appears after a Close event.
    #[error("invariant L-3 event after Close at index {index}")]
    InvariantEventAfterClose {
        /// Zero-based index of the event that follows a Close.
        index: usize,
    },

    /// L-3: the event class byte is not a known value.
    #[error("invariant L-3 unknown event class {class:#04x} at index {index}")]
    InvariantUnknownEventClass {
        /// Zero-based index of the offending event.
        index: usize,
        /// The unrecognised class byte.
        class: u8,
    },

    /// L-4: `t_bucket` decreased relative to the previous event.
    #[error("invariant L-4 t_bucket decreased at index {index}: prev={prev}, got={got}")]
    InvariantTBucketDecreased {
        /// Zero-based index of the offending event.
        index: usize,
        /// Previous `t_bucket` value.
        prev: u64,
        /// Current (lower) `t_bucket` value.
        got: u64,
    },

    /// L-4: the step between consecutive `t_bucket` values exceeds the
    /// maximum allowed cadence.
    #[error("invariant L-4 t_bucket step too large at index {index}: step={step}, max={max}")]
    InvariantTBucketStepExceeded {
        /// Zero-based index of the offending event.
        index: usize,
        /// The actual step (seconds).
        step: u64,
        /// The maximum allowed step (seconds).
        max: u64,
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
