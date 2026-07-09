//! `Ownable`-shaped facade over a single-key ownership record.
//!
//! EVM equivalent: `Ownable` (Solidity pattern-library v5 shape).
//!
//! In Ethereum, `Ownable` stores an `address owner` in contract storage and
//! provides `onlyOwner` modifier gating. In Kaspa there is no contract
//! storage — access control is enforced by the **covenant script** (who can
//! sign to spend a UTXO). This type helps you track and reason about the
//! owner's public key in application logic before anchoring it to a covenant.
//!
//! | Solidity `Ownable` | This type |
//! |---|---|
//! | `Ownable(address initialOwner)` | `OwnershipRecord::new(owner_key)` |
//! | `owner()` | `record.owner()` |
//! | `transferOwnership(newOwner)` | `record.transfer_ownership(new_owner)` |
//! | `renounceOwnership()` | `record.renounce_ownership()` |
//! | `onlyOwner` modifier | `record.verify_owner(key)?` |
//!
//! **Kaspa difference:** there is no global `msg.sender`. The caller must
//! supply the signing key that authorises the operation; the covenant script
//! then enforces the owner's signature on-chain.
//!
//! **Pre-production, unaudited, testnet-only.**

use crate::error::{Error, Result};

/// A single-key ownership record — the Kaspa equivalent of `Ownable`.
///
/// Holds the owner's 32-byte x-only Schnorr public key. All state changes
/// return a new `OwnershipRecord` (pure value type).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipRecord {
    /// The owner's 32-byte x-only Schnorr public key.
    /// All-zero bytes represent "ownership renounced" (no owner).
    owner_key: [u8; 32],
}

impl OwnershipRecord {
    /// Create a new ownership record.
    ///
    /// Equivalent to `Ownable(address initialOwner)`.
    pub fn new(owner_key: [u8; 32]) -> Self {
        Self { owner_key }
    }

    /// Return the current owner's public key.
    ///
    /// All-zero → ownership has been renounced.
    pub fn owner(&self) -> [u8; 32] {
        self.owner_key
    }

    /// Return `true` if `key` matches the current owner.
    pub fn is_owner(&self, key: [u8; 32]) -> bool {
        self.owner_key == key
    }

    /// Assert that `key` is the current owner; return `Err(NotOwner)` otherwise.
    ///
    /// Use this to gate operations that should only be performed by the owner
    /// (the `onlyOwner` modifier analog).
    pub fn verify_owner(&self, key: [u8; 32]) -> Result<()> {
        if self.is_owner(key) {
            Ok(())
        } else {
            Err(Error::NotOwner)
        }
    }

    /// Transfer ownership to `new_owner`.
    ///
    /// Returns a new `OwnershipRecord`. The caller must persist this and
    /// build a transaction whose outputs use the new key.
    ///
    /// Equivalent to Solidity's `transferOwnership(newOwner)`.
    ///
    /// **Kaspa difference:** the on-chain enforcement is the covenant script —
    /// include the new key in the output script when you broadcast the tx.
    pub fn transfer_ownership(self, new_owner: [u8; 32]) -> Self {
        Self {
            owner_key: new_owner,
        }
    }

    /// Renounce ownership — sets the owner key to all-zero bytes.
    ///
    /// A renounced UTXO has no covenant-enforced owner. **This is
    /// irreversible in application logic** (you would need to use a different
    /// pattern to recover).
    ///
    /// Equivalent to Solidity's `renounceOwnership()`.
    pub fn renounce_ownership(self) -> Self {
        Self {
            owner_key: [0u8; 32],
        }
    }

    /// Return `true` if ownership has been renounced (owner key is all-zero).
    pub fn is_renounced(&self) -> bool {
        self.owner_key == [0u8; 32]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = b;
        k
    }

    #[test]
    fn owner_returns_initial_key() {
        let rec = OwnershipRecord::new(key(1));
        assert_eq!(rec.owner(), key(1));
    }

    #[test]
    fn is_owner_matches() {
        let rec = OwnershipRecord::new(key(2));
        assert!(rec.is_owner(key(2)));
        assert!(!rec.is_owner(key(3)));
    }

    #[test]
    fn verify_owner_ok_for_owner() {
        let rec = OwnershipRecord::new(key(4));
        assert!(rec.verify_owner(key(4)).is_ok());
    }

    #[test]
    fn verify_owner_err_for_non_owner() {
        let rec = OwnershipRecord::new(key(5));
        assert!(matches!(rec.verify_owner(key(6)), Err(Error::NotOwner)));
    }

    #[test]
    fn transfer_ownership_updates_key() {
        let rec = OwnershipRecord::new(key(7));
        let rec2 = rec.transfer_ownership(key(8));
        assert_eq!(rec2.owner(), key(8));
    }

    #[test]
    fn renounce_zeroes_key() {
        let rec = OwnershipRecord::new(key(9));
        let rec2 = rec.renounce_ownership();
        assert!(rec2.is_renounced());
        assert_eq!(rec2.owner(), [0u8; 32]);
    }
}
