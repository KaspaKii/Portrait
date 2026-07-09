//! Covenant script compilation for vault spending conditions.
//!
//! [`compile_condition`] turns a [`SpendCondition`] into real Kaspa script
//! bytes using [`kaspa_txscript::script_builder::ScriptBuilder`] and the
//! Toccata opcode set.
//!
//! [`vault_script_digest`] wraps [`kcp_common::digest::script_digest`] for
//! convenience.
//!
//! ## Provenance
//!
//! Script mechanics are derived from the Kii Kastract codebase
//! (`kastract-covenant/src/scripts.rs`), same author, relicensed MIT under
//! the IP grant recorded 2026-06-11.
//!
//! ## v0 compilation limits
//!
//! v0 compilation is limited to:
//!
//! - **Leaf conditions**: `TimelockHeight`, `TimelockUnixSeconds`, `MultiSig`.
//! - **`All(leaves)`**: all children must be leaf conditions. Compiles to
//!   sequential `OP_VERIFY` checks: each leaf except the last is wrapped in
//!   `OP_VERIFY`, the last is left on the stack for the script result.
//! - **`Any` of exactly 2 branches**: each branch may be a leaf or
//!   `All(leaves)`. Compiles using `OP_IF` / `OP_ELSE` / `OP_ENDIF`, mirroring
//!   the payment-escrow branch mechanics from the Kastract donor.
//!
//! Conditions outside these shapes return
//! [`Error::CompileUnsupported`](crate::error::Error::CompileUnsupported).
//!
//! The **pure evaluator** ([`crate::evaluator`]) supports full nesting
//! regardless of compilation limits.
//!
//! ## Compiled script shapes
//!
//! ### TimelockHeight / TimelockUnixSeconds
//!
//! ```text
//! <deadline i64> OP_CHECKLOCKTIMEVERIFY OP_DROP <controller_xonly 32 bytes> OP_CHECKSIG
//! ```
//!
//! ### MultiSig (k-of-n)
//!
//! ```text
//! <threshold i64> <pk1> … <pkN> <n i64> OP_CHECKMULTISIG
//! ```
//!
//! ### Any(branch_a, branch_b)
//!
//! ```text
//! OP_IF
//!     <compiled branch_a>
//! OP_ELSE
//!     <compiled branch_b>
//! OP_ENDIF
//! ```
//!
//! The spending witness selects the branch with `OP_1` (branch_a) or `OP_0`
//! (branch_b), mirroring the Kastract payment-escrow builder.

use kaspa_txscript::opcodes::codes::{
    OpCheckLockTimeVerify, OpCheckMultiSig, OpCheckSig, OpDrop, OpElse, OpEndIf, OpIf, OpVerify,
};
use kaspa_txscript::script_builder::ScriptBuilder;

use kcp_common::digest::script_digest as kcp_script_digest;

use crate::condition::SpendCondition;
use crate::error::{Error, Result};

/// Compile a spending condition into raw Kaspa script bytes.
///
/// # v0 limitations
///
/// Only leaf conditions, `All(leaves)`, and `Any` of exactly 2 branches are
/// supported. Return [`Error::CompileUnsupported`] for anything else.
///
/// # Errors
///
/// - [`Error::CompileUnsupported`] for unsupported composite shapes.
/// - [`Error::ScriptBuilder`] if the opcode-level builder rejects the input
///   (e.g. an integer value out of the script-integer range).
pub fn compile_condition(condition: &SpendCondition) -> Result<Vec<u8>> {
    match condition {
        SpendCondition::TimelockHeight {
            deadline,
            controller_xonly,
        }
        | SpendCondition::TimelockUnixSeconds {
            deadline,
            controller_xonly,
        } => compile_timelock(*deadline, controller_xonly),

        SpendCondition::MultiSig {
            threshold,
            xonly_keys,
        } => compile_multisig(*threshold, xonly_keys),

        SpendCondition::All { children } => compile_all_leaves(children),

        SpendCondition::Any { children } => {
            if children.len() != 2 {
                return Err(Error::CompileUnsupported(format!(
                    "Any with {} branches: v0 compilation supports Any of exactly 2 branches",
                    children.len()
                )));
            }
            compile_any_two(&children[0], &children[1])
        }
    }
}

