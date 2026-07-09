//! Spending-condition types for vault covenants.
//!
//! A [`SpendCondition`] describes when and by whom a vault UTXO may be spent.
//! Conditions are pure data — they carry no on-chain state and impose no
//! runtime allocation beyond the heap cost of their fields.
//!
//! ## Leaf types
//!
//! - [`SpendCondition::TimelockHeight`] — spendable after a given DAA height.
//! - [`SpendCondition::TimelockUnixSeconds`] — spendable after a given
//!   Unix timestamp (seconds).
//! - [`SpendCondition::MultiSig`] — k-of-n Schnorr multisig.
//!
//! Both timelock leaves carry a `controller_xonly` field: the 32-byte x-only
//! Schnorr public key authorised to spend once the time barrier has passed.
//! This maps directly to the `<xonly_pubkey> OP_CHECKSIG` tail of the compiled
//! time-bar script.
//!
//! ## Composite types
//!
//! - [`SpendCondition::All`] — all sub-conditions must be satisfied (logical
//!   AND). Non-empty; max nesting depth [`MAX_DEPTH`].
//! - [`SpendCondition::Any`] — at least one sub-condition must be satisfied
//!   (logical OR). Non-empty; max nesting depth [`MAX_DEPTH`].
//!
//! ## Validation
//!
//! Call [`SpendCondition::validate`] before anchoring or compiling a
//! condition. The method returns a [`crate::error::Error::ConditionInvalid`]
//! error for every rule violation.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Maximum nesting depth for composite conditions ([`SpendCondition::All`] /
/// [`SpendCondition::Any`]).
///
/// A depth-1 condition is a leaf. A depth-2 condition is e.g.
/// `All([leaf, leaf])`. A depth-9 condition would exceed this limit and is
/// rejected by [`SpendCondition::validate`].
pub const MAX_DEPTH: usize = 8;

/// Maximum number of public keys in a [`SpendCondition::MultiSig`] condition.
pub const MAX_MULTISIG_KEYS: usize = 16;

/// A vault spending condition.
///
/// Serde derives are provided so conditions can be serialised to canonical
/// JSON for [`vault_id`](crate::payload) computation and off-chain storage.
///
/// All variants are serialised as `{ "type": "<variant_name>", … }` objects
/// so that the type field is unambiguous in canonical JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpendCondition {
    /// Spendable by `controller_xonly` after the chain reaches DAA height
    /// `deadline`.
    ///
    /// Compiles to: `<deadline> OP_CHECKLOCKTIMEVERIFY OP_DROP
    /// <controller_xonly> OP_CHECKSIG`
    TimelockHeight {
        /// DAA height at or above which spending is permitted.
        deadline: u64,
        /// 32-byte x-only Schnorr public key authorised to spend after the
        /// deadline.
        #[serde(
            serialize_with = "serialize_bytes32",
            deserialize_with = "deserialize_bytes32"
        )]
        controller_xonly: [u8; 32],
    },

    /// Spendable by `controller_xonly` after Unix time `deadline` (seconds
    /// since the epoch).
    ///
    /// Compiles to: `<deadline> OP_CHECKLOCKTIMEVERIFY OP_DROP
    /// <controller_xonly> OP_CHECKSIG`
    TimelockUnixSeconds {
        /// Unix timestamp (seconds) at or after which spending is permitted.
        deadline: u64,
        /// 32-byte x-only Schnorr public key authorised to spend after the
        /// deadline.
        #[serde(
            serialize_with = "serialize_bytes32",
            deserialize_with = "deserialize_bytes32"
        )]
        controller_xonly: [u8; 32],
    },

    /// k-of-n Schnorr multisig condition.
    ///
    /// Compiles to: `<threshold> <pk1> … <pkN> <n> OP_CHECKMULTISIG`
    MultiSig {
        /// Number of signatures required (1 ≤ `threshold` ≤ `xonly_keys.len()` ≤ 16).
        threshold: u8,
        /// Ordered list of 32-byte x-only Schnorr public keys. Must contain
        /// between 1 and [`MAX_MULTISIG_KEYS`] entries, no duplicates.
        #[serde(
            serialize_with = "serialize_vec_bytes32",
            deserialize_with = "deserialize_vec_bytes32"
        )]
        xonly_keys: Vec<[u8; 32]>,
    },

    /// All sub-conditions must be satisfied (logical AND).
    ///
    /// v0 script compilation is limited to leaves or `All { children: leaves }`
    /// — see [`crate::script`] for details. The pure evaluator supports
    /// arbitrary nesting.
    All {
        /// Non-empty list of sub-conditions; max nesting depth [`MAX_DEPTH`].
        children: Vec<SpendCondition>,
    },

    /// At least one sub-condition must be satisfied (logical OR).
    ///
    /// v0 script compilation supports `Any` of exactly 2 branches — see
    /// [`crate::script`] for details. The pure evaluator supports arbitrary
    /// nesting.
    Any {
        /// Non-empty list of sub-conditions; max nesting depth [`MAX_DEPTH`].
        children: Vec<SpendCondition>,
    },
}

