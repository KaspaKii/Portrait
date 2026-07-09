//! On-chain carrier payload codec for KTT token operation evidence.
//!
//! ## Wire format
//!
//! ```text
//! Offset  Len  Field
//! ------  ---  -----
//!      0    5  magic            b"KCPKT"
//!      5    1  version          0x01
//!      6   32  token_id         canonical_hash(genesis issuance params)
//!     38    1  op_class         0x00=issue, 0x01=transfer, 0x02=mint, 0x03=burn
//!     39   32  state_commitment canonical_hash(post-op KttState)
//! ------  ---
//!             TOTAL: 71 bytes
//! ```
//!
//! The magic bytes `KCPKT` and version byte guard against accidentally parsing
//! an unrelated transaction payload as a KTT evidence payload.
//!
//! `token_id` is the [`kcp_common::canonical::canonical_hash`] of the genesis
//! issuance parameters, tying the on-chain record to a specific token
//! deployment.
//!
//! `op_class` identifies the operation anchored by this carrier transaction.
//!
//! `state_commitment` is the canonical_hash of the post-operation
//! [`KttState`](crate::state::KttState), encoded via
//! [`KttState::encode`](crate::state::KttState::encode).

use crate::error::{Error, Result};

/// Magic prefix for KTT token evidence payloads.
pub const PAYLOAD_MAGIC: &[u8; 5] = b"KCPKT";

/// Codec version for this encoding.
pub const PAYLOAD_VERSION: u8 = 0x01;

/// Total wire length of an encoded payload.
pub const PAYLOAD_LEN: usize = 5 + 1 + 32 + 1 + 32; // 71

/// Operation class discriminant carried in the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpClass {
    /// Genesis issuance (creating the initial minter or supply state).
    Issue = 0x00,
    /// Transfer between owners.
    Transfer = 0x01,
    /// Mint new supply via a minter state.
    Mint = 0x02,
    /// Burn tokens (reduce supply).
    Burn = 0x03,
}

impl OpClass {
    /// Decode from a single byte.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PayloadBadVersion`] for an unrecognised byte. (Reuses
    /// the version error variant for compactness — unknown op_class bytes in
    /// a future payload version are treated the same way.)
    pub fn from_byte(b: u8) -> Result<Self> {
        match b {
            0x00 => Ok(Self::Issue),
            0x01 => Ok(Self::Transfer),
            0x02 => Ok(Self::Mint),
            0x03 => Ok(Self::Burn),
            other => Err(Error::PayloadBadVersion(other)), // re-using; op_class variant future
        }
    }

    /// Encode to the wire byte.
    pub fn to_byte(self) -> u8 {
        self as u8
    }
}

/// A decoded KTT token evidence payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// Token identifier: canonical_hash of the genesis issuance parameters.
    pub token_id: [u8; 32],
    /// The operation anchored by this carrier transaction.
    pub op_class: OpClass,
    /// Commitment to the post-operation token state: canonical_hash of the
    /// encoded [`KttState`](crate::state::KttState).
    pub state_commitment: [u8; 32],
}

