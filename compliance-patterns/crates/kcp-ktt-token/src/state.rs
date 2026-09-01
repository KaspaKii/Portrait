//! KCC20-shape token state — the 4-field on-chain representation.
//!
//! ## Wire layout
//!
//! ```text
//! Offset  Len  Field
//! ------  ---  -----
//!      0    1  identifier_type   (0x00 = Pubkey, 0x01 = ScriptHash, 0x02 = CovenantId)
//!      1   32  owner_identifier  (32 raw bytes)
//!     33    8  amount            (u64, little-endian)
//!     41    1  is_minter         (0x00 = false, 0x01 = true)
//! ------  ---
//!             TOTAL: 42 bytes
//! ```
//!
//! This fixed-width layout is what the KCC20 `validateOutputState` /
//! `validateOutputStateWithTemplate` enforcement primitives operate on. The
//! ordering (identifier_type first, then owner, then amount, then is_minter)
//! follows the KCC20 reference contract field sequence (FACTS SS-008b).
//!
//! All encode/decode is zero-copy where possible. Strict decode: any unknown
//! byte value in a discriminant field is an error.

use crate::error::{Error, Result};

/// The fixed encoded length of a [`KttState`] in bytes.
pub const STATE_LEN: usize = 1 + 32 + 8 + 1; // 42

/// The discriminant for an owner identifier in a [`KttState`].
///
/// Mirrors the KCC20 `identifierType` field:
/// - `0x00`: 32-byte x-only public key
/// - `0x01`: 32-byte script hash (P2SH)
/// - `0x02`: 32-byte covenant identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IdentifierType {
    /// 32-byte x-only public key (Schnorr / BIP-340).
    Pubkey = 0x00,
    /// 32-byte P2SH script hash.
    ScriptHash = 0x01,
    /// 32-byte covenant identifier (KIP-20 covenant-id).
    CovenantId = 0x02,
}

impl IdentifierType {
    /// Decode from a single byte.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StateBadIdentifierType`] for any byte other than
    /// `0x00`, `0x01`, or `0x02`.
    pub fn from_byte(b: u8) -> Result<Self> {
        match b {
            0x00 => Ok(Self::Pubkey),
            0x01 => Ok(Self::ScriptHash),
            0x02 => Ok(Self::CovenantId),
            other => Err(Error::StateBadIdentifierType(other)),
        }
    }

    /// Encode to the wire byte.
    pub fn to_byte(self) -> u8 {
        self as u8
    }
}

/// The 4-field KCC20-shape token state.
///
/// Matches the `KCC20Token` state fields described in FACTS SS-008b:
/// `ownerIdentifier` (byte\[32\]), `identifierType` (byte), `amount`
/// (u64), `isMinter` (bool).
///
/// Every token UTXO on-chain carries one of these states encoded in the
/// covenant payload (in v0: as a carrier-transaction payload via
/// [`crate::payload`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KttState {
    /// How to interpret `owner_identifier`.
    pub identifier_type: IdentifierType,
    /// 32-byte identifier of the owner — pubkey, script-hash, or
    /// covenant-id depending on `identifier_type`.
    pub owner_identifier: [u8; 32],
    /// Token balance in the smallest representable unit.
    pub amount: u64,
    /// `true` if this state holder controls issuance (the minter branch of
    /// the KCC20Minter pattern, FACTS SS-008c).
    pub is_minter: bool,
}

impl KttState {
    /// Encode to the 42-byte wire format.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(STATE_LEN);
        out.push(self.identifier_type.to_byte());
        out.extend_from_slice(&self.owner_identifier);
        out.extend_from_slice(&self.amount.to_le_bytes());
        out.push(if self.is_minter { 0x01 } else { 0x00 });
        debug_assert_eq!(out.len(), STATE_LEN);
        out
    }

    /// Decode from the 42-byte wire format.
    ///
    /// # Errors
    ///
    /// - [`Error::StateBadLength`] if the slice is not exactly 42 bytes.
    /// - [`Error::StateBadIdentifierType`] if byte 0 is not 0x00/0x01/0x02.
    /// - [`Error::StateBadIsMinterByte`] if byte 41 is not 0x00/0x01.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != STATE_LEN {
            return Err(Error::StateBadLength {
                expected: STATE_LEN,
                got: bytes.len(),
            });
        }
        let identifier_type = IdentifierType::from_byte(bytes[0])?;

        let mut owner_identifier = [0u8; 32];
        owner_identifier.copy_from_slice(&bytes[1..33]);

        let amount = u64::from_le_bytes(bytes[33..41].try_into().expect("slice is 8 bytes"));

        let is_minter = match bytes[41] {
            0x00 => false,
            0x01 => true,
            other => return Err(Error::StateBadIsMinterByte(other)),
        };

        Ok(Self {
            identifier_type,
            owner_identifier,
            amount,
            is_minter,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state(id_type: IdentifierType, is_minter: bool) -> KttState {
        KttState {
            identifier_type: id_type,
            owner_identifier: [0xab; 32],
            amount: 1_000_000,
            is_minter,
        }
    }

    #[test]
    fn encode_length() {
        assert_eq!(
            sample_state(IdentifierType::Pubkey, false).encode().len(),
            STATE_LEN
        );
    }

    #[test]
    fn round_trip_pubkey_not_minter() {
        let s = sample_state(IdentifierType::Pubkey, false);
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn round_trip_pubkey_is_minter() {
        let s = sample_state(IdentifierType::Pubkey, true);
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn round_trip_script_hash_not_minter() {
        let s = sample_state(IdentifierType::ScriptHash, false);
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn round_trip_script_hash_is_minter() {
        let s = sample_state(IdentifierType::ScriptHash, true);
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn round_trip_covenant_id_not_minter() {
        let s = sample_state(IdentifierType::CovenantId, false);
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn round_trip_covenant_id_is_minter() {
        let s = sample_state(IdentifierType::CovenantId, true);
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn identifier_type_wire_values() {
        assert_eq!(IdentifierType::Pubkey.to_byte(), 0x00);
        assert_eq!(IdentifierType::ScriptHash.to_byte(), 0x01);
        assert_eq!(IdentifierType::CovenantId.to_byte(), 0x02);
    }

    #[test]
    fn decode_bad_length_short() {
        let err = KttState::decode(&[0u8; 41]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::StateBadLength {
                    expected: 42,
                    got: 41
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn decode_bad_length_long() {
        let err = KttState::decode(&[0u8; 43]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::StateBadLength {
                    expected: 42,
                    got: 43
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn decode_bad_identifier_type() {
        let mut bytes = sample_state(IdentifierType::Pubkey, false).encode();
        bytes[0] = 0x03;
        let err = KttState::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::StateBadIdentifierType(0x03)),
            "unexpected: {err}"
        );
    }

    #[test]
    fn decode_bad_is_minter_byte() {
        let mut bytes = sample_state(IdentifierType::Pubkey, false).encode();
        bytes[41] = 0x02;
        let err = KttState::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::StateBadIsMinterByte(0x02)),
            "unexpected: {err}"
        );
    }

    #[test]
    fn amount_zero_round_trips() {
        let s = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: [0u8; 32],
            amount: 0,
            is_minter: false,
        };
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn amount_max_round_trips() {
        let s = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: [0xffu8; 32],
            amount: u64::MAX,
            is_minter: true,
        };
        assert_eq!(KttState::decode(&s.encode()).unwrap(), s);
    }
}
