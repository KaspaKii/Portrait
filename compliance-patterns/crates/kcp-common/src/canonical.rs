//! Deterministic JSON canonicalisation and hashing.
//!
//! Canonical form: JSON with object keys sorted recursively. Numbers are
//! preserved as-is from the input; callers should pre-format decimals as
//! strings if exact decimal representation matters.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Errors from canonicalisation.
#[derive(Debug, Error)]
pub enum CanonicalError {
    /// The value could not be serialised to JSON.
    #[error("serialisation: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Produce a canonical JSON byte string with sorted object keys.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let v = serde_json::to_value(value)?;
    let sorted = sort_value(v);
    Ok(serde_json::to_vec(&sorted)?)
}

/// SHA-256 of the canonical JSON byte string. Returns 32 raw bytes.
pub fn canonical_hash<T: Serialize>(value: &T) -> Result<[u8; 32], CanonicalError> {
    let bytes = canonical_json(value)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

fn sort_value(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> =
                map.into_iter().map(|(k, v)| (k, sort_value(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sort_value).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_sorted_deterministic() {
        let a = canonical_json(&json!({"b": 1, "a": 2})).unwrap();
        let b = canonical_json(&json!({"a": 2, "b": 1})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn nested_keys_sorted_deterministic() {
        let a = canonical_json(&json!({"z": {"b": 1, "a": [{"d": 4, "c": 3}]}})).unwrap();
        let b = canonical_json(&json!({"z": {"a": [{"c": 3, "d": 4}], "b": 1}})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn hash_deterministic() {
        let a = canonical_hash(&json!({"x": 1, "y": [3, 2, 1]})).unwrap();
        let b = canonical_hash(&json!({"y": [3, 2, 1], "x": 1})).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn hash_differs_on_different_value() {
        let a = canonical_hash(&json!({"x": 1})).unwrap();
        let b = canonical_hash(&json!({"x": 2})).unwrap();
        assert_ne!(a, b);
    }
}
