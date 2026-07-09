//! Solidity-pattern-shaped facades over the `kcp-*` pattern library.
//!
//! ## Who this is for
//!
//! An Ethereum developer who knows the established Solidity pattern libraries
//! and wants to build on Kaspa
//! from a familiar API surface. Method names are intentionally Solidity-shaped;
//! semantics are UTXO-native (see each module's doc for what changes).
//!
//! ## Modules
//!
//! | Solidity pattern | This crate | Underlying `kcp-*` crate |
//! |---|---|---|
//! | `ERC20` | [`erc20::Token`] | `kcp-ktt-token` |
//! | `Ownable` | [`ownable::OwnershipRecord`] | (pure value type) |
//! | `TimelockController` | [`timelock::TimelockController`] | `kcp-governance` |
//! | `ERC4626` / `Escrow` | [`vault::Vault`] | `kcp-vault` |
//!
//! **Pre-production, unaudited, testnet-only.**

pub mod erc20;
pub mod error;
pub mod ownable;
pub mod timelock;
pub mod vault;

pub use erc20::Token;
pub use error::Error;
pub use ownable::OwnershipRecord;
pub use timelock::TimelockController;
pub use vault::{Vault, VaultDescriptor};
