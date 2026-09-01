//! Kaspa wallet helper: load a key, derive the receive address, expose the
//! Schnorr keypair.
//!
//! Key material is read from a plaintext file containing either a
//! 64-character hex string (raw 32-byte secp256k1 private key) or a BIP-39
//! mnemonic on a single line (comment lines starting with `#` are ignored).
//! Mnemonics derive at the Kaspa BIP-44 path `m/44'/111111'/0'/0/{index}`.
//!
//! Testnet keys only. Nothing in this library is for mainnet value.

use std::path::Path;

pub use kaspa_addresses::Prefix;
use kaspa_addresses::{Address, Version};
use kaspa_bip32::{
    secp256k1::{self, Keypair},
    DerivationPath, ExtendedPrivateKey, Language, Mnemonic, SecretKey,
};
use thiserror::Error;

/// Errors from wallet loading and key derivation.
#[derive(Debug, Error)]
pub enum WalletError {
    /// The key file could not be read.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// The key line is not a valid mnemonic or hex private key.
    #[error("invalid key material: {0}")]
    KeyMaterial(String),

    /// BIP-32 derivation failed.
    #[error("derivation: {0}")]
    Derivation(String),

    /// secp256k1 rejected the key.
    #[error("secp: {0}")]
    Secp(#[from] secp256k1::Error),
}

/// Result alias for wallet operations.
pub type WalletResult<T> = Result<T, WalletError>;

/// A loaded wallet: Schnorr keypair plus the derived Kaspa address.
#[derive(Clone)]
pub struct Wallet {
    /// The secp256k1 keypair used for Schnorr transaction signing.
    pub keypair: Keypair,
    /// The derived Kaspa address (x-only pubkey, `Version::PubKey`).
    pub address: Address,
    /// The BIP-44 address index this wallet was derived at (0 for raw hex keys).
    pub address_index: u32,
    /// The address prefix (network) this wallet was derived for.
    pub prefix: Prefix,
}

impl std::fmt::Debug for Wallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallet")
            .field("address", &self.address.to_string())
            .field("address_index", &self.address_index)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl Wallet {
    /// Load a wallet from `path`. The first non-empty, non-comment line is
    /// interpreted as either a 64-character hex private key or a BIP-39
    /// mnemonic phrase.
    pub fn load(path: &Path, address_index: u32, prefix: Prefix) -> WalletResult<Self> {
        let raw = std::fs::read_to_string(path)?;
        let line = raw
            .lines()
            .find(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
            .ok_or_else(|| WalletError::KeyMaterial("file has no key/mnemonic line".into()))?
            .trim()
            .to_string();
        let candidate = line.strip_prefix("0x").unwrap_or(&line);
        if candidate.len() == 64 && candidate.chars().all(|c| c.is_ascii_hexdigit()) {
            return Self::from_private_key_hex(candidate, address_index, prefix);
        }
        Self::from_phrase(&line, "", address_index, prefix)
    }

    /// Build a wallet from a raw 32-byte secp256k1 private key in hex
    /// (with or without a `0x` prefix). Bypasses BIP-32 derivation.
    pub fn from_private_key_hex(
        hex_str: &str,
        address_index: u32,
        prefix: Prefix,
    ) -> WalletResult<Self> {
        let bytes = hex::decode(hex_str.trim_start_matches("0x"))
            .map_err(|e| WalletError::KeyMaterial(format!("invalid hex private key: {e}")))?;
        if bytes.len() != 32 {
            return Err(WalletError::KeyMaterial(
                "private key must be 32 bytes (64 hex chars)".into(),
            ));
        }
        let keypair = Keypair::from_seckey_slice(secp256k1::SECP256K1, &bytes)?;
        let xonly = keypair.x_only_public_key().0.serialize();
        let address = Address::new(prefix, Version::PubKey, &xonly);
        Ok(Self {
            keypair,
            address,
            address_index,
            prefix,
        })
    }

    /// Build a wallet from a BIP-39 mnemonic phrase, derived at the Kaspa
    /// BIP-44 path `m/44'/111111'/0'/0/{address_index}`.
    pub fn from_phrase(
        phrase: &str,
        passphrase: &str,
        address_index: u32,
        prefix: Prefix,
    ) -> WalletResult<Self> {
        let mnemonic = Mnemonic::new(phrase, Language::English)
            .map_err(|e| WalletError::KeyMaterial(e.to_string()))?;
        let seed = mnemonic.to_seed(passphrase);
        let xprv: ExtendedPrivateKey<SecretKey> = ExtendedPrivateKey::new(seed.as_bytes())
            .map_err(|e| WalletError::Derivation(e.to_string()))?;

        let path: DerivationPath = format!("m/44'/111111'/0'/0/{address_index}")
            .parse()
            .map_err(|e: kaspa_bip32::Error| WalletError::Derivation(e.to_string()))?;
        let child = xprv
            .derive_path(&path)
            .map_err(|e| WalletError::Derivation(e.to_string()))?;
        let secret_bytes: [u8; 32] = child.private_key().secret_bytes();
        let keypair = Keypair::from_seckey_slice(secp256k1::SECP256K1, &secret_bytes)?;

        let xonly = keypair.x_only_public_key().0.serialize();
        let address = Address::new(prefix, Version::PubKey, &xonly);

        Ok(Self {
            keypair,
            address,
            address_index,
            prefix,
        })
    }

    /// The wallet's address as a bech32 string.
    pub fn address_string(&self) -> String {
        self.address.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // BIP-39 English test phrase (the standard all-"abandon" vector).
    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn hex_key_deterministic_and_testnet_prefixed() {
        let key = "11".repeat(32);
        let a = Wallet::from_private_key_hex(&key, 0, Prefix::Testnet).unwrap();
        let b = Wallet::from_private_key_hex(&key, 0, Prefix::Testnet).unwrap();
        assert_eq!(a.address_string(), b.address_string());
        assert!(a.address_string().starts_with("kaspatest:"));
    }

    #[test]
    fn hex_key_0x_prefix_equivalent() {
        let key = "22".repeat(32);
        let plain = Wallet::from_private_key_hex(&key, 0, Prefix::Testnet).unwrap();
        let prefixed =
            Wallet::from_private_key_hex(&format!("0x{key}"), 0, Prefix::Testnet).unwrap();
        assert_eq!(plain.address_string(), prefixed.address_string());
    }

    #[test]
    fn phrase_derivation_deterministic_and_index_sensitive() {
        let a0 = Wallet::from_phrase(PHRASE, "", 0, Prefix::Testnet).unwrap();
        let a0_again = Wallet::from_phrase(PHRASE, "", 0, Prefix::Testnet).unwrap();
        let a1 = Wallet::from_phrase(PHRASE, "", 1, Prefix::Testnet).unwrap();
        assert_eq!(a0.address_string(), a0_again.address_string());
        assert_ne!(a0.address_string(), a1.address_string());
    }

    #[test]
    fn rejects_short_hex_key() {
        assert!(Wallet::from_private_key_hex("abcd", 0, Prefix::Testnet).is_err());
    }
}