/// Compile a timelock (height or unix-seconds) leaf condition.
///
/// Output: `<deadline i64> OP_CHECKLOCKTIMEVERIFY OP_DROP <controller_xonly> OP_CHECKSIG`
fn compile_timelock(deadline: u64, controller_xonly: &[u8; 32]) -> Result<Vec<u8>> {
    let mut b = ScriptBuilder::new();
    b.add_i64(deadline as i64)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        .add_op(OpCheckLockTimeVerify)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        .add_op(OpDrop)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        .add_data(controller_xonly)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        .add_op(OpCheckSig)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    Ok(b.drain())
}

/// Compile a k-of-n multisig leaf condition.
///
/// Output: `<threshold i64> <pk1> … <pkN> <n i64> OP_CHECKMULTISIG`
fn compile_multisig(threshold: u8, xonly_keys: &[[u8; 32]]) -> Result<Vec<u8>> {
    let mut b = ScriptBuilder::new();
    b.add_i64(threshold as i64)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    for pk in xonly_keys {
        b.add_data(pk.as_ref())
            .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    }
    b.add_i64(xonly_keys.len() as i64)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        .add_op(OpCheckMultiSig)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    Ok(b.drain())
}

/// Check that a condition is a leaf (not a composite).
fn is_leaf(c: &SpendCondition) -> bool {
    !matches!(c, SpendCondition::All { .. } | SpendCondition::Any { .. })
}

/// Compile `All(children)` where all children must be leaves.
///
/// For n leaves, produces: `<leaf_1_body> OP_VERIFY <leaf_2_body> OP_VERIFY … <leaf_n_body>`
///
/// The last leaf is not followed by `OP_VERIFY`; its result remains on the
/// stack as the script evaluation result. For a single leaf, no `OP_VERIFY` is
/// emitted.
fn compile_all_leaves(children: &[SpendCondition]) -> Result<Vec<u8>> {
    if children.is_empty() {
        return Err(Error::CompileUnsupported(
            "All with zero children is not valid".into(),
        ));
    }

    for (i, child) in children.iter().enumerate() {
        if !is_leaf(child) {
            return Err(Error::CompileUnsupported(format!(
                "All: child at index {i} is composite; \
                 v0 compilation of All only supports leaf children"
            )));
        }
    }

    let mut out: Vec<u8> = Vec::new();
    let last = children.len() - 1;
    let mut b_verify = ScriptBuilder::new();
    b_verify
        .add_op(OpVerify)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    let verify_bytes = b_verify.drain();

    for (i, child) in children.iter().enumerate() {
        let leaf_bytes = compile_leaf(child)?;
        out.extend_from_slice(&leaf_bytes);
        if i < last {
            out.extend_from_slice(&verify_bytes);
        }
    }
    Ok(out)
}

/// Compile a single leaf condition to bytes (helper used by `All`).
fn compile_leaf(c: &SpendCondition) -> Result<Vec<u8>> {
    match c {
        SpendCondition::TimelockHeight {
            deadline,
            controller_xonly,
        }
        | SpendCondition::TimelockUnixSeconds {
            deadline,
            controller_xonly,
        } => compile_timelock(*deadline, controller_xonly),

        SpendCondition::MultiSig {
            threshold,
            xonly_keys,
        } => compile_multisig(*threshold, xonly_keys),

        SpendCondition::All { .. } | SpendCondition::Any { .. } => {
            // Should have been caught by the caller.
            Err(Error::CompileUnsupported(
                "composite condition encountered in leaf compilation path".into(),
            ))
        }
    }
}

