//! Error types for `kcp-yield-vault`.

/// Errors returned by vault profile operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    /// Depositing zero assets is not permitted.
    ZeroDeposit,
    /// Withdrawing zero shares is not permitted.
    ZeroWithdraw,
    /// Insufficient shares for the requested withdrawal.
    InsufficientShares,
    /// Arithmetic overflow during shares/assets conversion.
    ArithmeticOverflow,
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroDeposit => write!(f, "deposit amount must be greater than zero"),
            Self::ZeroWithdraw => write!(f, "withdraw shares must be greater than zero"),
            Self::InsufficientShares => write!(f, "insufficient shares for withdrawal"),
            Self::ArithmeticOverflow => write!(f, "arithmetic overflow in vault calculation"),
        }
    }
}

impl std::error::Error for VaultError {}

/// Result type for `kcp-yield-vault`.
pub type Result<T> = std::result::Result<T, VaultError>;
