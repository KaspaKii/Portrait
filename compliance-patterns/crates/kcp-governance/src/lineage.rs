//! Anchoring binding: map a [`GovernorState`] run onto a `kcp-sealed-lineage`
//! append-only lineage.
//!
//! This module is the seam between the pure governance value types and a
//! tamper-evident audit trail. It does **not** introduce a new covenant, and it
//! does **not** by itself put anything on-chain: it produces sealed-lineage
//! payloads and re-verifies them off-chain. A governor run is anchored as one
//! sealed event per lifecycle step.
//!
//! ## Enforcement honesty (read before relying on this)
//!
//! By default a sealed lineage is anchored with **plain pay-to-address** UTXOs
//! (`kcp_sealed_lineage::tx::{create,append}_lineage_tx`): control is by key
//! alone and **consensus does not introspect the payload**. On that default
//! path the chain invariants (L-1 sequence, L-2 identity, L-3 event-class, L-4
//! temporal) — and everything this module checks — are validated **off-chain
//! only**, exactly as the sibling `kcp-sealed-lineage` and
//! `kcp-transferable-record` modules state.
//!
//! Consensus rejects a malformed successor **only if** the run is anchored under
//! the separate covenant-id-bound sealed-lineage UTXO chain (`[KCP-SL-003]`,
//! demonstrated on testnet-10). No library API — including this one — wires a
//! governance payload into that covenant; doing so is a deliberate, manual
//! step. Treat the on-chain layer as *available*, not *automatic*.
//!
//! ## Trust anchor
//!
//! The on-chain payloads are the trust anchor: obtain them from the (covenant-
//! bound) lineage UTXO chain **independently of whoever discloses the states**.
//! [`verify_governor_lineage`] on its own proves only that a disclosed
//! `(state, blind)` bundle is internally consistent with its payloads — not that
//! the payloads are the ones that were actually anchored.
//!
//! ## What this binding checks (off-chain)
//!
//! - the sealed-lineage chain invariants L-1..L-4 (append-only sequence, stable
//!   identity, event-class rules, temporal envelope);
//! - the anchored `lineage_id` is derived from the governor's **immutable
//!   config** (proposal id, voting window, signatory set, threshold, timelock
//!   delay), so a config swap is detectable;
//! - each event's commitment seals the **full canonical `GovernorState`**, so a
//!   tampered disclosure is detectable;
//! - the lifecycle **status lattice** is well-formed: event classes match the
//!   status, successive statuses follow the documented transitions, and each
//!   disclosed state is internally consistent (a `Passed`/`Executed` state must
//!   show quorum; a `Rejected` state must not; an `Executed` state must have a
//!   scheduled timelock).
//!
//! ## What this binding does NOT prove
//!
//! Even under the covenant, consensus enforces the lineage **structure**, not
//! the governance **rules**. And this verifier's consistency checks read only
//! the disclosed state: they confirm a state is *self-consistent*, not that
//! quorum was actually reached from authorised signatories, that the timelock
//! delay truly elapsed, or that heights were honest. Those remain off-chain
//! properties of the value types; a dedicated Governor covenant that binds them
//! on-chain is the remaining gap (`KNOWN-ISSUES.md`).
//!
//! **Pure/offline.** This module depends on `kcp-sealed-lineage` default
//! features only (no `wrpc`); it never touches a node, keys, or funds.

use serde::Serialize;

use kcp_sealed_lineage::invariants::{self, APPEND, CLOSE, GENESIS};
use kcp_sealed_lineage::payload::Payload;
use kcp_sealed_lineage::record;

use crate::error::GovernanceError;
use crate::governor::GovernorState;
use crate::proposal::ProposalStatus;

/// The immutable-config identity body a governor lineage is keyed by.
///
/// Only fields that must not change across a governor's life appear here: the
/// content-addressed proposal id, the voting window, the signatory set, the
/// approval threshold, and the timelock delay. Mutable run state (accumulated
/// approvals, scheduling height, lifecycle status) is deliberately excluded, so
/// every step of a single run derives the same `lineage_id`.
#[derive(Serialize)]
struct GovernorIdentity {
    kind: &'static str,
    proposal_id: String,
    proposed_at_height: u64,
    voting_deadline: u64,
    signatories: Vec<String>,
    threshold: u8,
    timelock_min_delay: u64,
}

