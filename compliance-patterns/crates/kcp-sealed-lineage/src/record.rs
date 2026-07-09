//! Lineage identity and sealed-commitment helpers.
//!
//! ## Lineage identity
//!
//! A lineage's identity (`lineage_id`) is established at genesis as the
//! canonical SHA-256 hash of the genesis identity body:
//!
//! ```text
//! lineage_id = SHA-256(canonical_json(genesis_identity_body))
//! ```
//!
//! This uses [`kcp_common::canonical::canonical_hash`], which sorts object
//! keys recursively before hashing, so key insertion order does not affect
//! the identity.
//!
//! ## Sealed commitment construction
//!
//! A sealed commitment is a blinded commitment to an off-chain record body:
//!
//! ```text
//! commitment = SHA-256(canonical_json(record_body) || blind)
//! ```
//!
//! where `blind` is a 32-byte random value supplied by the publisher and kept
//! off-chain. The construction is:
//!
//! 1. Serialise `record_body` to canonical JSON bytes
//!    (`canonical_json(record_body)`).
//! 2. Concatenate the 32-byte blind directly after the JSON bytes.
//! 3. SHA-256-hash the concatenation.
//!
//! The blind ensures that an observer who does not know the record body cannot
//! enumerate plausible bodies and verify a match against the on-chain
//! commitment.
//!
//! ### SHA-256 vs. Poseidon divergence
//!
//! The donor lineage system (Kii SCL) uses a BN254 Poseidon commitment
//! (ZK-friendly). v0 of this pattern deliberately uses SHA-256 — it is
//! simpler, requires no elliptic-curve dependencies, and can be reviewed by
//! any cryptographer without ZK tooling. The Poseidon upgrade path is tied to
//! future ZK-circuit work scheduled for the KIP-16 era; it is out of scope
//! for v0.
//!
//! ## Status
//!
//! **v0 — unaudited.** Commitments are not enforced by consensus in v0;
//! they are application-layer evidence only.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Compute the `lineage_id` — the canonical SHA-256 hash of the genesis
/// identity body.
///
/// Any JSON-serialisable value is accepted. The hash is deterministic across
/// platforms because object keys are sorted recursively before hashing.
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `genesis_identity_body` cannot be
/// serialised to JSON.
pub fn lineage_id<T: Serialize>(genesis_identity_body: &T) -> Result<[u8; 32]> {
    kcp_common::canonical::canonical_hash(genesis_identity_body).map_err(Error::Canonical)
}

/// Compute the canonical JSON bytes for an arbitrary serialisable body.
///
/// Helper exposed so callers can inspect or log the exact bytes that feed
/// into a commitment without re-implementing canonicalisation.
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `body` cannot be serialised to JSON.
pub fn canonical_bytes<T: Serialize>(body: &T) -> Result<Vec<u8>> {
    kcp_common::canonical::canonical_json(body).map_err(Error::Canonical)
}

/// Compute a sealed commitment over a record body and a 32-byte blind.
///
/// Construction:
/// ```text
/// commitment = SHA-256(canonical_json(record_body) || blind)
/// ```
///
/// 1. `record_body` is serialised to canonical JSON bytes.
/// 2. The 32-byte `blind` is appended directly.
/// 3. SHA-256 is computed over the concatenation.
///
/// The `blind` must be kept secret by the publisher; it should be generated
/// from a cryptographically secure RNG. Without the blind an observer cannot
/// verify a hypothesised body against the on-chain commitment.
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `record_body` cannot be serialised to JSON.
pub fn commitment<T: Serialize>(record_body: &T, blind: &[u8; 32]) -> Result<[u8; 32]> {
    let json_bytes =
        kcp_common::canonical::canonical_json(record_body).map_err(Error::Canonical)?;
    let mut hasher = Sha256::new();
    hasher.update(&json_bytes);
    hasher.update(blind);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ZERO_BLIND: [u8; 32] = [0u8; 32];
    const ONE_BLIND: [u8; 32] = [1u8; 32];

    // ---- lineage_id ----

    #[test]
    fn lineage_id_deterministic() {
        let body = json!({"name": "kcp-sl-evidence", "issuer": "ExampleCorp"});
        let a = lineage_id(&body).unwrap();
        let b = lineage_id(&body).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn lineage_id_key_order_independent() {
        let a = lineage_id(&json!({"b": 1, "a": 2})).unwrap();
        let b = lineage_id(&json!({"a": 2, "b": 1})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn lineage_id_differs_on_different_body() {
        let a = lineage_id(&json!({"name": "alpha"})).unwrap();
        let b = lineage_id(&json!({"name": "beta"})).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn lineage_id_is_32_bytes() {
        let id = lineage_id(&json!({"k": "v"})).unwrap();
        assert_eq!(id.len(), 32);
    }

    // ---- commitment ----

    #[test]
    fn commitment_deterministic() {
        let body = json!({"action": "append", "ref": "doc-42"});
        let a = commitment(&body, &ZERO_BLIND).unwrap();
        let b = commitment(&body, &ZERO_BLIND).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn commitment_different_blind_differs() {
        let body = json!({"action": "append"});
        let a = commitment(&body, &ZERO_BLIND).unwrap();
        let b = commitment(&body, &ONE_BLIND).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn commitment_different_body_differs() {
        let a = commitment(&json!({"x": 1}), &ZERO_BLIND).unwrap();
        let b = commitment(&json!({"x": 2}), &ZERO_BLIND).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn commitment_key_order_independent() {
        let a = commitment(&json!({"b": 1, "a": 2}), &ONE_BLIND).unwrap();
        let b = commitment(&json!({"a": 2, "b": 1}), &ONE_BLIND).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn commitment_differs_from_unblinded_hash() {
        // The commitment is SHA-256(json || blind), NOT SHA-256(json).
        // Even with a zero blind the two are different because the blind bytes
        // are appended to the JSON before hashing.
        let body = json!({"k": "v"});
        let blinded = commitment(&body, &ZERO_BLIND).unwrap();
        let unblinded = lineage_id(&body).unwrap(); // SHA-256(json) only
        assert_ne!(blinded, unblinded);
    }

    #[test]
    fn commitment_is_32_bytes() {
        let c = commitment(&json!({"k": "v"}), &ZERO_BLIND).unwrap();
        assert_eq!(c.len(), 32);
    }

    // ---- canonical_bytes ----

    #[test]
    fn canonical_bytes_key_order_independent() {
        let a = canonical_bytes(&json!({"b": 1, "a": 2})).unwrap();
        let b = canonical_bytes(&json!({"a": 2, "b": 1})).unwrap();
        assert_eq!(a, b);
    }
}
