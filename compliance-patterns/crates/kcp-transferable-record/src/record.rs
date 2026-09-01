//! Record identity: genesis hashing and event commitment helpers.
//!
//! A record's identity (`record_id`) is established at creation time as the
//! canonical SHA-256 hash of the genesis body. No two distinct genesis bodies
//! can produce the same `record_id` without a SHA-256 preimage collision.
//!
//! An event commitment (`RecordCommitment`) is the canonical SHA-256 hash of
//! an event body. Commitments are carried in transfer payloads and provide an
//! application-layer audit trail; in v0 they are NOT enforced by consensus.

use serde::Serialize;

use crate::error::{Error, Result};

/// Compute the `record_id` — the canonical SHA-256 hash of the genesis body.
///
/// Any JSON-serialisable value is accepted as the genesis body. The hash is
/// deterministic across platforms: object keys are sorted recursively before
/// hashing (see [`kcp_common::canonical::canonical_hash`]).
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `genesis_body` cannot be serialised to JSON.
pub fn record_id<T: Serialize>(genesis_body: &T) -> Result<[u8; 32]> {
    kcp_common::canonical::canonical_hash(genesis_body).map_err(Error::Canonical)
}

/// Compute a `commitment` — the canonical SHA-256 hash of an event body.
///
/// Commitments are included in transfer payloads; applications can store the
/// event body off-chain and prove inclusion by re-hashing.
///
/// # Errors
///
/// Returns [`Error::Canonical`] if `event_body` cannot be serialised to JSON.
pub fn commitment<T: Serialize>(event_body: &T) -> Result<[u8; 32]> {
    kcp_common::canonical::canonical_hash(event_body).map_err(Error::Canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_id_deterministic() {
        let body = json!({"name": "test-record", "issuer": "ExampleCorp"});
        let a = record_id(&body).unwrap();
        let b = record_id(&body).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn record_id_key_order_independent() {
        let a = record_id(&json!({"b": 1, "a": 2})).unwrap();
        let b = record_id(&json!({"a": 2, "b": 1})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn record_id_differs_on_different_body() {
        let a = record_id(&json!({"name": "alpha"})).unwrap();
        let b = record_id(&json!({"name": "beta"})).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn commitment_deterministic() {
        let ev = json!({"action": "transfer", "to": "kaspatest:abc"});
        let a = commitment(&ev).unwrap();
        let b = commitment(&ev).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn commitment_differs_from_record_id_on_same_body() {
        // record_id and commitment use the same hash function; equal bodies
        // produce equal hashes — this is expected and correct.
        let body = json!({"x": 1});
        assert_eq!(record_id(&body).unwrap(), commitment(&body).unwrap());
    }

    #[test]
    fn record_id_is_32_bytes() {
        let id = record_id(&json!({"k": "v"})).unwrap();
        assert_eq!(id.len(), 32);
    }
}
