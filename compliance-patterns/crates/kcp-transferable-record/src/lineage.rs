//! Transfer lineage validation.
//!
//! A transferable record begins with a genesis controller (the x-only public
//! key that owns the record UTXO at creation). Each transfer adds a
//! [`TransferEvent`] that records the new controller, a sequence number, the
//! record identifier, and a payload commitment.
//!
//! ## Lineage invariants (v0)
//!
//! - **TR-1 — monotone sequence:** `seq` starts at 1 for the first transfer
//!   and increments by exactly 1 for each subsequent event.
//! - **TR-2 — record identity:** every event carries the same `record_id`.
//! - **TR-3 — payload consistency:** every event's `commitment` is a 32-byte
//!   value (non-zero enforced here only as a structural sanity check; content
//!   is application-layer).
//!
//! ## Custody is NOT verified here
//!
//! [`TransferEvent::controller_xonly`] is **carried, never checked**. It records
//! the controller *after* an event; the key that authorised the event is not
//! recorded anywhere in this type, and the controller is not part of the
//! 78-byte on-chain payload either (see [`crate::payload`]) — it is the
//! record UTXO's locking script. So there is nothing in a disclosed event list
//! to cross-check a controller against: this function cannot detect a
//! fabricated chain whose controller keys are discontinuous or invented.
//!
//! **The custody anchor is the UTXO chain, not this function.** Each transfer
//! spends the previous record UTXO, so consensus required the previous
//! controller's signature; reconstruct the controller sequence from the chain
//! (independently of whoever discloses the events) and treat `validate_chain`
//! as a payload-invariant check only.
//!
//! An earlier revision papered over this: `validate_chain` took a
//! `genesis_controller` argument that it never read, while its doc-comment
//! claimed it validated "against the genesis controller". The argument is gone
//! rather than left as a security-looking no-op.
//!
//! ## Re-affirmation (same-controller transfer)
//!
//! A transfer to the same controller as the previous one is **allowed**. This
//! constitutes a re-affirmation event (updating the on-chain commitment without
//! changing ownership). Clients that want to prohibit no-op controller rotations
//! must enforce that rule above this layer.
//!
//! ## v0 honesty note
//!
//! These invariants are checked **off-chain** only. In v0, consensus does not
//! introspect the payload or verify the sequence number; lineage validity is
//! application-layer. Full introspection-enforced lineage (consensus rejecting
//! malformed successors) is the documented next step.

use crate::error::{Error, Result};

/// A single transfer event in a record's lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferEvent {
    /// Transfer sequence number (first event = 1, strictly incrementing).
    pub seq: u64,
    /// The record identifier; must be identical across all events.
    pub record_id: [u8; 32],
    /// The x-only public key of the controller **after** this event.
    ///
    /// Carried for the caller's benefit; [`validate_chain`] never reads it, and
    /// it is not part of the on-chain payload. See the module docs.
    pub controller_xonly: [u8; 32],
    /// SHA-256 commitment to the event body.
    pub commitment: [u8; 32],
}

