//! Attestation record and commitment helpers.
//!
//! ## Attestation identity
//!
//! An attestation's identity (`attestation_id`) is the canonical SHA-256 hash
//! of the [`AttestationRecord`]:
//!
//! ```text
//! attestation_id = SHA-256(canonical_json(record))
//! ```
//!
//! This uses [`kcp_common::canonical::canonical_hash`], which sorts object
//! keys recursively before hashing, so key insertion order does not affect
//! the identity.
//!
//! ## Commitment construction
//!
//! A commitment is a blinded commitment to an attestation record:
//!
//! ```text
//! commitment = SHA-256(canonical_json(record) || blind)
//! ```
//!
//! where `blind` is a 32-byte value supplied by the committing party and kept
//! off-chain. The construction mirrors `kcp-sealed-lineage` exactly:
//!
//! 1. Serialise `record` to canonical JSON bytes (`canonical_json(record)`).
//! 2. Concatenate the 32-byte blind directly after the JSON bytes.
//! 3. SHA-256-hash the concatenation.
//!
//! The blind ensures that an observer who does not know the record cannot
//! enumerate plausible records and verify a match against the commitment.
//! In v0, each party chooses their own blind (see [`crate::mate`] for the
//! XOR-share negotiation that prevents unilateral blind selection).
//!
//! ### SHA-256 vs. Poseidon divergence
//!
//! The donor system (Kii PLA) uses a BN254 Poseidon commitment (ZK-friendly).
//! v0 deliberately uses SHA-256 — simpler, no elliptic-curve dependencies,
//! reviewable without ZK tooling. The Poseidon upgrade is tied to KIP-16-era
//! ZK work and is out of scope for v0.
//!
//! ## Status
//!
//! **v0 — unaudited.** Commitments are not enforced by consensus in v0;
//! they are application-layer evidence only.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// A generic two-party attestation record.
///
/// The record is intentionally generic: `subject` and `terms_hash` are opaque
/// 32-byte arrays, and `nonce` is a caller-supplied u64 that prevents replay
/// across separate attestation sessions. Callers may embed richer semantics in
/// the `subject` and `terms_hash` fields (e.g. document hashes, contract ids)
/// without altering the mating or commitment mechanics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationRecord {
    /// Opaque 32-byte subject identifier (e.g. a document hash, entity id).
    ///
    /// Serialised as a lowercase hex string for canonical JSON stability.
    pub subject: String,
    /// Opaque 32-byte hash of the agreed terms (e.g. SHA-256 of a contract).
    ///
    /// Serialised as a lowercase hex string for canonical JSON stability.
    pub terms_hash: String,
    /// Caller-supplied nonce (u64) to distinguish separate attestation sessions
    /// over the same subject and terms.
    pub nonce: u64,
}

impl AttestationRecord {
    /// Construct a record from raw bytes. `subject` and `terms_hash` are
    /// hex-encoded for canonical JSON stability.
    pub fn new(subject: [u8; 32], terms_hash: [u8; 32], nonce: u64) -> Self {
        Self {
            subject: hex::encode(subject),
            terms_hash: hex::encode(terms_hash),
            nonce,
        }
    }
}

/// Compute the `attestation_id` — the canonical SHA-256 hash of the record.
///
/// Any JSON-serialisable value is accepted; object keys are sorted recursively
/// before hashing, so insertion order does not affect the result.
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `record` cannot be serialised to JSON.
pub fn attestation_id<T: Serialize>(record: &T) -> Result<[u8; 32]> {
    kcp_common::canonical::canonical_hash(record).map_err(Error::Canonical)
}

/// Compute the canonical JSON bytes for an arbitrary serialisable value.
///
/// Helper exposed so callers can inspect or log the exact bytes that feed into
/// a commitment without re-implementing canonicalisation.
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `value` cannot be serialised to JSON.
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    kcp_common::canonical::canonical_json(value).map_err(Error::Canonical)
}

