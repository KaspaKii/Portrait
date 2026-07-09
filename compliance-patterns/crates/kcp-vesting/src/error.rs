//! Error types for `kcp-vesting`.

/// Errors returned by vesting schedule operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VestingError {
    /// The vesting duration is zero.
    DurationZero,
    /// The requested release amount exceeds the releasable amount.
    NothingToRelease,
    /// Arithmetic overflow during vesting calculation.
    ArithmeticOverflow,
}

impl std::fmt::Display for VestingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DurationZero => write!(f, "vesting duration must be greater than zero"),
            Self::NothingToRelease => write!(f, "no vested amount available to release"),
            Self::ArithmeticOverflow => write!(f, "arithmetic overflow in vesting calculation"),
        }
    }
}

impl std::error::Error for VestingError {}

/// Result type for `kcp-vesting`.
pub type Result<T> = std::result::Result<T, VestingError>;
