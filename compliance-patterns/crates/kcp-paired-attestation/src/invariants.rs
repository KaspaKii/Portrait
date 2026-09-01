//! Two-party attestation lineage invariants (pure, off-chain validation).
//!
//! A paired-attestation lineage is a sequence of decoded [`crate::payload::Payload`]
//! values extracted from on-chain transactions in order. This module defines
//! four invariants (PA-1 through PA-4) that must hold for a chain to be
//! considered well-formed.
//!
//! ## Event classes
//!
//! | Value | Name         | Meaning |
//! |-------|--------------|---------|
//! | `0x00` | [`PARTY_A_COMMIT`] | Party A's commitment; only valid at `seq = 0`. |
//! | `0x01` | [`PARTY_B_MATE`]   | Party B's mating commitment; only valid at `seq = 1`. |
//! | `0x02` | [`CLOSE`]          | Terminal event; nothing may follow. |
//!
//! Unknown class values are rejected by [`validate_chain`] as
//! [`Error::InvariantUnknownEventClass`].
//!
//! ## Invariants (PA-1 through PA-4)
//!
//! - **PA-1 — monotone sequence:** `seq` starts at `0` (`PartyACommit`) and
//!   increments by exactly `1` for every subsequent event.
//! - **PA-2 — attestation identity:** every event in the chain carries the
//!   same `attestation_id`.
//! - **PA-3 — event-class order:** `PartyACommit` (`0x00`) is only valid at
//!   `seq = 0`; `PartyBMate` (`0x01`) is only valid at `seq = 1`; `Close`
//!   (`0x02`) is terminal — no event may appear after a `Close`; unknown
//!   class values are rejected.
//! - **PA-4 — mate proof:** the `seq = 1` (`PartyBMate`) event must carry a
//!   [`MateProof`] in its associated proof slot, and that proof must pass
//!   [`crate::mate::verify_mate`]. The proof bytes are passed in alongside
//!   the payload slice (see [`validate_chain`] for the calling convention).
//!
//! ## v0 honesty note
//!
//! These invariants are checked **off-chain** only. Consensus does not
//! introspect the payload or verify the chain; lineage validity is
//! application-layer. The full on-chain two-datasig enforcement is proven
//! viable (FACTS SS-024-v4) but requires P2SH spend-path plumbing not yet in
//! `kcp-common`; it is the documented next step.

use crate::error::{Error, Result};
use crate::mate::{verify_mate, MateProof};
use crate::payload::Payload;

/// `PartyACommit` event class: only valid at `seq = 0`.
pub const PARTY_A_COMMIT: u8 = 0x00;

/// `PartyBMate` event class: only valid at `seq = 1`.
pub const PARTY_B_MATE: u8 = 0x01;

/// `Close` event class: terminal — nothing may follow this event.
pub const CLOSE: u8 = 0x02;

