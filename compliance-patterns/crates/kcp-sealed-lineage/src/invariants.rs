//! Sealed-lineage chain invariants (pure, off-chain validation).
//!
//! A sealed lineage is a sequence of decoded [`crate::payload::Payload`]
//! values extracted from on-chain transactions in order. This module defines
//! the four off-chain invariants that must hold for a chain to be considered
//! well-formed.
//!
//! ## Event classes
//!
//! | Value | Name    | Meaning |
//! |-------|---------|---------|
//! | `0x00` | [`GENESIS`] | First event; only valid at `seq = 0`. |
//! | `0x01` | [`APPEND`]  | Normal evidence append; valid at any `seq ≥ 1`. |
//! | `0x02` | [`CLOSE`]   | Terminal event; nothing may follow. |
//!
//! Unknown class values are rejected by [`validate_chain`] as
//! [`Error::InvariantUnknownEventClass`].
//!
//! ## Invariants (L-1 through L-4)
//!
//! - **L-1 — monotone sequence:** `seq` starts at `0` (Genesis) and
//!   increments by exactly `1` for every subsequent event.
//! - **L-2 — lineage identity:** every event in the chain carries the same
//!   `lineage_id`.
//! - **L-3 — event-class rules:** Genesis (`0x00`) is only valid at `seq = 0`;
//!   Append (`0x01`) is valid at any `seq ≥ 1`; Close (`0x02`) is terminal —
//!   no event may appear after a Close; unknown class values are rejected.
//! - **L-4 — temporal envelope:** `t_bucket` values must be non-decreasing and
//!   each step must not exceed [`T_BUCKET_MAX_STEP_SECS`] seconds
//!   (`90 × 24 × 3600 = 7 776 000 s`).
//!
//! ## Value-carry note
//!
//! The donor lineage system enforces a fifth invariant (V-5): each successor
//! transaction must carry the full lineage UTXO value forward (minus fee).
//! In v0 this is enforced at the transaction layer by the single-successor
//! UTXO shape used in [`crate::tx`], not here. Pure-chain validation has no
//! access to UTXO amounts; callers that need V-5 must verify it separately
//! from the on-chain transaction graph.
//!
//! ## v0 honesty note
//!
//! These invariants are checked **off-chain** only. In v0, consensus does not
//! introspect the payload or verify the chain; lineage validity is
//! application-layer. Full introspection-enforced lineage (consensus rejecting
//! malformed successors) is the documented next step, requiring covenant
//! declaration opcodes available on Toccata.

use crate::error::{Error, Result};
use crate::payload::Payload;

/// Genesis event class: only valid at `seq = 0`.
pub const GENESIS: u8 = 0x00;

/// Append event class: valid at any `seq ≥ 1`.
pub const APPEND: u8 = 0x01;

/// Close event class: terminal — nothing may follow this event.
pub const CLOSE: u8 = 0x02;

/// Maximum allowed step between consecutive `t_bucket` values (seconds).
///
/// Set to `90 × 24 × 3600 = 7 776 000 s` (ninety days). This follows the
/// donor lineage system's evidence-cadence envelope, which requires publishers
/// to refresh or close a lineage at least once per quarter to keep the
/// chain "live". A gap larger than this suggests missed evidence windows or
/// a stale/abandoned lineage and is treated as a protocol violation.
pub const T_BUCKET_MAX_STEP_SECS: u64 = 90 * 24 * 3600; // 7_776_000