impl SpendCondition {
    /// Validate the condition, returning an error if any rule is violated.
    ///
    /// Rules:
    /// - `TimelockHeight` / `TimelockUnixSeconds`: no structural constraints
    ///   beyond the type itself (any `u64` deadline and 32-byte key are valid).
    /// - `MultiSig`: `1 ≤ threshold ≤ keys.len() ≤ 16`; no duplicate keys.
    /// - `All` / `Any`: non-empty; max nesting depth [`MAX_DEPTH`]; each
    ///   sub-condition recursively valid.
    pub fn validate(&self) -> Result<()> {
        self.validate_at_depth(1)
    }

    fn validate_at_depth(&self, depth: usize) -> Result<()> {
        match self {
            SpendCondition::TimelockHeight { .. } | SpendCondition::TimelockUnixSeconds { .. } => {
                Ok(())
            }

            SpendCondition::MultiSig {
                threshold,
                xonly_keys,
            } => {
                let n = xonly_keys.len();
                if n == 0 {
                    return Err(Error::ConditionInvalid(
                        "MultiSig: xonly_keys must not be empty".into(),
                    ));
                }
                if n > MAX_MULTISIG_KEYS {
                    return Err(Error::ConditionInvalid(format!(
                        "MultiSig: xonly_keys.len() = {n} exceeds maximum {MAX_MULTISIG_KEYS}"
                    )));
                }
                let t = *threshold as usize;
                if t == 0 {
                    return Err(Error::ConditionInvalid(
                        "MultiSig: threshold must be at least 1".into(),
                    ));
                }
                if t > n {
                    return Err(Error::ConditionInvalid(format!(
                        "MultiSig: threshold {t} exceeds key count {n}"
                    )));
                }
                // Check for duplicate keys.
                for i in 0..xonly_keys.len() {
                    for j in (i + 1)..xonly_keys.len() {
                        if xonly_keys[i] == xonly_keys[j] {
                            return Err(Error::ConditionInvalid(format!(
                                "MultiSig: duplicate key at indices {i} and {j}"
                            )));
                        }
                    }
                }
                Ok(())
            }

            SpendCondition::All { children } | SpendCondition::Any { children } => {
                if children.is_empty() {
                    return Err(Error::ConditionInvalid(
                        "composite condition must not be empty".into(),
                    ));
                }
                if depth > MAX_DEPTH {
                    return Err(Error::ConditionInvalid(format!(
                        "condition nesting depth {depth} exceeds maximum {MAX_DEPTH}"
                    )));
                }
                for child in children {
                    child.validate_at_depth(depth + 1)?;
                }
                Ok(())
            }
        }
    }
}

// ── Serde helpers for [u8; 32] as hex strings ────────────────────────────────

fn serialize_bytes32<S>(bytes: &[u8; 32], s: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&hex::encode(bytes))
}

fn deserialize_bytes32<'de, D>(d: D) -> std::result::Result<[u8; 32], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
    bytes
        .try_into()
        .map_err(|_| serde::de::Error::custom("expected 32 bytes (64 hex chars)"))
}

fn serialize_vec_bytes32<S>(vec: &[[u8; 32]], s: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(vec.len()))?;
    for item in vec {
        seq.serialize_element(&hex::encode(item))?;
    }
    seq.end()
}

