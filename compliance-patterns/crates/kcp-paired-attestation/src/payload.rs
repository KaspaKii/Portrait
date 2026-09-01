//! On-chain payload codec for paired-attestation events.
//!
//! ## Wire format
//!
//! ```text
//! Offset  Len  Field
//! ------  ---  -----
//!      0    5  magic           b"KCPPA"
//!      5    1  version         0x01
//!      6   32  attestation_id  SHA-256 of the canonical attestation record
//!     38    8  seq             event sequence number, u64 little-endian
//!     46    1  event_class     0x00=PartyACommit, 0x01=PartyBMate, 0x02=Close
//!     47   32  commitment      blinded SHA-256 commitment of the committing party
//! ------  ---
//!             TOTAL: 79 bytes
//! ```
//!
//! The magic bytes `KCPPA` plus the version byte guard against accidentally
//! parsing an unrelated transaction payload as a paired-attestation event.
//! Version `0x01` is the v0 codec; future versions increment this byte.
//!
//! The `commitment` field carries:
//!
//! - at `seq = 0` (`PartyACommit`): Party A's commitment to the shared record
//!   under Party A's blind.
//! - at `seq = 1` (`PartyBMate`): Party B's commitment to the same record
//!   under Party B's blind (the mate proof is verified off-chain before
//!   anchoring).
//! - at `seq = 2` (`Close`): a terminal commitment indicating the attestation
//!   lineage is closed.

use crate::error::{Error, Result};

/// Magic prefix for paired-attestation payloads.
pub const PAYLOAD_MAGIC: &[u8; 5] = b"KCPPA";

/// Codec version for this encoding.
pub const PAYLOAD_VERSION: u8 = 0x01;

/// Total wire length of an encoded payload (bytes).
///
/// `5 + 1 + 32 + 8 + 1 + 32 = 79`
pub const PAYLOAD_LEN: usize = 5 + 1 + 32 + 8 + 1 + 32;

/// A decoded paired-attestation payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// The attestation identifier (SHA-256 of the canonical attestation record).
    pub attestation_id: [u8; 32],
    /// Event sequence number.
    ///
    /// `PartyACommit` has `seq = 0`; `PartyBMate` has `seq = 1`.
    pub seq: u64,
    /// Event class byte.
    ///
    /// `0x00` = `PartyACommit`, `0x01` = `PartyBMate`, `0x02` = `Close`.
    pub event_class: u8,
    /// Blinded SHA-256 commitment of the committing party for this step.
    pub commitment: [u8; 32],
}

impl Payload {
    /// Encode the payload to the 79-byte wire format.
    ///
    /// Returns the raw bytes to embed in the Kaspa transaction payload field.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PAYLOAD_LEN);
        out.extend_from_slice(PAYLOAD_MAGIC);
        out.push(PAYLOAD_VERSION);
        out.extend_from_slice(&self.attestation_id);
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.push(self.event_class);
        out.extend_from_slice(&self.commitment);
        debug_assert_eq!(out.len(), PAYLOAD_LEN);
        out
    }

    /// Decode a payload from raw bytes.
    ///
    /// # Errors
    ///
    /// - [`Error::PayloadBadLength`] if the byte slice is not exactly
    ///   [`PAYLOAD_LEN`] bytes.
    /// - [`Error::PayloadBadMagic`] if the first 5 bytes are not `b"KCPPA"`.
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

        let mut attestation_id = [0u8; 32];
        attestation_id.copy_from_slice(&bytes[6..38]);

        let seq = u64::from_le_bytes(bytes[38..46].try_into().expect("slice length checked"));

        let event_class = bytes[46];

        let mut commitment = [0u8; 32];
        commitment.copy_from_slice(&bytes[47..79]);

        Ok(Self {
            attestation_id,
            seq,
            event_class,
            commitment,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: u64, event_class: u8) -> Payload {
        Payload {
            attestation_id: [0xaau8; 32],
            seq,
            event_class,
            commitment: [0xbbu8; 32],
        }
    }

    #[test]
    fn payload_len_is_79() {
        assert_eq!(PAYLOAD_LEN, 79);
    }

    #[test]
    fn round_trip_party_a_commit() {
        let p = sample(0, 0x00);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_party_b_mate() {
        let p = sample(1, 0x01);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_close() {
        let p = sample(2, 0x02);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn round_trip_seq_max() {
        let p = sample(u64::MAX, 0x01);
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.seq, u64::MAX);
    }

    #[test]
    fn encode_starts_with_magic_and_version() {
        let encoded = sample(0, 0x00).encode();
        assert_eq!(&encoded[0..5], b"KCPPA");
        assert_eq!(encoded[5], 0x01);
    }

    #[test]
    fn fields_round_trip_distinct_values() {
        let p = Payload {
            attestation_id: [0x11u8; 32],
            seq: 42,
            event_class: 0x01,
            commitment: [0x22u8; 32],
        };
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.attestation_id, [0x11u8; 32]);
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded.event_class, 0x01);
        assert_eq!(decoded.commitment, [0x22u8; 32]);
    }

    #[test]
    fn decode_wrong_length_short() {
        let err = Payload::decode(&[0u8; 78]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: _,
                    got: 78
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_wrong_length_long() {
        let err = Payload::decode(&[0u8; 80]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: _,
                    got: 80
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_bad_magic() {
        let mut bytes = sample(0, 0x00).encode();
        bytes[0] = b'X';
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadMagic),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_bad_version() {
        let mut bytes = sample(0, 0x00).encode();
        bytes[5] = 0x02;
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadVersion(0x02)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn corruption_in_commitment_detected_by_equality() {
        let p = sample(1, 0x01);
        let mut bytes = p.encode();
        bytes[47] ^= 0xff; // flip first byte of commitment
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.commitment, p.commitment);
    }

    #[test]
    fn corruption_in_attestation_id_detected_by_equality() {
        let p = sample(1, 0x01);
        let mut bytes = p.encode();
        bytes[6] ^= 0xff; // flip first byte of attestation_id
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.attestation_id, p.attestation_id);
    }
}
