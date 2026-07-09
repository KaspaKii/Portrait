//! Stable script digests.
//!
//! A script digest is the SHA-256 of a domain-separation tag plus the raw
//! script bytes, used as a stable identifier for a covenant script across
//! this library. The tag is library-specific (`KCP_SCRIPT_DIGEST_v1`): these
//! digests deliberately do NOT collide with identifiers minted by donor
//! codebases that use the same construction under their own tags.

use sha2::{Digest, Sha256};

/// Domain-separation tag for [`script_digest`]. Bump the suffix on any
/// breaking change to the digest construction.
pub const SCRIPT_DIGEST_TAG: &[u8] = b"KCP_SCRIPT_DIGEST_v1\n";

/// SHA-256 of the tagged script bytes — a stable covenant-script identifier.
pub fn script_digest(script: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(SCRIPT_DIGEST_TAG);
    h.update(script);
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_deterministic() {
        assert_eq!(script_digest(b"abc"), script_digest(b"abc"));
    }

    #[test]
    fn digest_differs_on_input() {
        assert_ne!(script_digest(b"abc"), script_digest(b"abd"));
    }

    #[test]
    fn digest_is_tag_separated() {
        // The tag must actually participate: digest(x) != sha256(x).
        let mut h = Sha256::new();
        h.update(b"abc");
        let plain: [u8; 32] = h.finalize().into();
        assert_ne!(script_digest(b"abc"), plain);
    }
}