/// Validate a decoded sealed-lineage chain against all four invariants.
///
/// `payloads` must be ordered oldest-first (genesis first) exactly as they
/// appear in the on-chain UTXO chain.
///
/// An empty `payloads` slice is rejected — a valid chain must have at least
/// a genesis event.
///
/// # Errors
///
/// Returns the first invariant violation encountered, from earlier events to
/// later:
///
/// - [`Error::InvariantEmptyChain`] if `payloads` is empty.
/// - [`Error::InvariantUnknownEventClass`] if any event has an unrecognised
///   class byte.
/// - [`Error::InvariantSeqGap`] if any `seq` is not the expected next value
///   (L-1).
/// - [`Error::InvariantLineageIdMismatch`] if any event carries a `lineage_id`
///   different from the first event (L-2).
/// - [`Error::InvariantGenesisNotAtSeqZero`] if a Genesis event appears at
///   `seq ≠ 0`, or if a non-Genesis event appears at `seq = 0` (L-3).
/// - [`Error::InvariantEventAfterClose`] if any event follows a Close (L-3).
/// - [`Error::InvariantTBucketDecreased`] if `t_bucket` decreases (L-4).
/// - [`Error::InvariantTBucketStepExceeded`] if the step between consecutive
///   `t_bucket` values exceeds [`T_BUCKET_MAX_STEP_SECS`] (L-4).
pub fn validate_chain(payloads: &[Payload]) -> Result<()> {
    if payloads.is_empty() {
        return Err(Error::InvariantEmptyChain);
    }

    let expected_lineage_id = payloads[0].lineage_id;
    let mut closed = false;

    for (i, p) in payloads.iter().enumerate() {
        let expected_seq = i as u64;

        // L-3: reject unknown event classes first (cleanest error ordering).
        if p.event_class != GENESIS && p.event_class != APPEND && p.event_class != CLOSE {
            return Err(Error::InvariantUnknownEventClass {
                index: i,
                class: p.event_class,
            });
        }

        // L-3: nothing may follow a Close.
        if closed {
            return Err(Error::InvariantEventAfterClose { index: i });
        }

        // L-1: seq must be exactly i.
        if p.seq != expected_seq {
            return Err(Error::InvariantSeqGap {
                index: i,
                expected: expected_seq,
                got: p.seq,
            });
        }

        // L-2: lineage_id must be identical throughout.
        if p.lineage_id != expected_lineage_id {
            return Err(Error::InvariantLineageIdMismatch { index: i });
        }

        // L-3: Genesis only at seq 0; non-Genesis must not be at seq 0.
        if p.event_class == GENESIS && p.seq != 0 {
            return Err(Error::InvariantGenesisNotAtSeqZero { index: i });
        }
        if p.event_class != GENESIS && p.seq == 0 {
            return Err(Error::InvariantGenesisNotAtSeqZero { index: i });
        }

        // L-4: temporal envelope.
        if i > 0 {
            let prev_t = payloads[i - 1].t_bucket;
            if p.t_bucket < prev_t {
                return Err(Error::InvariantTBucketDecreased {
                    index: i,
                    prev: prev_t,
                    got: p.t_bucket,
                });
            }
            let step = p.t_bucket - prev_t;
            if step > T_BUCKET_MAX_STEP_SECS {
                return Err(Error::InvariantTBucketStepExceeded {
                    index: i,
                    step,
                    max: T_BUCKET_MAX_STEP_SECS,
                });
            }
        }

        if p.event_class == CLOSE {
            closed = true;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LID: [u8; 32] = [0xaa; 32];
    const COM: [u8; 32] = [0xcc; 32];
    const T0: u64 = 1_700_000_000;

    fn p(seq: u64, class: u8, t: u64) -> Payload {
        Payload {
            lineage_id: LID,
            seq,
            event_class: class,
            t_bucket: t,
            commitment: COM,
        }
    }

    // ---- positive tests ----

    #[test]
    fn valid_single_genesis() {
        validate_chain(&[p(0, GENESIS, T0)]).unwrap();
    }

    #[test]
    fn valid_genesis_then_append() {
        validate_chain(&[p(0, GENESIS, T0), p(1, APPEND, T0)]).unwrap();
    }

    #[test]
    fn valid_genesis_append_close() {
        validate_chain(&[p(0, GENESIS, T0), p(1, APPEND, T0), p(2, CLOSE, T0)]).unwrap();
    }

    #[test]
    fn valid_t_bucket_same_across_events() {
        validate_chain(&[p(0, GENESIS, T0), p(1, APPEND, T0), p(2, APPEND, T0)]).unwrap();
    }

    #[test]
    fn valid_t_bucket_increasing() {
        validate_chain(&[
            p(0, GENESIS, T0),
            p(1, APPEND, T0 + 1000),
            p(2, APPEND, T0 + 2000),
        ])
        .unwrap();
    }

    #[test]
    fn valid_t_bucket_step_exactly_90_days() {
        // Exactly at the boundary should pass.
        validate_chain(&[p(0, GENESIS, T0), p(1, APPEND, T0 + T_BUCKET_MAX_STEP_SECS)]).unwrap();
    }

    // ---- empty chain ----

    #[test]
    fn empty_chain_rejected() {
        let err = validate_chain(&[]).unwrap_err();
        assert!(matches!(err, Error::InvariantEmptyChain), "{err}");
    }

    // ---- L-1: seq gap ----

    #[test]
    fn seq_skips_fails_l1() {
        let err = validate_chain(&[p(0, GENESIS, T0), p(2, APPEND, T0)]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvariantSeqGap {
                    index: 1,
                    expected: 1,
                    got: 2
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn seq_repeats_fails_l1() {
        let err =
            validate_chain(&[p(0, GENESIS, T0), p(1, APPEND, T0), p(1, APPEND, T0)]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvariantSeqGap {
                    index: 2,
                    expected: 2,
                    got: 1
                }
            ),
            "{err}"
        );
    }

    // ---- L-2: lineage_id mismatch ----

    #[test]
    fn lineage_id_mismatch_fails_l2() {
        let mut bad = p(1, APPEND, T0);
        bad.lineage_id = [0xbb; 32];
        let err = validate_chain(&[p(0, GENESIS, T0), bad]).unwrap_err();
        assert!(
            matches!(err, Error::InvariantLineageIdMismatch { index: 1 }),
            "{err}"
        );
    }

    // ---- L-3: event-class rules ----

    #[test]
    fn unknown_event_class_rejected() {
        let err = validate_chain(&[p(0, 0xff, T0)]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvariantUnknownEventClass {
                    index: 0,
                    class: 0xff
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn genesis_at_nonzero_seq_fails_l3() {
        // seq is checked before event_class rule, so build a valid-seq event
        // but force Genesis at seq 1.
        let mut bad = p(1, GENESIS, T0);
        bad.seq = 1;
        let err = validate_chain(&[p(0, GENESIS, T0), bad]).unwrap_err();
        assert!(
            matches!(err, Error::InvariantGenesisNotAtSeqZero { index: 1 }),
            "{err}"
        );
    }

    #[test]
    fn append_at_seq_zero_fails_l3() {
        let err = validate_chain(&[p(0, APPEND, T0)]).unwrap_err();
        assert!(
            matches!(err, Error::InvariantGenesisNotAtSeqZero { index: 0 }),
            "{err}"
        );
    }

    #[test]
    fn close_at_seq_zero_fails_l3() {
        let err = validate_chain(&[p(0, CLOSE, T0)]).unwrap_err();
        assert!(
            matches!(err, Error::InvariantGenesisNotAtSeqZero { index: 0 }),
            "{err}"
        );
    }

    #[test]
    fn event_after_close_fails_l3() {
        let err =
            validate_chain(&[p(0, GENESIS, T0), p(1, CLOSE, T0), p(2, APPEND, T0)]).unwrap_err();
        assert!(
            matches!(err, Error::InvariantEventAfterClose { index: 2 }),
            "{err}"
        );
    }

    #[test]
    fn two_closes_fails_l3() {
        let err =
            validate_chain(&[p(0, GENESIS, T0), p(1, CLOSE, T0), p(2, CLOSE, T0)]).unwrap_err();
        assert!(
            matches!(err, Error::InvariantEventAfterClose { index: 2 }),
            "{err}"
        );
    }

    // ---- L-4: temporal envelope ----

    #[test]
    fn t_bucket_decreases_fails_l4() {
        let err = validate_chain(&[p(0, GENESIS, T0 + 1000), p(1, APPEND, T0)]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvariantTBucketDecreased {
                    index: 1,
                    prev: _,
                    got: _
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn t_bucket_step_exceeds_90_days_fails_l4() {
        let err = validate_chain(&[
            p(0, GENESIS, T0),
            p(1, APPEND, T0 + T_BUCKET_MAX_STEP_SECS + 1),
        ])
        .unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvariantTBucketStepExceeded {
                    index: 1,
                    step: _,
                    max: T_BUCKET_MAX_STEP_SECS
                }
            ),
            "{err}"
        );
    }
}