fn deserialize_vec_bytes32<'de, D>(d: D) -> std::result::Result<Vec<[u8; 32]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let strs: Vec<String> = Vec::deserialize(d)?;
    strs.into_iter()
        .map(|s| {
            let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
            bytes
                .try_into()
                .map_err(|_| serde::de::Error::custom("expected 32 bytes (64 hex chars)"))
        })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = seed;
        k
    }

    // ── TimelockHeight ───────────────────────────────────────────────────────

    #[test]
    fn timelock_height_valid() {
        let c = SpendCondition::TimelockHeight {
            deadline: 1_000_000,
            controller_xonly: key(1),
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn timelock_height_zero_deadline_valid() {
        // Zero is a valid u64 deadline (already past, but structurally valid).
        let c = SpendCondition::TimelockHeight {
            deadline: 0,
            controller_xonly: key(1),
        };
        assert!(c.validate().is_ok());
    }

    // ── TimelockUnixSeconds ──────────────────────────────────────────────────

    #[test]
    fn timelock_unix_seconds_valid() {
        let c = SpendCondition::TimelockUnixSeconds {
            deadline: 1_700_000_000,
            controller_xonly: key(2),
        };
        assert!(c.validate().is_ok());
    }

    // ── MultiSig valid ───────────────────────────────────────────────────────

    #[test]
    fn multisig_1_of_1_valid() {
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(1)],
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn multisig_2_of_3_valid() {
        let c = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![key(1), key(2), key(3)],
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn multisig_max_keys_valid() {
        let keys: Vec<[u8; 32]> = (0..MAX_MULTISIG_KEYS as u8).map(key).collect();
        let c = SpendCondition::MultiSig {
            threshold: MAX_MULTISIG_KEYS as u8,
            xonly_keys: keys,
        };
        assert!(c.validate().is_ok());
    }

    // ── MultiSig invalid ─────────────────────────────────────────────────────

    #[test]
    fn multisig_empty_keys_rejected() {
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![],
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn multisig_threshold_zero_rejected() {
        let c = SpendCondition::MultiSig {
            threshold: 0,
            xonly_keys: vec![key(1)],
        };
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("threshold must be at least 1"));
    }

    #[test]
    fn multisig_threshold_exceeds_keys_rejected() {
        let c = SpendCondition::MultiSig {
            threshold: 3,
            xonly_keys: vec![key(1), key(2)],
        };
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("threshold 3 exceeds key count 2"));
    }

    #[test]
    fn multisig_too_many_keys_rejected() {
        let keys: Vec<[u8; 32]> = (0..(MAX_MULTISIG_KEYS + 1) as u8).map(key).collect();
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: keys,
        };
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("exceeds maximum"));
    }

    #[test]
    fn multisig_duplicate_keys_rejected() {
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(1), key(2), key(1)], // key(1) duplicated
        };
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate key"));
    }

    // ── All / Any valid ──────────────────────────────────────────────────────

    #[test]
    fn all_of_leaves_valid() {
        let c = SpendCondition::All {
            children: vec![
                SpendCondition::TimelockHeight {
                    deadline: 100,
                    controller_xonly: key(1),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![key(2)],
                },
            ],
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn any_of_leaves_valid() {
        let c = SpendCondition::Any {
            children: vec![
                SpendCondition::TimelockUnixSeconds {
                    deadline: 1_700_000_000,
                    controller_xonly: key(1),
                },
                SpendCondition::MultiSig {
                    threshold: 2,
                    xonly_keys: vec![key(2), key(3)],
                },
            ],
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn nested_composite_at_max_depth_valid() {
        // Build: Any { children: [ All { children: [leaf, leaf] }, leaf ] }
        // depth 3, well within MAX_DEPTH
        let leaf = || SpendCondition::TimelockHeight {
            deadline: 1,
            controller_xonly: key(9),
        };
        let c = SpendCondition::Any {
            children: vec![
                SpendCondition::All {
                    children: vec![leaf(), leaf()],
                },
                leaf(),
            ],
        };
        assert!(c.validate().is_ok());
    }

    // ── All / Any invalid ────────────────────────────────────────────────────

    #[test]
    fn all_empty_rejected() {
        let c = SpendCondition::All { children: vec![] };
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn any_empty_rejected() {
        let c = SpendCondition::Any { children: vec![] };
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn depth_exceeds_max_rejected() {
        // Build a chain of MAX_DEPTH+1 nested Any wrappers around a leaf.
        let mut c = SpendCondition::TimelockHeight {
            deadline: 1,
            controller_xonly: key(1),
        };
        // Each wrapping adds 1 depth level. After MAX_DEPTH+1 wrappers the
        // outermost Any is at depth 1 but its innermost child is at depth
        // MAX_DEPTH+2, which exceeds the limit.
        for _ in 0..(MAX_DEPTH + 1) {
            c = SpendCondition::Any { children: vec![c] };
        }
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("nesting depth"), "error: {err}");
    }

    #[test]
    fn composite_with_invalid_child_rejected() {
        let c = SpendCondition::All {
            children: vec![SpendCondition::MultiSig {
                threshold: 5,
                xonly_keys: vec![key(1), key(2)], // threshold > keys
            }],
        };
        let err = c.validate().unwrap_err();
        assert!(err.to_string().contains("threshold"));
    }

    // ── Serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn serde_round_trip_timelock_height() {
        let c = SpendCondition::TimelockHeight {
            deadline: 474_165_565,
            controller_xonly: key(0xab),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: SpendCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn serde_round_trip_multisig() {
        let c = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![key(1), key(2), key(3)],
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: SpendCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn serde_round_trip_composite() {
        let c = SpendCondition::Any {
            children: vec![
                SpendCondition::TimelockUnixSeconds {
                    deadline: 1_700_000_000,
                    controller_xonly: key(5),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![key(6)],
                },
            ],
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: SpendCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
