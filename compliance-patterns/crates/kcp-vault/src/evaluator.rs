//! Off-chain spending-condition evaluator.
//!
//! [`evaluate`] decides whether a [`SpendCondition`] is satisfied in a given
//! [`EvalContext`]. All evaluation is pure and offline — no node connection is
//! required.
//!
//! ## Semantics (mirrors the `kii-vault` evaluator)
//!
//! - **TimelockHeight** — passes when `ctx.daa_score >= deadline`.
//! - **TimelockUnixSeconds** — passes when `ctx.unix_seconds >= deadline`.
//! - **MultiSig** — counts the distinct signers in `ctx.signers_present` that
//!   appear in `xonly_keys`; passes when the count is at least `threshold`.
//! - **All** — all sub-conditions must evaluate to `true`.
//! - **Any** — at least one sub-condition must evaluate to `true`.
//!
//! The evaluator does **not** validate the condition structure; call
//! [`SpendCondition::validate`](crate::condition::SpendCondition::validate)
//! before relying on evaluation results if the condition originates from
//! untrusted input.

use crate::condition::SpendCondition;

/// The runtime context for evaluating a spending condition.
///
/// Fields are deliberately simple `u64` / `Vec` so the evaluator stays pure
/// and allocation-free beyond the caller's own data.
#[derive(Debug, Clone)]
pub struct EvalContext {
    /// The current DAA (Difficulty Adjustment Algorithm) block height. Used to
    /// evaluate [`SpendCondition::TimelockHeight`] conditions.
    pub daa_score: u64,
    /// The current Unix time in seconds since the epoch. Used to evaluate
    /// [`SpendCondition::TimelockUnixSeconds`] conditions.
    pub unix_seconds: u64,
    /// The set of signers whose Schnorr signatures are present in the spending
    /// transaction. Each entry is a 32-byte x-only public key. The evaluator
    /// de-duplicates this list when counting multisig quorum.
    pub signers_present: Vec<[u8; 32]>,
}