/// Compile `Any(branch_a, branch_b)` — two-branch OR.
///
/// ```text
/// OP_IF
///     <branch_a bytes>
/// OP_ELSE
///     <branch_b bytes>
/// OP_ENDIF
/// ```
///
/// Each branch may be a leaf or `All(leaves)`.
fn compile_any_two(branch_a: &SpendCondition, branch_b: &SpendCondition) -> Result<Vec<u8>> {
    let a_bytes = compile_branch(branch_a)?;
    let b_bytes = compile_branch(branch_b)?;

    let mut b = ScriptBuilder::new();
    b.add_op(OpIf)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    let if_prefix = b.drain();

    let mut b2 = ScriptBuilder::new();
    b2.add_op(OpElse)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    let else_bytes = b2.drain();

    let mut b3 = ScriptBuilder::new();
    b3.add_op(OpEndIf)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    let endif_bytes = b3.drain();

    let mut out = Vec::new();
    out.extend_from_slice(&if_prefix);
    out.extend_from_slice(&a_bytes);
    out.extend_from_slice(&else_bytes);
    out.extend_from_slice(&b_bytes);
    out.extend_from_slice(&endif_bytes);
    Ok(out)
}

/// Compile a branch (leaf or `All { children: leaves }`) for use inside `Any`.
fn compile_branch(c: &SpendCondition) -> Result<Vec<u8>> {
    match c {
        SpendCondition::All { children } => compile_all_leaves(children),
        SpendCondition::Any { .. } => Err(Error::CompileUnsupported(
            "nested Any inside Any: v0 does not support Any branches containing Any".into(),
        )),
        leaf if is_leaf(leaf) => compile_leaf(leaf),
        _ => Err(Error::CompileUnsupported(
            "unsupported branch shape in Any".into(),
        )),
    }
}

