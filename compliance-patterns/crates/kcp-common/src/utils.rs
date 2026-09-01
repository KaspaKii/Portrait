//! Utility helpers: unit conversion, canonical encoding, hash convenience.

use sha2::{Digest, Sha256};

// 1 KAS = 100_000_000 sompi (100 million). Use integer arithmetic to avoid float rounding.
const SOMPI_PER_KAS: u64 = 100_000_000;

/// Convert a KAS amount (as a floating-point value) to sompi.
///
/// Precision is truncated at 8 decimal places (1 sompi). Use with care:
/// floating-point representation may introduce ±1 sompi rounding.
pub fn kas_to_sompi(kas: f64) -> u64 {
    (kas * SOMPI_PER_KAS as f64) as u64
}

/// Convert sompi to KAS.
pub fn sompi_to_kas(sompi: u64) -> f64 {
    sompi as f64 / SOMPI_PER_KAS as f64
}

/// Encode a `u64` as 8 bytes, little-endian.
pub fn encode_u64_le(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

/// Decode 8 bytes (little-endian) to a `u64`.
pub fn decode_u64_le(b: &[u8; 8]) -> u64 {
    u64::from_le_bytes(*b)
}

/// Return the first 32 bytes of `data`, zero-padding if `data.len() < 32`.
/// Truncates if `data.len() > 32`.
pub fn to_bytes32(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = data.len().min(32);
    out[..n].copy_from_slice(&data[..n]);
    out
}

/// Compute SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Compute double SHA-256 (SHA-256d) of `data`.
pub fn sha256d(data: &[u8]) -> [u8; 32] {
    sha256(&sha256(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kas_to_sompi_one_kas() {
        assert_eq!(kas_to_sompi(1.0), 100_000_000);
    }

    #[test]
    fn kas_to_sompi_half_kas() {
        assert_eq!(kas_to_sompi(0.5), 50_000_000);
    }

    #[test]
    fn sompi_to_kas_round_trip() {
        assert_eq!(sompi_to_kas(100_000_000), 1.0);
    }

    #[test]
    fn encode_u64_le_known_value() {
        assert_eq!(
            encode_u64_le(0x0102_0304_0506_0708),
            [8, 7, 6, 5, 4, 3, 2, 1]
        );
    }

    #[test]
    fn decode_u64_le_known_value() {
        assert_eq!(
            decode_u64_le(&[8, 7, 6, 5, 4, 3, 2, 1]),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn to_bytes32_short_input_zero_padded() {
        let result = to_bytes32(&[0u8; 10]);
        assert_eq!(result.len(), 32);
        assert_eq!(&result[..10], &[0u8; 10]);
        assert_eq!(&result[10..], &[0u8; 22]);
    }

    #[test]
    fn to_bytes32_long_input_truncated() {
        let result = to_bytes32(&[1u8; 40]);
        assert_eq!(result, [1u8; 32]);
    }

    #[test]
    fn sha256_is_deterministic() {
        assert_eq!(sha256(b"hello"), sha256(b"hello"));
    }

    #[test]
    fn sha256d_differs_from_sha256() {
        assert_ne!(sha256d(b"hello"), sha256(b"hello"));
    }
}