/// A single anchored governor step: the on-chain payload plus the off-band
/// disclosure (`state`, `blind`) an auditor is later handed to re-verify it.
#[derive(Debug, Clone)]
pub struct AnchoredStep {
    /// The 87-byte sealed-lineage payload as it appears on-chain.
    pub payload: Payload,
    /// The full governor state sealed by `payload.commitment`.
    pub state: GovernorState,
    /// The 32-byte blind under which `state` was sealed.
    pub blind: [u8; 32],
}

/// Derive the sealed-lineage `lineage_id` from a governor's **immutable config**.
///
/// The id is stable across an entire run (it excludes mutable run state) and
/// changes if any config field changes, so a config swap between two anchored
/// steps is detectable by [`verify_governor_lineage`].
///
/// # Errors
///
/// Returns [`GovernanceError::LineageSerialization`] if the identity body
/// cannot be canonicalised.
pub fn governor_lineage_id(state: &GovernorState) -> Result<[u8; 32], GovernanceError> {
    let identity = GovernorIdentity {
        kind: "kcp-governance/v0",
        proposal_id: hex::encode(state.proposal.id),
        proposed_at_height: state.proposal.proposed_at_height,
        voting_deadline: state.proposal.voting_deadline,
        signatories: state.vote.signatories.iter().map(hex::encode).collect(),
        threshold: state.vote.threshold,
        timelock_min_delay: state.action.minimum_delay,
    };
    record::lineage_id(&identity).map_err(|e| GovernanceError::LineageSerialization(e.to_string()))
}

/// Build the sealed-lineage [`Payload`] for one governor step.
///
/// The event class is `GENESIS` at `seq = 0`, `CLOSE` when the status is
/// terminal (`Executed` / `Rejected` / `Cancelled`), and `APPEND` otherwise.
/// The commitment seals the full canonical `GovernorState` under `blind` via
/// [`record::commitment`].
///
/// # Errors
///
/// Returns [`GovernanceError::LineageSerialization`] if the state cannot be
/// canonicalised for the commitment.
pub fn anchor_step(
    state: &GovernorState,
    lineage_id: [u8; 32],
    seq: u64,
    t_bucket: u64,
    blind: &[u8; 32],
) -> Result<Payload, GovernanceError> {
    let commitment = record::commitment(state, blind)
        .map_err(|e| GovernanceError::LineageSerialization(e.to_string()))?;
    Ok(Payload {
        lineage_id,
        seq,
        event_class: event_class_for(seq, state.status),
        t_bucket,
        commitment,
    })
}