/// Compute a blinded commitment over a record and a 32-byte blind.
///
/// Construction:
/// ```text
/// commitment = SHA-256(canonical_json(record) || blind)
/// ```
///
/// 1. `record` is serialised to canonical JSON bytes.
/// 2. The 32-byte `blind` is appended directly.
/// 3. SHA-256 is computed over the concatenation.
///
/// Each party supplies their own blind; the [`crate::mate`] module handles the
/// XOR-share negotiation that makes the combined blind unpredictable to either
/// party individually. The blind must be kept off-chain; without it an observer
/// cannot verify a hypothesised record against the commitment.
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `record` cannot be serialised to JSON.
pub fn commit<T: Serialize>(record: &T, blind: &[u8; 32]) -> Result<[u8; 32]> {
    let json_bytes = kcp_common::canonical::canonical_json(record).map_err(Error::Canonical)?;
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

    // ---- attestation_id ----

    #[test]
    fn attestation_id_deterministic() {
        let body = json!({"subject": "aabb", "terms_hash": "ccdd", "nonce": 1});
        let a = attestation_id(&body).unwrap();
        let b = attestation_id(&body).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn attestation_id_key_order_independent() {
        let a = attestation_id(&json!({"b": 1, "a": 2})).unwrap();
        let b = attestation_id(&json!({"a": 2, "b": 1})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn attestation_id_differs_on_different_record() {
        let a = attestation_id(&json!({"nonce": 1})).unwrap();
        let b = attestation_id(&json!({"nonce": 2})).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn attestation_id_is_32_bytes() {
        let id = attestation_id(&json!({"k": "v"})).unwrap();
        assert_eq!(id.len(), 32);
    }

    // ---- commit ----

    #[test]
    fn commit_deterministic() {
        let body = json!({"subject": "aabb", "terms_hash": "ccdd", "nonce": 7});
        let a = commit(&body, &ZERO_BLIND).unwrap();
        let b = commit(&body, &ZERO_BLIND).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn commit_different_blind_differs() {
        let body = json!({"nonce": 1});
        let a = commit(&body, &ZERO_BLIND).unwrap();
        let b = commit(&body, &ONE_BLIND).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn commit_different_record_differs() {
        let a = commit(&json!({"nonce": 1}), &ZERO_BLIND).unwrap();
        let b = commit(&json!({"nonce": 2}), &ZERO_BLIND).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn commit_key_order_independent() {
        let a = commit(&json!({"b": 1, "a": 2}), &ONE_BLIND).unwrap();
        let b = commit(&json!({"a": 2, "b": 1}), &ONE_BLIND).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn commit_is_32_bytes() {
        let c = commit(&json!({"k": "v"}), &ZERO_BLIND).unwrap();
        assert_eq!(c.len(), 32);
    }

    #[test]
    fn commit_differs_from_unblinded_hash() {
        // commit = SHA-256(json || blind), not SHA-256(json).
        // Even with a zero blind the two differ because blind bytes are appended.
        let body = json!({"k": "v"});
        let blinded = commit(&body, &ZERO_BLIND).unwrap();
        let unblinded = attestation_id(&body).unwrap();
        assert_ne!(blinded, unblinded);
    }

    // ---- AttestationRecord ----

    #[test]
    fn record_new_round_trips_hex() {
        let subject = [0xabu8; 32];
        let terms = [0xcdu8; 32];
        let rec = AttestationRecord::new(subject, terms, 42);
        assert_eq!(rec.subject, hex::encode(subject));
        assert_eq!(rec.terms_hash, hex::encode(terms));
        assert_eq!(rec.nonce, 42);
    }

    #[test]
    fn record_attestation_id_deterministic() {
        let rec = AttestationRecord::new([1u8; 32], [2u8; 32], 1);
        let a = attestation_id(&rec).unwrap();
        let b = attestation_id(&rec).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn record_commit_deterministic() {
        let rec = AttestationRecord::new([1u8; 32], [2u8; 32], 1);
        let a = commit(&rec, &ZERO_BLIND).unwrap();
        let b = commit(&rec, &ZERO_BLIND).unwrap();
        assert_eq!(a, b);
    }
}
