use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CsciError {
    #[error("seq overflow: u64::MAX reached")]
    SeqOverflow,
    #[error("transfer amount must be > 0")]
    ZeroAmount,
    #[error("transfer amount {amount} exceeds available balance {balance}")]
    InsufficientBalance { amount: u64, balance: u64 },
    #[error("rule_hash mismatch: expected {expected}, got {actual}")]
    RuleHashMismatch { expected: String, actual: String },
    #[error("invalid journal: {0}")]
    InvalidJournal(String),
}

pub type Result<T> = std::result::Result<T, CsciError>;