/// Compute the domain-separated vault script digest.
///
/// This is a thin wrapper around [`kcp_common::digest::script_digest`] that
/// accepts compiled vault script bytes and returns the 32-byte digest.
pub fn vault_script_digest(script_bytes: &[u8]) -> [u8; 32] {
    kcp_script_digest(script_bytes)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::condition::SpendCondition;

    fn key(seed: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = seed;
        k
    }

    // Helper: assert that `bytes` contains `needle` as a sub-slice.
    fn contains_bytes(bytes: &[u8], needle: &[u8]) -> bool {
        bytes.windows(needle.len()).any(|w| w == needle)
    }

    // ── Timelock ─────────────────────────────────────────────────────────────

    #[test]
    fn timelock_height_contains_opcodes() {
        let c = SpendCondition::TimelockHeight {
            deadline: 1_000,
            controller_xonly: key(1),
        };
        let script = compile_condition(&c).unwrap();
        assert!(!script.is_empty());
        // OpCheckLockTimeVerify, OpDrop, OpCheckSig must be present.
        assert!(
            contains_bytes(&script, &[OpCheckLockTimeVerify]),
            "missing OpCheckLockTimeVerify"
        );
        assert!(contains_bytes(&script, &[OpDrop]), "missing OpDrop");
        assert!(contains_bytes(&script, &[OpCheckSig]), "missing OpCheckSig");
        // The 32-byte controller key must appear.
        assert!(
            contains_bytes(&script, &key(1)),
            "controller key not in script"
        );
    }

    #[test]
    fn timelock_unix_seconds_contains_opcodes() {
        let c = SpendCondition::TimelockUnixSeconds {
            deadline: 1_700_000_000,
            controller_xonly: key(2),
        };
        let script = compile_condition(&c).unwrap();
        assert!(contains_bytes(&script, &[OpCheckLockTimeVerify]));
        assert!(contains_bytes(&script, &[OpDrop]));
        assert!(contains_bytes(&script, &[OpCheckSig]));
        assert!(contains_bytes(&script, &key(2)));
    }

    #[test]
    fn timelock_digest_deterministic() {
        let c = SpendCondition::TimelockHeight {
            deadline: 500,
            controller_xonly: key(3),
        };
        let s1 = compile_condition(&c).unwrap();
        let s2 = compile_condition(&c).unwrap();
        assert_eq!(vault_script_digest(&s1), vault_script_digest(&s2));
    }

    #[test]
    fn different_timelocks_different_digest() {
        let c1 = SpendCondition::TimelockHeight {
            deadline: 500,
            controller_xonly: key(3),
        };
        let c2 = SpendCondition::TimelockHeight {
            deadline: 501,
            controller_xonly: key(3),
        };
        let d1 = vault_script_digest(&compile_condition(&c1).unwrap());
        let d2 = vault_script_digest(&compile_condition(&c2).unwrap());
        assert_ne!(d1, d2);
    }

    // ── MultiSig ─────────────────────────────────────────────────────────────

    #[test]
    fn multisig_contains_opcodes() {
        let c = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![key(1), key(2), key(3)],
        };
        let script = compile_condition(&c).unwrap();
        assert!(contains_bytes(&script, &[OpCheckMultiSig]));
        assert!(contains_bytes(&script, &key(1)));
        assert!(contains_bytes(&script, &key(2)));
        assert!(contains_bytes(&script, &key(3)));
    }

    // ── All(leaves) ──────────────────────────────────────────────────────────

    #[test]
    fn all_two_leaves_contains_verify() {
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
        let script = compile_condition(&c).unwrap();
        // Both leaf opcodes present.
        assert!(contains_bytes(&script, &[OpCheckLockTimeVerify]));
        assert!(contains_bytes(&script, &[OpCheckMultiSig]));
        // OP_VERIFY joins the leaves.
        assert!(contains_bytes(&script, &[OpVerify]));
    }

    #[test]
    fn all_with_composite_child_unsupported() {
        let c = SpendCondition::All {
            children: vec![SpendCondition::Any {
                children: vec![SpendCondition::TimelockHeight {
                    deadline: 1,
                    controller_xonly: key(1),
                }],
            }],
        };
        let err = compile_condition(&c).unwrap_err();
        assert!(
            matches!(err, Error::CompileUnsupported(_)),
            "expected CompileUnsupported, got: {err}"
        );
    }

    // ── Any(2) ───────────────────────────────────────────────────────────────

    #[test]
    fn any_two_contains_if_else_endif() {
        let c = SpendCondition::Any {
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
        let script = compile_condition(&c).unwrap();
        assert!(contains_bytes(&script, &[OpIf]));
        assert!(contains_bytes(&script, &[OpElse]));
        assert!(contains_bytes(&script, &[OpEndIf]));
        assert!(contains_bytes(&script, &[OpCheckLockTimeVerify]));
        assert!(contains_bytes(&script, &[OpCheckMultiSig]));
    }

    #[test]
    fn any_three_unsupported() {
        let leaf = SpendCondition::TimelockHeight {
            deadline: 1,
            controller_xonly: key(1),
        };
        let c = SpendCondition::Any {
            children: vec![leaf.clone(), leaf.clone(), leaf],
        };
        let err = compile_condition(&c).unwrap_err();
        assert!(
            matches!(err, Error::CompileUnsupported(_)),
            "expected CompileUnsupported, got: {err}"
        );
    }

    #[test]
    fn any_one_unsupported() {
        let c = SpendCondition::Any {
            children: vec![SpendCondition::TimelockHeight {
                deadline: 1,
                controller_xonly: key(1),
            }],
        };
        let err = compile_condition(&c).unwrap_err();
        assert!(
            matches!(err, Error::CompileUnsupported(_)),
            "expected CompileUnsupported, got: {err}"
        );
    }

    // ── Digest ───────────────────────────────────────────────────────────────

    #[test]
    fn vault_script_digest_deterministic() {
        let script = compile_condition(&SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(5)],
        })
        .unwrap();
        let d1 = vault_script_digest(&script);
        let d2 = vault_script_digest(&script);
        assert_eq!(d1, d2);
    }

    #[test]
    fn vault_script_digest_differs_from_raw_sha256() {
        use sha2::{Digest, Sha256};
        let script = compile_condition(&SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(5)],
        })
        .unwrap();
        let kcp_digest = vault_script_digest(&script);
        let raw: [u8; 32] = Sha256::digest(&script).into();
        // The KCP domain tag must make these different.
        assert_ne!(kcp_digest, raw);
    }
}