/// Independently verify a disclosed governor lineage.
///
/// Runs exactly five checks, in order:
///
/// 1. the chain is non-empty;
/// 2. the sealed-lineage chain invariants hold
///    ([`invariants::validate_chain`]: L-1 sequence, L-2 identity, L-3
///    event-class rules, L-4 temporal envelope);
/// 3. each step's config-derived [`governor_lineage_id`] matches its anchored
///    `lineage_id`;
/// 4. each step's disclosed state reproduces its anchored commitment;
/// 5. each step's event class matches its status, each disclosed state is
///    internally consistent with its own status (quorum/timelock sanity), and
///    every successive status is a legal transition along the documented
///    lifecycle lattice.
///
/// This proves the disclosed `(state, blind)` bundle is internally consistent
/// with its payloads; per the module docs, obtain the payloads themselves from
/// the (covenant-bound) lineage independently of the discloser.
///
/// # Errors
///
/// - [`GovernanceError::LineageEmpty`] — check 1.
/// - [`GovernanceError::LineageChainInvalid`] — check 2.
/// - [`GovernanceError::LineageIdentityMismatch`] — check 3.
/// - [`GovernanceError::LineageCommitmentMismatch`] — check 4.
/// - [`GovernanceError::LineageIllegalTransition`] — check 5 (event-class or
///   lattice violation).
/// - [`GovernanceError::LineageStateInconsistent`] — check 5 (a state that
///   contradicts its own status).
/// - [`GovernanceError::LineageSerialization`] — if canonicalisation fails while
///   recomputing an id or commitment.
pub fn verify_governor_lineage(steps: &[AnchoredStep]) -> Result<(), GovernanceError> {
    // (1) A valid lineage has at least a genesis step.
    if steps.is_empty() {
        return Err(GovernanceError::LineageEmpty);
    }

    // (2) Sealed-lineage chain invariants (L-1..L-4) over the anchored payloads.
    let payloads: Vec<Payload> = steps.iter().map(|s| s.payload.clone()).collect();
    invariants::validate_chain(&payloads)
        .map_err(|e| GovernanceError::LineageChainInvalid(e.to_string()))?;

    // (3) Every step's immutable config must derive the anchored lineage_id.
    for (i, s) in steps.iter().enumerate() {
        if governor_lineage_id(&s.state)? != s.payload.lineage_id {
            return Err(GovernanceError::LineageIdentityMismatch { index: i });
        }
    }

    // (4) Every disclosed state must reproduce its anchored commitment.
    for (i, s) in steps.iter().enumerate() {
        let recomputed = record::commitment(&s.state, &s.blind)
            .map_err(|e| GovernanceError::LineageSerialization(e.to_string()))?;
        if recomputed != s.payload.commitment {
            return Err(GovernanceError::LineageCommitmentMismatch { index: i });
        }
    }

    // (5) Event classes must match the status, each disclosed state must be
    //     internally consistent with its status, and statuses must follow the
    //     documented lifecycle lattice.
    for (i, s) in steps.iter().enumerate() {
        if s.payload.event_class != event_class_for(s.payload.seq, s.state.status) {
            return Err(GovernanceError::LineageIllegalTransition { index: i });
        }
        if !state_is_self_consistent(&s.state) {
            return Err(GovernanceError::LineageStateInconsistent { index: i });
        }
        if i > 0 && !is_legal_successor(steps[i - 1].state.status, s.state.status) {
            return Err(GovernanceError::LineageIllegalTransition { index: i });
        }
    }

    Ok(())
}

/// Whether a disclosed state is internally consistent with its own status,
/// using only fields the discloser reveals. This does **not** prove the rules
/// were followed (see the module docs) — it rejects a state that contradicts
/// itself: a `Passed`/`Executed` state that does not show quorum, a `Rejected`
/// state that does, or an `Executed` state whose timelock was never scheduled.
fn state_is_self_consistent(state: &GovernorState) -> bool {
    let quorum = state.vote.quorum_met();
    match state.status {
        ProposalStatus::Passed => quorum,
        ProposalStatus::Executed => quorum && state.action.scheduled_at.is_some(),
        ProposalStatus::Rejected => !quorum,
        ProposalStatus::Pending | ProposalStatus::Active | ProposalStatus::Cancelled => true,
    }
}

/// The event class an anchored step must carry: `GENESIS` at `seq = 0`, `CLOSE`
/// for a terminal status, `APPEND` otherwise. Genesis takes precedence, so a
/// lineage that starts already-terminal is a genesis-only chain.
fn event_class_for(seq: u64, status: ProposalStatus) -> u8 {
    if seq == 0 {
        GENESIS
    } else if is_terminal(status) {
        CLOSE
    } else {
        APPEND
    }
}

/// A terminal lifecycle status — one that admits no successor. Mirrors the
/// documented lifecycle: `Executed` is final, `Rejected` is dead, `Cancelled`
/// cannot be revived. Terminal steps are anchored as `CLOSE`, which the L-3
/// chain invariant treats as final (off-chain here; in consensus only under the
/// covenant-bound lineage).
fn is_terminal(status: ProposalStatus) -> bool {
    matches!(
        status,
        ProposalStatus::Executed | ProposalStatus::Rejected | ProposalStatus::Cancelled
    )
}

