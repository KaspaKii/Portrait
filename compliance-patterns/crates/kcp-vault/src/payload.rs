//! On-chain payload codec for vault evidence transactions.
//!
//! ## Wire format
//!
//! ```text
//! Offset  Len  Field
//! ------  ---  -----
//!      0    5  magic          b"KCPVT"
//!      5    1  version        0x01
//!      6   32  vault_id       canonical_hash(condition JSON)
//!     38   32  script_digest  KCP_SCRIPT_DIGEST_v1 hash of compiled script
//! ------  ---
//!             TOTAL: 70 bytes
//! ```
//!
//! The magic bytes `KCPVT` plus the version byte guard against accidentally
//! parsing an unrelated transaction payload as a vault evidence payload.
//! Version `0x01` is this v0 codec; future versions increment this byte.
//!
//! The `vault_id` is the [`kcp_common::canonical::canonical_hash`] of the
//! [`SpendCondition`](crate::condition::SpendCondition) serialised to
//! canonical JSON (sorted keys). This ties the on-chain record to the exact
//! condition that was compiled and anchored.
//!
//! The `script_digest` is computed by
//! [`kcp_common::digest::script_digest`] over the compiled script bytes,
//! providing a stable, domain-separated identifier for the covenant script.

use crate::error::{Error, Result};

/// Magic prefix for vault evidence payloads.
pub const PAYLOAD_MAGIC: &[u8; 5] = b"KCPVT";

/// Codec version for this encoding.
pub const PAYLOAD_VERSION: u8 = 0x01;

/// Total wire length of an encoded payload.
pub const PAYLOAD_LEN: usize = 5 + 1 + 32 + 32; // 70

/// A decoded vault evidence payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// The vault identifier: [`kcp_common::canonical::canonical_hash`] of the
    /// condition serialised to canonical JSON.
    pub vault_id: [u8; 32],
    /// Domain-separated SHA-256 digest of the compiled covenant script.
    /// Computed by [`kcp_common::digest::script_digest`].
    pub script_digest: [u8; 32],
}

impl Payload {
    /// Encode the payload to the 70-byte wire format.
    ///
    /// Returns the raw bytes to embed in the Kaspa transaction payload field.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PAYLOAD_LEN);
        out.extend_from_slice(PAYLOAD_MAGIC);
        out.push(PAYLOAD_VERSION);
        out.extend_from_slice(&self.vault_id);
        out.extend_from_slice(&self.script_digest);
        debug_assert_eq!(out.len(), PAYLOAD_LEN);
        out
    }

    /// Decode a payload from raw bytes.
    ///
    /// # Errors
    ///
    /// - [`Error::PayloadBadLength`] if the byte slice is not exactly 70 bytes.
    /// - [`Error::PayloadBadMagic`] if the first 5 bytes are not `b"KCPVT"`.
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

        let mut vault_id = [0u8; 32];
        vault_id.copy_from_slice(&bytes[6..38]);

        let mut script_digest = [0u8; 32];
        script_digest.copy_from_slice(&bytes[38..70]);

        Ok(Self {
            vault_id,
            script_digest,
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Payload {
        Payload {
            vault_id: [0xaa; 32],
            script_digest: [0xbb; 32],
        }
    }

    #[test]
    fn encode_length() {
        assert_eq!(sample().encode().len(), PAYLOAD_LEN);
    }

    #[test]
    fn round_trip() {
        let p = sample();
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn encode_starts_with_magic_and_version() {
        let encoded = sample().encode();
        assert_eq!(&encoded[0..5], b"KCPVT");
        assert_eq!(encoded[5], 0x01);
    }

    #[test]
    fn fields_round_trip_distinct_values() {
        let p = Payload {
            vault_id: [0x11; 32],
            script_digest: [0x22; 32],
        };
        let decoded = Payload::decode(&p.encode()).unwrap();
        assert_eq!(decoded.vault_id, [0x11; 32]);
        assert_eq!(decoded.script_digest, [0x22; 32]);
    }

    #[test]
    fn decode_wrong_length_short() {
        let err = Payload::decode(&[0u8; 69]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 70,
                    got: 69
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_wrong_length_long() {
        let err = Payload::decode(&[0u8; 71]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::PayloadBadLength {
                    expected: 70,
                    got: 71
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn decode_bad_magic() {
        let mut bytes = sample().encode();
        bytes[0] = b'X';
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(matches!(err, Error::PayloadBadMagic), "unexpected: {err}");
    }

    #[test]
    fn decode_bad_version() {
        let mut bytes = sample().encode();
        bytes[5] = 0x02;
        let err = Payload::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::PayloadBadVersion(0x02)),
            "unexpected: {err}"
        );
    }

    #[test]
    fn corruption_in_vault_id_detected() {
        let p = sample();
        let mut bytes = p.encode();
        bytes[6] ^= 0xff;
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.vault_id, p.vault_id);
    }

    #[test]
    fn corruption_in_script_digest_detected() {
        let p = sample();
        let mut bytes = p.encode();
        bytes[38] ^= 0xff;
        let decoded = Payload::decode(&bytes).unwrap();
        assert_ne!(decoded.script_digest, p.script_digest);
    }
}
