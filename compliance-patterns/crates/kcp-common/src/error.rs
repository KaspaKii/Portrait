//! Shared error types for `kcp-common`.

use thiserror::Error;

/// Errors produced by `kcp-common` modules (node-facing plumbing and
/// access-control primitives).
#[derive(Debug, Error)]
pub enum Error {
    /// RPC transport or node-side failure, with context.
    #[error("rpc: {0}")]
    Rpc(String),

    /// An access-control and security primitive ([`crate::access::Ownable`] /
    /// [`crate::access::Multisig`]) failed validation.
    ///
    /// Note: `kcp-vault` independently defines a `ConditionInvalid` variant in
    /// its own error type. The two are not unified in v0.1 — see `KNOWN-ISSUES.md`.
    #[error("condition invalid: {0}")]
    ConditionInvalid(String),
}

/// Result alias for `kcp-common` operations.
pub type Result<T> = std::result::Result<T, Error>;