/// Validate a decoded two-party attestation chain against all four invariants.
///
/// `payloads` must be ordered oldest-first (PartyACommit first) exactly as
/// they appear in the on-chain UTXO chain.
///
/// `mate_proof` is required when the chain includes a `PartyBMate` event
/// (`seq = 1`). Pass `None` if the chain does not yet include that event;
/// if it does include it and `mate_proof` is `None`, validation fails with
/// [`Error::InvariantMateProofInvalid`].
///
/// An empty `payloads` slice is rejected — a valid chain must have at least
/// a `PartyACommit` event.
///
/// # Errors
///
/// Returns the first invariant violation encountered:
///
/// - [`Error::InvariantEmptyChain`] if `payloads` is empty.
/// - [`Error::InvariantUnknownEventClass`] for an unrecognised class byte (PA-3).
/// - [`Error::InvariantEventAfterClose`] if any event follows a `Close` (PA-3).
/// - [`Error::InvariantSeqGap`] if `seq` is not the expected next value (PA-1).
/// - [`Error::InvariantAttestationIdMismatch`] if `attestation_id` differs
///   from the first event (PA-2).
/// - [`Error::InvariantClassAtSeqZero`] if the class-at-seq rule is violated
///   (PA-3): `PartyACommit` at `seq ≠ 0`, or `PartyBMate`/`Close` at `seq = 0`.
/// - [`Error::InvariantMateNotAtSeqOne`] if `PartyBMate` appears at `seq ≠ 1`
///   (PA-3).
/// - [`Error::InvariantMateProofInvalid`] if the mate proof fails or is absent
///   when the `PartyBMate` event is present (PA-4).
pub fn validate_chain(payloads: &[Payload], mate_proof: Option<&MateProof>) -> Result<()> {
    if payloads.is_empty() {
        return Err(Error::InvariantEmptyChain);
    }

    let expected_attestation_id = payloads[0].attestation_id;
    let mut closed = false;

    for (i, p) in payloads.iter().enumerate() {
        let expected_seq = i as u64;

        // PA-3: reject unknown event classes first.
        if p.event_class != PARTY_A_COMMIT
            && p.event_class != PARTY_B_MATE
            && p.event_class != CLOSE
        {
            return Err(Error::InvariantUnknownEventClass {
                index: i,
                class: p.event_class,
            });
        }

        // PA-3: nothing may follow a Close.
        if closed {
            return Err(Error::InvariantEventAfterClose { index: i });
        }

        // PA-1: seq must be exactly i.
        if p.seq != expected_seq {
            return Err(Error::InvariantSeqGap {
                index: i,
                expected: expected_seq,
                got: p.seq,
            });
        }

        // PA-2: attestation_id must be identical throughout.
        if p.attestation_id != expected_attestation_id {
            return Err(Error::InvariantAttestationIdMismatch { index: i });
        }

        // PA-3: PartyACommit only at seq 0; non-PartyACommit must not be at seq 0.
        if p.event_class == PARTY_A_COMMIT && p.seq != 0 {
            return Err(Error::InvariantClassAtSeqZero { index: i });
        }
        if p.event_class != PARTY_A_COMMIT && p.seq == 0 {
            return Err(Error::InvariantClassAtSeqZero { index: i });
        }

        // PA-3: PartyBMate must appear at seq 1 only.
        if p.event_class == PARTY_B_MATE && p.seq != 1 {
            return Err(Error::InvariantMateNotAtSeqOne { index: i });
        }

        // PA-4: the seq-1 PartyBMate event must carry a valid mate proof.
        if p.event_class == PARTY_B_MATE && p.seq == 1 {
            match mate_proof {
                None => {
                    return Err(Error::InvariantMateProofInvalid {
                        index: i,
                        reason: "no mate proof supplied for PartyBMate event".into(),
                    });
                }
                Some(proof) => {
                    verify_mate(proof).map_err(|e| Error::InvariantMateProofInvalid {
                        index: i,
                        reason: e.to_string(),
                    })?;
                }
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
    use crate::mate::{build_mate_proof, recompute_commit_for_test};
    use crate::record::canonical_bytes;
    use serde_json::json;

    const AID: [u8; 32] = [0xaau8; 32];
    const COM: [u8; 32] = [0xccu8; 32];

    fn p(seq: u64, class: u8) -> Payload {
        Payload {
            attestation_id: AID,
            seq,
            event_class: class,
            commitment: COM,
        }
    }

    fn make_mate_proof() -> MateProof {
        let record = json!({"subject": "aabb", "terms_hash": "ccdd", "nonce": 1u64});
        let record_bytes = canonical_bytes(&record).unwrap();
        let blind_a = [0x11u8; 32];
        let blind_b = [0x22u8; 32];
        let commit_a = recompute_commit_for_test(&record_bytes, &blind_a);
        let commit_b = recompute_commit_for_test(&record_bytes, &blind_b);
        build_mate_proof(&record, blind_a, blind_b, commit_a, commit_b).unwrap()
    }

    // ---- positive tests ----

    #[test]
    fn valid_single_party_a_commit() {
        validate_chain(&[p(0, PARTY_A_COMMIT)], None).unwrap();
    }

    #[test]
    fn valid_party_a_then_party_b_mate() {
        let proof = make_mate_proof();
        // Use attestation_id that matches the proof's attestation_id.
        let chain = vec![
            Payload {
                attestation_id: proof.attestation_id,
                seq: 0,
                event_class: PARTY_A_COMMIT,
                commitment: proof.commit_a,
            },
            Payload {
                attestation_id: proof.attestation_id,
                seq: 1,
                event_class: PARTY_B_MATE,
                commitment: proof.commit_b,
            },
        ];
        validate_chain(&chain, Some(&proof)).unwrap();
    }

    #[test]
    fn valid_chain_with_close() {
        let proof = make_mate_proof();
        let chain = vec![
            Payload {
                attestation_id: proof.attestation_id,
                seq: 0,
                event_class: PARTY_A_COMMIT,
                commitment: proof.commit_a,
            },
            Payload {
                attestation_id: proof.attestation_id,
                seq: 1,
                event_class: PARTY_B_MATE,
                commitment: proof.commit_b,
            },
            Payload {
                attestation_id: proof.attestation_id,
                seq: 2,
                event_class: CLOSE,
                commitment: [0u8; 32],
            },
        ];
        validate_chain(&chain, Some(&proof)).unwrap();
    }

    // ---- empty chain ----

    #[test]
    fn empty_chain_rejected() {
        let err = validate_chain(&[], None).unwrap_err();
        assert!(matches!(err, Error::InvariantEmptyChain), "{err}");
    }

    // ---- PA-1: seq gap ----

    #[test]
    fn seq_skip_fails_pa1() {
        let proof = make_mate_proof();
        let chain = vec![
            Payload {
                attestation_id: proof.attestation_id,
                seq: 0,
                event_class: PARTY_A_COMMIT,
                commitment: COM,
            },
            // seq 2 skips 1
            Payload {
                attestation_id: proof.attestation_id,
                seq: 2,
                event_class: PARTY_B_MATE,
                commitment: COM,
            },
        ];
        let err = validate_chain(&chain, Some(&proof)).unwrap_err();
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

    // ---- PA-2: attestation_id mismatch ----

    #[test]
    fn attestation_id_mismatch_fails_pa2() {
        let mut bad = p(1, PARTY_B_MATE);
        bad.attestation_id = [0xbbu8; 32];
        let proof = make_mate_proof();
        let chain = vec![
            Payload {
                attestation_id: proof.attestation_id,
                seq: 0,
                event_class: PARTY_A_COMMIT,
                commitment: COM,
            },
            bad,
        ];
        let err = validate_chain(&chain, Some(&proof)).unwrap_err();
        assert!(
            matches!(err, Error::InvariantAttestationIdMismatch { index: 1 }),
            "{err}"
        );
    }

    // ---- PA-3: event-class rules ----

    #[test]
    fn unknown_event_class_rejected() {
        let err = validate_chain(
            &[Payload {
                attestation_id: AID,
                seq: 0,
                event_class: 0xff,
                commitment: COM,
            }],
            None,
        )
        .unwrap_err();
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
    fn party_b_mate_at_seq_zero_fails_pa3() {
        let err = validate_chain(&[p(0, PARTY_B_MATE)], None).unwrap_err();
        assert!(
            matches!(err, Error::InvariantClassAtSeqZero { index: 0 }),
            "{err}"
        );
    }

    #[test]
    fn close_at_seq_zero_fails_pa3() {
        let err = validate_chain(&[p(0, CLOSE)], None).unwrap_err();
        assert!(
            matches!(err, Error::InvariantClassAtSeqZero { index: 0 }),
            "{err}"
        );
    }

    #[test]
    fn event_after_close_fails_pa3() {
        let proof = make_mate_proof();
        let chain = vec![
            Payload {
                attestation_id: proof.attestation_id,
                seq: 0,
                event_class: PARTY_A_COMMIT,
                commitment: proof.commit_a,
            },
            Payload {
                attestation_id: proof.attestation_id,
                seq: 1,
                event_class: PARTY_B_MATE,
                commitment: proof.commit_b,
            },
            Payload {
                attestation_id: proof.attestation_id,
                seq: 2,
                event_class: CLOSE,
                commitment: [0u8; 32],
            },
            // spurious event after Close
            Payload {
                attestation_id: proof.attestation_id,
                seq: 3,
                event_class: PARTY_A_COMMIT,
                commitment: [0u8; 32],
            },
        ];
        let err = validate_chain(&chain, Some(&proof)).unwrap_err();
        assert!(
            matches!(err, Error::InvariantEventAfterClose { index: 3 }),
            "{err}"
        );
    }

    // ---- PA-4: mate proof ----

    #[test]
    fn missing_mate_proof_fails_pa4() {
        let proof = make_mate_proof();
        let chain = vec![
            Payload {
                attestation_id: proof.attestation_id,
                seq: 0,
                event_class: PARTY_A_COMMIT,
                commitment: proof.commit_a,
            },
            Payload {
                attestation_id: proof.attestation_id,
                seq: 1,
                event_class: PARTY_B_MATE,
                commitment: proof.commit_b,
            },
        ];
        let err = validate_chain(&chain, None).unwrap_err();
        assert!(
            matches!(err, Error::InvariantMateProofInvalid { index: 1, .. }),
            "{err}"
        );
    }

    #[test]
    fn invalid_mate_proof_fails_pa4() {
        let proof = make_mate_proof();
        let mut bad_proof = proof.clone();
        bad_proof.blind_a = [0xffu8; 32]; // corrupt the proof
        let chain = vec![
            Payload {
                attestation_id: proof.attestation_id,
                seq: 0,
                event_class: PARTY_A_COMMIT,
                commitment: proof.commit_a,
            },
            Payload {
                attestation_id: proof.attestation_id,
                seq: 1,
                event_class: PARTY_B_MATE,
                commitment: proof.commit_b,
            },
        ];
        let err = validate_chain(&chain, Some(&bad_proof)).unwrap_err();
        assert!(
            matches!(err, Error::InvariantMateProofInvalid { index: 1, .. }),
            "{err}"
        );
    }
}
