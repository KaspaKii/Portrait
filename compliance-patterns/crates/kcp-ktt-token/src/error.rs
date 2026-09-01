//! Error types for the `kcp-ktt-token` crate.

use thiserror::Error;

/// Errors from the KTT token pattern.
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

    /// Payload magic bytes are not `b"KCPKT"`.
    #[error("payload bad magic: not a KCPKT token payload")]
    PayloadBadMagic,

    /// Payload version byte is not `0x01`.
    #[error("payload bad version: got {0:#04x}, expected 0x01")]
    PayloadBadVersion(u8),

    /// State byte slice has the wrong length.
    #[error("state bad length: expected {expected}, got {got}")]
    StateBadLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        got: usize,
    },

    /// State has an unrecognised `identifier_type` byte.
    #[error("state bad identifier type: {0:#04x}")]
    StateBadIdentifierType(u8),

    /// State has an unrecognised `is_minter` byte (must be 0x00 or 0x01).
    #[error("state bad is_minter byte: {0:#04x}")]
    StateBadIsMinterByte(u8),

    /// KTT-1: supply conservation violated (output sum ≠ input sum when no
    /// minter input is involved in the operation).
    #[error("KTT-1 supply conservation violated: inputs sum to {input_sum}, outputs sum to {output_sum}")]
    SupplyConservation {
        /// Total amount across all inputs.
        input_sum: u64,
        /// Total amount across all outputs.
        output_sum: u64,
    },

    /// KTT-2: a non-minter input attempted to produce a minter output.
    #[error("KTT-2 minter escalation: non-minter input produced a minter output")]
    MinterEscalation,

    /// KTT-3: owner authorisation is not present for the spending input.
    #[error("KTT-3 owner auth absent: no authorised owner present for input state")]
    OwnerAuthAbsent,

    /// KTT-4: an output carries an invalid identifier type.
    #[error("KTT-4 invalid identifier type in output")]
    InvalidIdentifierType,

    /// Arithmetic overflow while summing token amounts.
    #[error("arithmetic overflow summing token amounts")]
    AmountOverflow,

    /// Operation received zero inputs where at least one is required.
    #[error("at least one input state is required")]
    NoInputs,

    /// Operation received zero outputs where at least one is required.
    #[error("at least one output state is required")]
    NoOutputs,

    /// Mint attempted without a minter input state.
    #[error("mint requires a minter input (is_minter = true)")]
    MintWithoutMinter,

    /// Burn amount exceeds the available input amount.
    #[error("burn amount {burn} exceeds input amount {available}")]
    BurnExceedsInput {
        /// Amount to burn.
        burn: u64,
        /// Amount available.
        available: u64,
    },

    /// Canonical hashing failed (serialisation error).
    #[error("canonical: {0}")]
    Canonical(#[from] kcp_common::canonical::CanonicalError),

    /// RPC transport or node-side failure.
    #[error("rpc: {0}")]
    Rpc(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
