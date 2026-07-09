//! Error types for the `kcp-vault` crate.

use thiserror::Error;

/// Errors from the vault pattern.
#[derive(Debug, Error)]
pub enum Error {
    /// A [`crate::condition::SpendCondition`] failed validation.
    #[error("condition invalid: {0}")]
    ConditionInvalid(String),

    /// Payload byte slice has the wrong length.
    #[error("payload bad length: expected {expected}, got {got}")]
    PayloadBadLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        got: usize,
    },

    /// Payload magic bytes are not `b"KCPVT"`.
    #[error("payload bad magic: not a KCPVT vault payload")]
    PayloadBadMagic,

    /// Payload version byte is not `0x01`.
    #[error("payload bad version: got {0:#04x}, expected 0x01")]
    PayloadBadVersion(u8),

    /// Script compilation is not supported for the given condition shape.
    ///
    /// v0 compilation is limited to: leaf conditions, `All(leaves)`, and
    /// `Any` of exactly 2 branches. Deeper or wider composite conditions must
    /// be broken apart by the caller. The pure evaluator still supports full
    /// nesting.
    #[error("script compile unsupported: {0}")]
    CompileUnsupported(String),

    /// Script builder returned an error (opcode encoding failure).
    #[error("script builder: {0}")]
    ScriptBuilder(String),

    /// Canonical hashing failed (serialisation error).
    #[error("canonical: {0}")]
    Canonical(#[from] kcp_common::canonical::CanonicalError),

    /// RPC transport or node-side failure.
    #[error("rpc: {0}")]
    Rpc(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