impl Payload {
    /// Encode the payload to the 71-byte wire format.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PAYLOAD_LEN);
        out.extend_from_slice(PAYLOAD_MAGIC);
        out.push(PAYLOAD_VERSION);
        out.extend_from_slice(&self.token_id);
        out.push(self.op_class.to_byte());
        out.extend_from_slice(&self.state_commitment);
        debug_assert_eq!(out.len(), PAYLOAD_LEN);
        out
    }

    /// Decode a payload from raw bytes.
    ///
    /// # Errors
    ///
    /// - [`Error::PayloadBadLength`] if the byte slice is not exactly 71 bytes.
    /// - [`Error::PayloadBadMagic`] if the first 5 bytes are not `b"KCPKT"`.
    /// - [`Error::PayloadBadVersion`] if byte 5 is not `0x01`.
    /// - Re-uses [`Error::PayloadBadVersion`] for an unrecognised `op_class`
    ///   byte (byte 38).
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != PAYLOAD_LEN {
            return Err(Error::PayloadBadLength {
                expected: PAYLOAD_LEN,
                got: bytes.len(),
            });
        }
        if &bytes[0..5] != PAYLOAD_MAGIC {
            return Err(Error::PayloadBadMagic);
        }
        let version = bytes[5];
        if version != PAYLOAD_VERSION {
            return Err(Error::PayloadBadVersion(version));
        }

        let mut token_id = [0u8; 32];
        token_id.copy_from_slice(&bytes[6..38]);

        let op_class = OpClass::from_byte(bytes[38])?;

        let mut state_commitment = [0u8; 32];
        state_commitment.copy_from_slice(&bytes[39..71]);

        Ok(Self {
            token_id,
            op_class,
            state_commitment,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(op: OpClass) -> Payload {
        Payload {
            token_id: [0xaa; 32],
            op_class: op,
            state_commitment: [0xbb; 32],
        }
    }

    #[test]
    fn encode_length() {
        assert_eq!(sample(OpClass::Issue).encode().len(), PAYLOAD_LEN);
    }

    #[test]
    fn round_trip_issue() {
        let p = sample(OpClass::Issue);
        assert_eq!(Payload::decode(&p.encode()).unwrap(), p);
    }

    #[test]
    fn round_trip_transfer() {
        let p = sample(OpClass::Transfer);
        assert_eq!(Payload::decode(&p.encode()).unwrap(), p);
    }

    #[test]
    fn round_trip_mint() {
        let p = sample(OpClass::Mint);
        assert_eq!(Payload::decode(&p.encode()).unwrap(), p);
    }

    #[test]
    fn round_trip_burn() {
        let p = sample(OpClass::Burn);
        assert_eq!(Payload::decode(&p.encode()).unwrap(), p);
    }

    #[test]
    fn encode_starts_with_magic_and_version() {
        let encoded = sample(OpClass::Issue).encode();
        assert_eq!(&encoded[0..5], b"KCPKT");
        assert_eq!(encoded[5], 0x01);
    }

    #[test]
    fn op_class_wire_values() {
        assert_eq!(OpClass::Issue.to_byte(), 0x00);
        assert_eq!(OpClass::Transfer.to_byte(), 0x01);
        assert_eq!(OpClass::Mint.to_byte(), 0x02);
        assert_eq!(OpClass::Burn.to_byte(), 0x03);
    }

    #[test]
    fn decode_wrong_length_short() {
        let err = Payload::decode(&[0u8; 70]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 71,
                    got: 70
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn decode_wrong_length_long() {
        let err = Payload::decode(&[0u8; 72]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 71,
                    got: 72
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn decode_bad_magic() {
        let mut bytes = sample(OpClass::Issue).encode();
        bytes[0] = b'X';
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(matches!(err, Error::PayloadBadMagic), "unexpected: {err}");
    }

    #[test]
    fn decode_bad_version() {
        let mut bytes = sample(OpClass::Issue).encode();
        bytes[5] = 0x02;
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadVersion(0x02)),
            "unexpected: {err}"
        );
    }

    #[test]
    fn decode_bad_op_class() {
        let mut bytes = sample(OpClass::Issue).encode();
        bytes[38] = 0xff;
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadVersion(0xff)),
            "unexpected: {err}"
        );
    }

    #[test]
    fn fields_round_trip_distinct_values() {
        let p = Payload {
            token_id: [0x11; 32],
            op_class: OpClass::Burn,
            state_commitment: [0x22; 32],
        };
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.token_id, [0x11; 32]);
        assert_eq!(decoded.op_class, OpClass::Burn);
        assert_eq!(decoded.state_commitment, [0x22; 32]);
    }

    #[test]
    fn corruption_in_token_id_detected() {
        let p = sample(OpClass::Transfer);
        let mut bytes = p.encode();
        bytes[6] ^= 0xff;
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.token_id, p.token_id);
    }

    #[test]
    fn corruption_in_state_commitment_detected() {
        let p = sample(OpClass::Transfer);
        let mut bytes = p.encode();
        bytes[39] ^= 0xff;
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.state_commitment, p.state_commitment);
    }
}