/// Evaluate `condition` against `ctx`, returning `true` if the condition is
/// satisfied and `false` otherwise.
///
/// Evaluation is pure and non-recursive past the condition's own tree.
/// It does not modify `ctx`.
pub fn evaluate(condition: &SpendCondition, ctx: &EvalContext) -> bool {
    match condition {
        SpendCondition::TimelockHeight { deadline, .. } => ctx.daa_score >= *deadline,

        SpendCondition::TimelockUnixSeconds { deadline, .. } => ctx.unix_seconds >= *deadline,

        SpendCondition::MultiSig {
            threshold,
            xonly_keys,
        } => {
            // Count distinct signers present whose key appears in xonly_keys.
            // De-duplicate signers_present defensively (the list should already
            // be distinct, but we don't trust the caller's construction).
            let mut count: u8 = 0;
            let mut seen: Vec<[u8; 32]> = Vec::new();
            for signer in &ctx.signers_present {
                if seen.contains(signer) {
                    continue;
                }
                seen.push(*signer);
                if xonly_keys.contains(signer) {
                    count = count.saturating_add(1);
                }
            }
            count >= *threshold
        }

        SpendCondition::All { children } => children.iter().all(|c| evaluate(c, ctx)),

        SpendCondition::Any { children } => children.iter().any(|c| evaluate(c, ctx)),
    }
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

    fn ctx(daa: u64, unix: u64, signers: &[[u8; 32]]) -> EvalContext {
        EvalContext {
            daa_score: daa,
            unix_seconds: unix,
            signers_present: signers.to_vec(),
        }
    }

    // ── TimelockHeight ───────────────────────────────────────────────────────

    #[test]
    fn timelock_height_exact_passes() {
        let c = SpendCondition::TimelockHeight {
            deadline: 1_000,
            controller_xonly: key(1),
        };
        assert!(evaluate(&c, &ctx(1_000, 0, &[])));
    }

    #[test]
    fn timelock_height_above_passes() {
        let c = SpendCondition::TimelockHeight {
            deadline: 1_000,
            controller_xonly: key(1),
        };
        assert!(evaluate(&c, &ctx(1_001, 0, &[])));
    }

    #[test]
    fn timelock_height_below_fails() {
        let c = SpendCondition::TimelockHeight {
            deadline: 1_000,
            controller_xonly: key(1),
        };
        assert!(!evaluate(&c, &ctx(999, 0, &[])));
    }

    #[test]
    fn timelock_height_zero_deadline_always_passes() {
        let c = SpendCondition::TimelockHeight {
            deadline: 0,
            controller_xonly: key(1),
        };
        assert!(evaluate(&c, &ctx(0, 0, &[])));
    }

    // ── TimelockUnixSeconds ──────────────────────────────────────────────────

    #[test]
    fn timelock_unix_exact_passes() {
        let c = SpendCondition::TimelockUnixSeconds {
            deadline: 1_700_000_000,
            controller_xonly: key(2),
        };
        assert!(evaluate(&c, &ctx(0, 1_700_000_000, &[])));
    }

    #[test]
    fn timelock_unix_above_passes() {
        let c = SpendCondition::TimelockUnixSeconds {
            deadline: 1_700_000_000,
            controller_xonly: key(2),
        };
        assert!(evaluate(&c, &ctx(0, 1_700_000_001, &[])));
    }

    #[test]
    fn timelock_unix_below_fails() {
        let c = SpendCondition::TimelockUnixSeconds {
            deadline: 1_700_000_000,
            controller_xonly: key(2),
        };
        assert!(!evaluate(&c, &ctx(0, 1_699_999_999, &[])));
    }

    // ── MultiSig ─────────────────────────────────────────────────────────────

    #[test]
    fn multisig_1_of_1_exact_passes() {
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(1)],
        };
        assert!(evaluate(&c, &ctx(0, 0, &[key(1)])));
    }

    #[test]
    fn multisig_1_of_1_no_signer_fails() {
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(1)],
        };
        assert!(!evaluate(&c, &ctx(0, 0, &[])));
    }

    #[test]
    fn multisig_2_of_3_exact_threshold_passes() {
        let c = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![key(1), key(2), key(3)],
        };
        // Provide exactly 2 of the 3 keys.
        assert!(evaluate(&c, &ctx(0, 0, &[key(1), key(3)])));
    }

    #[test]
    fn multisig_2_of_3_one_below_threshold_fails() {
        let c = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![key(1), key(2), key(3)],
        };
        assert!(!evaluate(&c, &ctx(0, 0, &[key(1)])));
    }

    #[test]
    fn multisig_duplicate_signers_counted_once() {
        // A signer that appears twice in signers_present must only count once.
        let c = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![key(1), key(2)],
        };
        // key(1) appears twice — should still count as 1.
        assert!(!evaluate(&c, &ctx(0, 0, &[key(1), key(1)])));
    }

    #[test]
    fn multisig_wrong_key_fails() {
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(1)],
        };
        // key(2) is not in xonly_keys.
        assert!(!evaluate(&c, &ctx(0, 0, &[key(2)])));
    }

    #[test]
    fn multisig_extra_non_matching_signers_irrelevant() {
        let c = SpendCondition::MultiSig {
            threshold: 1,
            xonly_keys: vec![key(1)],
        };
        // key(99) is irrelevant; key(1) satisfies.
        assert!(evaluate(&c, &ctx(0, 0, &[key(99), key(1)])));
    }

    // ── All ──────────────────────────────────────────────────────────────────

    #[test]
    fn all_both_pass() {
        let c = SpendCondition::All {
            children: vec![
                SpendCondition::TimelockHeight {
                    deadline: 10,
                    controller_xonly: key(1),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![key(2)],
                },
            ],
        };
        assert!(evaluate(&c, &ctx(10, 0, &[key(2)])));
    }

    #[test]
    fn all_first_fails() {
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
        // daa_score 5 < deadline 100.
        assert!(!evaluate(&c, &ctx(5, 0, &[key(2)])));
    }

    #[test]
    fn all_second_fails() {
        let c = SpendCondition::All {
            children: vec![
                SpendCondition::TimelockHeight {
                    deadline: 10,
                    controller_xonly: key(1),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![key(2)],
                },
            ],
        };
        // No signers present.
        assert!(!evaluate(&c, &ctx(10, 0, &[])));
    }

    // ── Any ──────────────────────────────────────────────────────────────────

    #[test]
    fn any_first_passes() {
        let c = SpendCondition::Any {
            children: vec![
                SpendCondition::TimelockHeight {
                    deadline: 10,
                    controller_xonly: key(1),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![key(2)],
                },
            ],
        };
        // Timelock passes; no signers.
        assert!(evaluate(&c, &ctx(10, 0, &[])));
    }

    #[test]
    fn any_second_passes() {
        let c = SpendCondition::Any {
            children: vec![
                SpendCondition::TimelockHeight {
                    deadline: 1_000_000,
                    controller_xonly: key(1),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![key(2)],
                },
            ],
        };
        // Timelock not yet passed; multisig satisfied.
        assert!(evaluate(&c, &ctx(0, 0, &[key(2)])));
    }

    #[test]
    fn any_neither_passes() {
        let c = SpendCondition::Any {
            children: vec![
                SpendCondition::TimelockHeight {
                    deadline: 1_000_000,
                    controller_xonly: key(1),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![key(2)],
                },
            ],
        };
        assert!(!evaluate(&c, &ctx(0, 0, &[])));
    }

    // ── Composite nesting ─────────────────────────────────────────────────────

    #[test]
    fn composite_nesting_any_of_all() {
        // Any { children: [ All { children: [timelock_h, multisig] }, Any { children: [timelock_u] } ] }
        let c = SpendCondition::Any {
            children: vec![
                SpendCondition::All {
                    children: vec![
                        SpendCondition::TimelockHeight {
                            deadline: 5,
                            controller_xonly: key(1),
                        },
                        SpendCondition::MultiSig {
                            threshold: 2,
                            xonly_keys: vec![key(10), key(11)],
                        },
                    ],
                },
                SpendCondition::Any {
                    children: vec![SpendCondition::TimelockUnixSeconds {
                        deadline: 999,
                        controller_xonly: key(3),
                    }],
                },
            ],
        };

        // Outer Any: first branch (All) fails (no signers); second branch passes (unix >= 999).
        assert!(evaluate(&c, &ctx(10, 999, &[])));

        // Outer Any: first branch passes (daa=5, both keys present).
        assert!(evaluate(&c, &ctx(5, 0, &[key(10), key(11)])));

        // Neither branch passes.
        assert!(!evaluate(&c, &ctx(0, 0, &[])));
    }

    #[test]
    fn threshold_boundary_multisig() {
        // 3-of-3 requires all three.
        let c = SpendCondition::MultiSig {
            threshold: 3,
            xonly_keys: vec![key(1), key(2), key(3)],
        };
        assert!(!evaluate(&c, &ctx(0, 0, &[key(1), key(2)])));
        assert!(evaluate(&c, &ctx(0, 0, &[key(1), key(2), key(3)])));
    }
}
