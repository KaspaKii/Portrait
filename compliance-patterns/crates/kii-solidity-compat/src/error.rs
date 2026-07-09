use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    // ERC20 / KTT
    #[error("insufficient balance: available {available}, requested {requested}")]
    InsufficientBalance { available: u64, requested: u64 },
    #[error("ktt: {0}")]
    Ktt(#[from] kcp_ktt_token::error::Error),

    // Ownable
    #[error("caller is not the owner")]
    NotOwner,

    // TimelockController
    #[error("caller is not the proposer")]
    NotProposer,
    #[error("caller is not the executor")]
    NotExecutor,
    #[error("operation has been cancelled")]
    AlreadyCancelled,
    #[error("governance: {0}")]
    Governance(kcp_governance::error::GovernanceError),

    // Vault
    #[error("vault condition invalid: {0}")]
    VaultConditionInvalid(String),
    #[error("deposit amount must be > 0")]
    ZeroDeposit,
}

pub type Result<T> = std::result::Result<T, Error>;
