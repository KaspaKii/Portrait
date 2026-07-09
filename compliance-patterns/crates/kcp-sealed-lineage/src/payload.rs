//! On-chain payload codec for sealed-lineage events.
//!
//! ## Wire format
//!
//! ```text
//! Offset  Len  Field
//! ------  ---  -----
//!      0    5  magic        b"KCPSL"
//!      5    1  version      0x01
//!      6   32  lineage_id   SHA-256 of the canonical genesis identity body
//!     38    8  seq          event sequence number, u64 little-endian
//!     46    1  event_class  0x00=Genesis, 0x01=Append, 0x02=Close
//!     47    8  t_bucket     publisher-supplied seconds since Unix epoch, u64 LE
//!     55   32  commitment   sealed commitment to the off-chain record
//! ------  ---
//!             TOTAL: 87 bytes
//! ```
//!
//! The magic bytes `KCPSL` plus the version byte guard against accidentally
//! parsing an unrelated transaction payload as a sealed-lineage event.
//! Version `0x01` is this v0 codec; future versions increment this byte.

use crate::error::{Error, Result};

/// Magic prefix for sealed-lineage payloads.
pub const PAYLOAD_MAGIC: &[u8; 5] = b"KCPSL";

/// Codec version for this encoding.
pub const PAYLOAD_VERSION: u8 = 0x01;

/// Total wire length of an encoded payload.
pub const PAYLOAD_LEN: usize = 5 + 1 + 32 + 8 + 1 + 8 + 32; // 87

/// A decoded sealed-lineage payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// The lineage identifier (SHA-256 of the genesis identity body).
    pub lineage_id: [u8; 32],
    /// Event sequence number. Genesis has `seq = 0`; each append increments
    /// by exactly 1.
    pub seq: u64,
    /// Event class byte (`0x00` = Genesis, `0x01` = Append, `0x02` = Close).
    pub event_class: u8,
    /// Publisher-supplied timestamp: seconds since the Unix epoch (u64
    /// little-endian). Used for the temporal-envelope invariant (L-4) but
    /// not verified against wall-clock time by this library.
    pub t_bucket: u64,
    /// Sealed commitment to the off-chain record body.
    pub commitment: [u8; 32],
}

impl Payload {
    /// Encode the payload to the 87-byte wire format.
    ///
    /// Returns the raw bytes to embed in the Kaspa transaction payload field.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PAYLOAD_LEN);
        out.extend_from_slice(PAYLOAD_MAGIC);
        out.push(PAYLOAD_VERSION);
        out.extend_from_slice(&self.lineage_id);
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.push(self.event_class);
        out.extend_from_slice(&self.t_bucket.to_le_bytes());
        out.extend_from_slice(&self.commitment);
        debug_assert_eq!(out.len(), PAYLOAD_LEN);
        out
    }

    /// Decode a payload from raw bytes.
    ///
    /// # Errors
    ///
    /// - [`Error::PayloadBadLength`] if the byte slice is not exactly 87 bytes.
    /// - [`Error::PayloadBadMagic`] if the first 5 bytes are not `b"KCPSL"`.
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

        let mut lineage_id = [0u8; 32];
        lineage_id.copy_from_slice(&bytes[6..38]);

        let seq = u64::from_le_bytes(bytes[38..46].try_into().expect("slice length checked"));

        let event_class = bytes[46];

        let t_bucket = u64::from_le_bytes(bytes[47..55].try_into().expect("slice length checked"));

        let mut commitment = [0u8; 32];
        commitment.copy_from_slice(&bytes[55..87]);

        Ok(Self {
            lineage_id,
            seq,
            event_class,
            t_bucket,
            commitment,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload(seq: u64, event_class: u8) -> Payload {
        Payload {
            lineage_id: [0xaa; 32],
            seq,
            event_class,
            t_bucket: 1_700_000_000,
            commitment: [0xbb; 32],
        }
    }

    #[test]
    fn round_trip_genesis() {
        let p = sample_payload(0, 0x00);
        let encoded = p.encode();
        assert_eq!(encoded.len(), PAYLOAD_LEN);
        let decoded = Payload::decode(&encoded).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_append() {
        let p = sample_payload(1, 0x01);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_close() {
        let p = sample_payload(5, 0x02);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_seq_max() {
        let p = sample_payload(u64::MAX, 0x01);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.seq, u64::MAX);
    }

    #[test]
    fn encode_starts_with_magic_and_version() {
        let encoded = sample_payload(0, 0x00).encode();
        assert_eq!(&encoded[0..5], b"KCPSL");
        assert_eq!(encoded[5], 0x01);
    }

    #[test]
    fn fields_round_trip_distinct_values() {
        let p = Payload {
            lineage_id: [0x11; 32],
            seq: 42,
            event_class: 0x01,
            t_bucket: 9_999_999,
            commitment: [0x22; 32],
        };
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.lineage_id, [0x11; 32]);
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded.event_class, 0x01);
        assert_eq!(decoded.t_bucket, 9_999_999);
        assert_eq!(decoded.commitment, [0x22; 32]);
    }

    #[test]
    fn decode_wrong_length_short() {
        let err = Payload::decode(&[0u8; 86]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 87,
                    got: 86
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_wrong_length_long() {
        let err = Payload::decode(&[0u8; 88]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 87,
                    got: 88
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_bad_magic() {
        let mut bytes = sample_payload(0, 0x00).encode();
        bytes[0] = b'X';
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadMagic),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_bad_version() {
        let mut bytes = sample_payload(0, 0x00).encode();
        bytes[5] = 0x02;
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadVersion(0x02)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn corruption_in_commitment_detected_by_equality() {
        let p = sample_payload(1, 0x01);
        let mut bytes = p.encode();
        bytes[55] ^= 0xff; // flip first byte of commitment
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.commitment, p.commitment);
    }

    #[test]
    fn corruption_in_lineage_id_detected_by_equality() {
        let p = sample_payload(1, 0x01);
        let mut bytes = p.encode();
        bytes[6] ^= 0xff; // flip first byte of lineage_id
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.lineage_id, p.lineage_id);
    }
}
