//! On-chain payload codec for transferable records.
//!
//! ## Wire format
//!
//! ```text
//! Offset  Len  Field
//! ------  ---  -----
//!      0    5  magic        b"KCPTR"
//!      5    1  version      0x01
//!      6   32  record_id    SHA-256 of canonical genesis body
//!     38    8  seq          transfer sequence number, u64 little-endian
//!     46   32  commitment   SHA-256 of canonical event body
//! ------  ---
//!             TOTAL: 78 bytes
//! ```
//!
//! The magic bytes `KCPTR` plus the version byte guard against accidentally
//! parsing an unrelated transaction payload as a transferable-record event.
//! Version `0x01` is this v0 codec; future versions increment this byte.

use crate::error::{Error, Result};

/// Magic prefix for transferable-record payloads.
pub const PAYLOAD_MAGIC: &[u8; 5] = b"KCPTR";

/// Codec version for this encoding.
pub const PAYLOAD_VERSION: u8 = 0x01;

/// Total wire length of an encoded payload.
pub const PAYLOAD_LEN: usize = 5 + 1 + 32 + 8 + 32; // 78

/// A decoded transferable-record payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// The record identifier (SHA-256 of the genesis body).
    pub record_id: [u8; 32],
    /// Transfer sequence number. The first transfer has `seq = 1`; each
    /// subsequent transfer increments by exactly 1.
    pub seq: u64,
    /// Commitment to the event body (SHA-256 of the canonical event JSON).
    pub commitment: [u8; 32],
}

impl Payload {
    /// Encode the payload to the 78-byte wire format.
    ///
    /// Returns the raw bytes to embed in the Kaspa transaction payload field.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PAYLOAD_LEN);
        out.extend_from_slice(PAYLOAD_MAGIC);
        out.push(PAYLOAD_VERSION);
        out.extend_from_slice(&self.record_id);
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.commitment);
        debug_assert_eq!(out.len(), PAYLOAD_LEN);
        out
    }

    /// Decode a payload from raw bytes.
    ///
    /// # Errors
    ///
    /// - [`Error::PayloadBadLength`] if the byte slice is not exactly 78 bytes.
    /// - [`Error::PayloadBadMagic`] if the first 5 bytes are not `b"KCPTR"`.
    /// - [`Error::PayloadBadVersion`] if byte 5 is not `0x01`.
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

        let mut record_id = [0u8; 32];
        record_id.copy_from_slice(&bytes[6..38]);

        let seq = u64::from_le_bytes(bytes[38..46].try_into().expect("slice length checked"));

        let mut commitment = [0u8; 32];
        commitment.copy_from_slice(&bytes[46..78]);

        Ok(Self {
            record_id,
            seq,
            commitment,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload(seq: u64) -> Payload {
        Payload {
            record_id: [0xaa; 32],
            seq,
            commitment: [0xbb; 32],
        }
    }

    #[test]
    fn round_trip() {
        let p = sample_payload(1);
        let encoded = p.encode();
        assert_eq!(encoded.len(), PAYLOAD_LEN);
        let decoded = Payload::decode(&encoded).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_seq_zero() {
        let p = sample_payload(0);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.seq, 0);
    }

    #[test]
    fn round_trip_seq_max() {
        let p = sample_payload(u64::MAX);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.seq, u64::MAX);
    }

    #[test]
    fn encode_starts_with_magic() {
        let encoded = sample_payload(1).encode();
        assert_eq!(&encoded[0..5], b"KCPTR");
        assert_eq!(encoded[5], 0x01);
    }

    #[test]
    fn decode_wrong_length_short() {
        let err = Payload::decode(&[0u8; 77]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 78,
                    got: 77
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_wrong_length_long() {
        let err = Payload::decode(&[0u8; 79]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 78,
                    got: 79
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_bad_magic() {
        let mut bytes = sample_payload(1).encode();
        bytes[0] = b'X'; // corrupt first byte of magic
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadMagic),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_bad_version() {
        let mut bytes = sample_payload(1).encode();
        bytes[5] = 0x02; // unsupported version
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadVersion(0x02)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn fields_round_trip_with_distinct_values() {
        let p = Payload {
            record_id: [0x11; 32],
            seq: 42,
            commitment: [0x22; 32],
        };
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.record_id, [0x11; 32]);
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded.commitment, [0x22; 32]);
    }
}