/// Whether `cur` is a legal successor of `prev` along the documented governor
/// lifecycle lattice (`proposal.rs` / `governor.rs`):
///
/// ```text
/// Pending → Active        Active → Passed        Passed  → Executed
///                         Active → Rejected
/// Pending/Active/Passed → Cancelled   (explicit cancel from any non-terminal)
/// ```
///
/// A non-terminal status may also be its own successor: a run legitimately
/// anchors multiple snapshots within one status while sub-state evolves — each
/// accumulated approval while `Active`, or the timelock scheduling while
/// `Passed`. `refresh_status` is idempotent, so the status genuinely does not
/// change across those snapshots; the sealed states still differ. No transition
/// out of a terminal status is legal (terminal steps are `CLOSE`, which the L-3
/// invariant rejects a successor to regardless).
///
/// `Pending → Passed` and `Pending → Rejected` are also legal: a caller may
/// pre-load approvals and only sample the status after the window closes, so the
/// genesis-`Pending` snapshot can move straight to the resolved status in one
/// step. These are safe because the state-consistency check requires quorum on
/// `Passed` and non-quorum on `Rejected`.
fn is_legal_successor(prev: ProposalStatus, cur: ProposalStatus) -> bool {
    use ProposalStatus::{Active, Cancelled, Executed, Passed, Pending, Rejected};
    matches!(
        (prev, cur),
        // Intra-status evolution (non-terminal only).
        (Pending, Pending)
            | (Active, Active)
            | (Passed, Passed)
            // Documented forward edges.
            | (Pending, Active)
            | (Pending, Passed)
            | (Pending, Rejected)
            | (Active, Passed)
            | (Active, Rejected)
            | (Passed, Executed)
            // Explicit cancel from any non-terminal status.
            | (Pending, Cancelled)
            | (Active, Cancelled)
            | (Passed, Cancelled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::TimelockAction;
    use crate::proposal::GovernanceProposal;
    use crate::vote::MultiSigVote;

    const T0: u64 = 1_750_000_000;

    fn key(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    /// Deterministic per-step blind for reproducible tests.
    fn blind(seq: u64) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = 0xB1;
        b[1] = seq as u8;
        b
    }

    /// A fresh 2-of-3 governor, Pending at height 90.
    fn pending_governor() -> GovernorState {
        let proposal = GovernanceProposal::new("upgrade v2", 100, 200).unwrap();
        let vote = MultiSigVote::new(vec![key(1), key(2), key(3)], 2).unwrap();
        let action = TimelockAction::new(50).unwrap();
        GovernorState::new(proposal, vote, action, 90)
    }

    fn step(state: &GovernorState, lineage_id: [u8; 32], seq: u64) -> AnchoredStep {
        AnchoredStep {
            payload: anchor_step(state, lineage_id, seq, T0 + seq * 1000, &blind(seq)).unwrap(),
            state: state.clone(),
            blind: blind(seq),
        }
    }

    /// Drive a real governor through its full lifecycle, anchoring one step per
    /// snapshot: Pending → Active(+1) → Active(+2) → Passed → Passed(scheduled)
    /// → Executed. Returns the anchored steps and the lineage id.
    fn happy_run() -> (Vec<AnchoredStep>, [u8; 32]) {
        let mut gov = pending_governor();
        let lineage_id = governor_lineage_id(&gov).unwrap();
        let mut steps = Vec::new();

        // seq 0 — genesis, still Pending.
        steps.push(step(&gov, lineage_id, 0));

        // seq 1 — Active, first approval.
        gov.refresh_status(100);
        gov.approve(key(1), 100).unwrap();
        steps.push(step(&gov, lineage_id, 1));

        // seq 2 — Active, second approval (quorum met; Active → Active).
        gov.approve(key(2), 150).unwrap();
        steps.push(step(&gov, lineage_id, 2));

        // seq 3 — Passed (deadline reached with quorum).
        gov.refresh_status(200);
        assert_eq!(gov.status, ProposalStatus::Passed);
        steps.push(step(&gov, lineage_id, 3));

        // seq 4 — Passed, timelock scheduled (Passed → Passed).
        gov.schedule_action(200).unwrap();
        steps.push(step(&gov, lineage_id, 4));

        // seq 5 — Executed (terminal → CLOSE).
        gov.execute(250).unwrap();
        assert_eq!(gov.status, ProposalStatus::Executed);
        steps.push(step(&gov, lineage_id, 5));

        (steps, lineage_id)
    }

    #[test]
    fn lineage_id_stable_and_config_sensitive() {
        let mut gov = pending_governor();
        let base = governor_lineage_id(&gov).unwrap();

        // Stable across an entire run: mutable state does not move the id.
        gov.refresh_status(100);
        gov.approve(key(1), 100).unwrap();
        gov.approve(key(2), 100).unwrap();
        gov.refresh_status(200);
        gov.schedule_action(200).unwrap();
        assert_eq!(governor_lineage_id(&gov).unwrap(), base);

        // Sensitive to every immutable-config field.
        let diff_threshold = {
            let proposal = GovernanceProposal::new("upgrade v2", 100, 200).unwrap();
            let vote = MultiSigVote::new(vec![key(1), key(2), key(3)], 3).unwrap();
            GovernorState::new(proposal, vote, TimelockAction::new(50).unwrap(), 90)
        };
        assert_ne!(governor_lineage_id(&diff_threshold).unwrap(), base);

        let diff_signatories = {
            let proposal = GovernanceProposal::new("upgrade v2", 100, 200).unwrap();
            let vote = MultiSigVote::new(vec![key(1), key(2), key(9)], 2).unwrap();
            GovernorState::new(proposal, vote, TimelockAction::new(50).unwrap(), 90)
        };
        assert_ne!(governor_lineage_id(&diff_signatories).unwrap(), base);

        let diff_window = {
            let proposal = GovernanceProposal::new("upgrade v2", 100, 300).unwrap();
            let vote = MultiSigVote::new(vec![key(1), key(2), key(3)], 2).unwrap();
            GovernorState::new(proposal, vote, TimelockAction::new(50).unwrap(), 90)
        };
        assert_ne!(governor_lineage_id(&diff_window).unwrap(), base);

        let diff_delay = {
            let proposal = GovernanceProposal::new("upgrade v2", 100, 200).unwrap();
            let vote = MultiSigVote::new(vec![key(1), key(2), key(3)], 2).unwrap();
            GovernorState::new(proposal, vote, TimelockAction::new(99).unwrap(), 90)
        };
        assert_ne!(governor_lineage_id(&diff_delay).unwrap(), base);
    }

    #[test]
    fn anchor_step_event_classes() {
        let gov = pending_governor();
        let lineage_id = governor_lineage_id(&gov).unwrap();

        // seq 0 → GENESIS regardless of status.
        assert_eq!(
            anchor_step(&gov, lineage_id, 0, T0, &blind(0))
                .unwrap()
                .event_class,
            GENESIS
        );

        // Non-terminal status at seq > 0 → APPEND.
        let mut active = gov.clone();
        active.status = ProposalStatus::Active;
        assert_eq!(
            anchor_step(&active, lineage_id, 1, T0, &blind(1))
                .unwrap()
                .event_class,
            APPEND
        );

        // Each terminal status at seq > 0 → CLOSE.
        for terminal in [
            ProposalStatus::Executed,
            ProposalStatus::Rejected,
            ProposalStatus::Cancelled,
        ] {
            let mut g = gov.clone();
            g.status = terminal;
            assert_eq!(
                anchor_step(&g, lineage_id, 2, T0, &blind(2))
                    .unwrap()
                    .event_class,
                CLOSE
            );
        }
    }

    #[test]
    fn happy_path_round_trip_verifies() {
        let (steps, _) = happy_run();
        // Non-vacuity: a genuine multi-status run built through the governor's
        // own methods verifies clean.
        assert_eq!(steps.len(), 6);
        assert_eq!(steps[0].payload.event_class, GENESIS);
        assert_eq!(steps[5].payload.event_class, CLOSE);
        verify_governor_lineage(&steps).unwrap();
    }

    #[test]
    fn tamper_in_disclosed_state_detected() {
        let (mut steps, _) = happy_run();
        // Doctor a disclosed state without touching the anchored commitment.
        // `description` is sealed by the commitment but not by the lineage_id,
        // so the identity check passes and the commitment check fires.
        steps[2].state.proposal.description = "malicious swap".to_string();
        let err = verify_governor_lineage(&steps).unwrap_err();
        assert_eq!(err, GovernanceError::LineageCommitmentMismatch { index: 2 });
    }

    #[test]
    fn config_drift_detected() {
        let (mut steps, _) = happy_run();
        // Swap the immutable config on one disclosed step (raise the threshold).
        // The derived lineage_id no longer matches the anchored one.
        steps[3].state.vote.threshold = 3;
        let err = verify_governor_lineage(&steps).unwrap_err();
        assert_eq!(err, GovernanceError::LineageIdentityMismatch { index: 3 });
    }

    #[test]
    fn reordered_or_dropped_step_rejected() {
        let (mut steps, _) = happy_run();
        // Drop the seq-2 step: the chain now has a sequence gap (L-1).
        steps.remove(2);
        let err = verify_governor_lineage(&steps).unwrap_err();
        assert!(
            matches!(err, GovernanceError::LineageChainInvalid(_)),
            "{err}"
        );
    }

    #[test]
    fn event_after_close_rejected() {
        let mut gov = pending_governor();
        let lineage_id = governor_lineage_id(&gov).unwrap();
        let s0 = step(&gov, lineage_id, 0);

        // Cancel → terminal → CLOSE at seq 1.
        gov.cancel().unwrap();
        let s1 = step(&gov, lineage_id, 1);
        assert_eq!(s1.payload.event_class, CLOSE);

        // A further APPEND after the CLOSE (L-3 violation).
        let mut post = pending_governor();
        post.refresh_status(100);
        let s2 = step(&post, lineage_id, 2);

        let err = verify_governor_lineage(&[s0, s1, s2]).unwrap_err();
        assert!(
            matches!(err, GovernanceError::LineageChainInvalid(_)),
            "{err}"
        );
    }

    #[test]
    fn illegal_status_transition_rejected() {
        // A run that jumps Active → Executed, skipping the Passed/timelock gate.
        // (Executed → Active, as a literal regression, is caught earlier by the
        // event-after-close invariant; to isolate check 5's successor rule we
        // use an illegal *forward* skip that still passes the chain invariants.)
        let mut gov = pending_governor();
        let lineage_id = governor_lineage_id(&gov).unwrap();
        let s0 = step(&gov, lineage_id, 0);

        gov.refresh_status(100);
        gov.approve(key(1), 100).unwrap();
        gov.approve(key(2), 100).unwrap();
        let s1 = step(&gov, lineage_id, 1); // Active
        assert_eq!(gov.status, ProposalStatus::Active);

        // Hand-craft the illegal jump to Executed. Make the state internally
        // consistent (quorum + scheduled) so the failure isolates the illegal
        // *transition*, not the state-consistency check.
        let mut jumped = gov.clone();
        jumped.status = ProposalStatus::Executed;
        jumped.action.scheduled_at = Some(200);
        let s2 = step(&jumped, lineage_id, 2); // CLOSE, but Active → Executed

        let err = verify_governor_lineage(&[s0, s1, s2]).unwrap_err();
        assert_eq!(err, GovernanceError::LineageIllegalTransition { index: 2 });
    }

    #[test]
    fn passed_without_quorum_rejected() {
        // A disclosed state that claims `Passed` but shows no quorum contradicts
        // itself — caught by the state-consistency check even though the class
        // and the Pending → Passed transition are otherwise legal.
        let mut gov = pending_governor();
        let lineage_id = governor_lineage_id(&gov).unwrap();
        let s0 = step(&gov, lineage_id, 0); // Pending genesis

        gov.status = ProposalStatus::Passed; // no approvals recorded
        assert!(!gov.vote.quorum_met());
        let s1 = step(&gov, lineage_id, 1);

        let err = verify_governor_lineage(&[s0, s1]).unwrap_err();
        assert_eq!(err, GovernanceError::LineageStateInconsistent { index: 1 });
    }

    #[test]
    fn pending_to_passed_with_quorum_verifies() {
        // Approvals pre-loaded, status sampled only after the window closes: the
        // genesis-Pending snapshot moves straight to Passed in one honest step.
        let mut vote = MultiSigVote::new(vec![key(1), key(2), key(3)], 2).unwrap();
        vote.approve(key(1)).unwrap();
        vote.approve(key(2)).unwrap(); // quorum met
        let proposal = GovernanceProposal::new("upgrade v2", 100, 200).unwrap();
        let action = TimelockAction::new(50).unwrap();
        let mut gov = GovernorState::new(proposal, vote, action, 90); // Pending

        let lineage_id = governor_lineage_id(&gov).unwrap();
        let s0 = step(&gov, lineage_id, 0); // Pending genesis

        gov.refresh_status(200); // window closed, quorum → Passed
        assert_eq!(gov.status, ProposalStatus::Passed);
        let s1 = step(&gov, lineage_id, 1);

        verify_governor_lineage(&[s0, s1]).unwrap();
    }

    #[test]
    fn empty_chain_rejected() {
        let err = verify_governor_lineage(&[]).unwrap_err();
        assert_eq!(err, GovernanceError::LineageEmpty);
    }
}