/// Validate the payload invariants of a chain of transfer events.
///
/// `events` is the ordered sequence of transfer events read from the record's
/// on-chain UTXO chain. TR-1, TR-2 and the TR-3 structural check are enforced;
/// **controller keys are not checked at all** — see the module docs. A record
/// that has never been transferred has no chain to validate, so an empty slice
/// is an error rather than a vacuous accept.
///
/// # Errors
///
/// Returns the first invariant violation found, from earlier events to later:
///
/// - [`Error::LineageEmpty`] if `events` is empty — validating nothing must not
///   report success.
/// - [`Error::LineageSeqGap`] if any event's `seq` is not the expected next
///   value (TR-1).
/// - [`Error::LineageRecordIdMismatch`] if any event carries a `record_id`
///   different from the genesis `record_id` (TR-2). The genesis `record_id`
///   is taken from the first event.
/// - [`Error::LineageEmptyCommitment`] if any event's `commitment` is all
///   zero bytes (structural sanity — a genuine all-zero SHA-256 is
///   astronomically unlikely; if you need to permit it, bypass this layer).
pub fn validate_chain(events: &[TransferEvent]) -> Result<()> {
    if events.is_empty() {
        return Err(Error::LineageEmpty);
    }

    // The record_id is anchored to the first event in the chain.
    let expected_record_id = events[0].record_id;

    for (i, ev) in events.iter().enumerate() {
        let expected_seq = (i as u64) + 1;

        // TR-1: seq must increment by 1 from 1.
        if ev.seq != expected_seq {
            return Err(Error::LineageSeqGap {
                expected: expected_seq,
                got: ev.seq,
            });
        }

        // TR-2: record_id must be identical across all events.
        if ev.record_id != expected_record_id {
            return Err(Error::LineageRecordIdMismatch { index: i });
        }

        // TR-3 (structural): commitment must not be all-zero.
        if ev.commitment == [0u8; 32] {
            return Err(Error::LineageEmptyCommitment { index: i });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD_ID: [u8; 32] = [0xaa; 32];

    fn ev(seq: u64, controller: u8, commitment: u8) -> TransferEvent {
        TransferEvent {
            seq,
            record_id: RECORD_ID,
            controller_xonly: [controller; 32],
            commitment: [commitment; 32],
        }
    }

    // ---- TR-1: sequence ----

    #[test]
    fn valid_single_event() {
        let events = vec![ev(1, 0x02, 0xcc)];
        validate_chain(&events).unwrap();
    }

    #[test]
    fn valid_chain_of_three() {
        let events = vec![ev(1, 0x02, 0xcc), ev(2, 0x03, 0xdd), ev(3, 0x04, 0xee)];
        validate_chain(&events).unwrap();
    }

    #[test]
    fn empty_chain_is_rejected() {
        // Validating nothing must not report success: an auditor handed an
        // empty list has verified nothing, not verified a valid chain.
        let err = validate_chain(&[]).unwrap_err();
        assert!(matches!(err, Error::LineageEmpty), "unexpected: {err}");
    }

    // ---- custody is NOT verified (pins the documented limitation) ----

    /// `controller_xonly` is carried, never checked. A fabricated chain whose
    /// controller keys jump between unrelated keys is accepted, because nothing
    /// in a `TransferEvent` records *who authorised* the transfer. If this test
    /// ever fails, custody continuity has become checkable — update the module
    /// docs and the README threat model with it.
    #[test]
    fn discontinuous_controller_keys_are_accepted_custody_is_not_checked() {
        let events = vec![ev(1, 0x02, 0xcc), ev(2, 0x99, 0xdd), ev(3, 0x07, 0xee)];
        validate_chain(&events).expect("controller keys are not an input to validation");
    }

    #[test]
    fn seq_starts_at_zero_fails_tr1() {
        let events = vec![ev(0, 0x02, 0xcc)];
        let err = validate_chain(&events).unwrap_err();
        assert!(
            matches!(
                err,
                Error::LineageSeqGap {
                    expected: 1,
                    got: 0
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn seq_skips_fails_tr1() {
        let events = vec![ev(1, 0x02, 0xcc), ev(3, 0x03, 0xdd)];
        let err = validate_chain(&events).unwrap_err();
        assert!(
            matches!(
                err,
                Error::LineageSeqGap {
                    expected: 2,
                    got: 3
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn seq_repeats_fails_tr1() {
        let events = vec![ev(1, 0x02, 0xcc), ev(1, 0x03, 0xdd)];
        let err = validate_chain(&events).unwrap_err();
        assert!(
            matches!(
                err,
                Error::LineageSeqGap {
                    expected: 2,
                    got: 1
                }
            ),
            "unexpected: {err}"
        );
    }

    // ---- TR-2: record_id ----

    #[test]
    fn record_id_mismatch_fails_tr2() {
        let mut ev2 = ev(2, 0x03, 0xdd);
        ev2.record_id = [0xbb; 32]; // different record_id
        let events = vec![ev(1, 0x02, 0xcc), ev2];
        let err = validate_chain(&events).unwrap_err();
        assert!(
            matches!(err, Error::LineageRecordIdMismatch { index: 1 }),
            "unexpected: {err}"
        );
    }

    // ---- TR-3 (structural): commitment ----

    #[test]
    fn all_zero_commitment_fails_tr3() {
        let mut bad = ev(1, 0x02, 0xcc);
        bad.commitment = [0u8; 32];
        let err = validate_chain(&[bad]).unwrap_err();
        assert!(
            matches!(err, Error::LineageEmptyCommitment { index: 0 }),
            "unexpected: {err}"
        );
    }

    // ---- re-affirmation ----

    #[test]
    fn same_controller_reaffirmation_is_allowed() {
        // Transferring to the same controller is legal in v0.
        let events = vec![
            ev(1, 0x02, 0xcc),
            ev(2, 0x02, 0xdd), // same controller_xonly as event 1
        ];
        validate_chain(&events).unwrap();
    }
}
