//! Auditor-style worked example: anchor a governor lifecycle onto a sealed
//! lineage, then independently re-verify the disclosed chain.
//!
//! ## What it demonstrates
//!
//! 1. A **publisher** drives a real 2-of-3 governor through its lifecycle
//!    (`Pending → Active → Passed → Executed`) using the governor's own methods,
//!    anchoring one sealed-lineage event per snapshot. The `lineage_id` is
//!    derived from the governor's immutable config; each event seals the full
//!    canonical `GovernorState`.
//! 2. An **auditor**, handed only the anchored steps (payload + disclosed
//!    `(state, blind)`), independently calls [`verify_governor_lineage`] and
//!    accepts the chain.
//! 3. **Tamper detection**: a doctored disclosure (here a swapped config) no
//!    longer derives its anchored `lineage_id`, so the auditor rejects it with a
//!    specific error.
//!
//! ## The boundary (honest scope)
//!
//! This example is a purely **off-chain** verifier; it never anchors anything.
//! On the default anchoring path a sealed lineage is written as plain
//! pay-to-address UTXOs, so consensus does not introspect the payload and the
//! chain invariants (append-only sequence, stable identity, event-class,
//! temporal) are validated off-chain only. Consensus rejects a malformed
//! successor **only if** the run is anchored under the separate covenant-bound
//! sealed-lineage chain (`[KCP-SL-003]`, demonstrated on testnet-10), which no
//! library API auto-wires. Even then it enforces the lineage *structure*, not
//! the governance *rules* (quorum before `Passed`, timelock before `Executed`,
//! signatory legality), which stay off-chain in the value types.
//!
//! The on-chain payloads are the trust anchor: obtain them from that lineage
//! independently of whoever discloses the states. `verify_governor_lineage`
//! alone proves only that a disclosed `(state, blind)` bundle is internally
//! consistent with its payloads.
//!
//! ## Usage
//! ```text
//! cargo run -p kcp-governance --example governance_lineage
//! ```
//! No node, no keys, no funds. Pure/offline.
//!
//! Status: **v0 — unaudited — testnet-only.**

use kcp_governance::action::TimelockAction;
use kcp_governance::error::GovernanceError;
use kcp_governance::governor::GovernorState;
use kcp_governance::lineage::{
    anchor_step, governor_lineage_id, verify_governor_lineage, AnchoredStep,
};
use kcp_governance::proposal::{GovernanceProposal, ProposalStatus};
use kcp_governance::vote::MultiSigVote;

type BoxError = Box<dyn std::error::Error>;

const T0: u64 = 1_750_000_000; // fixed synthetic epoch for a stable run

fn key(byte: u8) -> [u8; 32] {
    [byte; 32]
}

/// Deterministic per-step blind for the demo. A real publisher draws this from
/// a CSPRNG and discloses it off-band; here it is reproducible so the run is
/// stable.
fn demo_blind(seq: u64) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0] = 0xB1;
    b[1] = seq as u8;
    b
}

fn anchor(state: &GovernorState, lineage_id: [u8; 32], seq: u64) -> Result<AnchoredStep, BoxError> {
    Ok(AnchoredStep {
        payload: anchor_step(state, lineage_id, seq, T0 + seq * 86_400, &demo_blind(seq))?,
        state: state.clone(),
        blind: demo_blind(seq),
    })
}

/// The publisher drives a real governor through its lifecycle, anchoring one
/// step per snapshot.
fn publish_run() -> Result<(Vec<AnchoredStep>, [u8; 32]), BoxError> {
    let proposal = GovernanceProposal::new("fund the auditor", 100, 200)?;
    let vote = MultiSigVote::new(vec![key(1), key(2), key(3)], 2)?;
    let action = TimelockAction::new(50)?;
    let mut gov = GovernorState::new(proposal, vote, action, 90);

    let lineage_id = governor_lineage_id(&gov)?;
    let mut steps = Vec::new();

    // seq 0 — genesis, still Pending.
    steps.push(anchor(&gov, lineage_id, 0)?);

    // seq 1 — Active, first approval.
    gov.refresh_status(100);
    gov.approve(key(1), 100)?;
    steps.push(anchor(&gov, lineage_id, 1)?);

    // seq 2 — Active, second approval (quorum met).
    gov.approve(key(2), 150)?;
    steps.push(anchor(&gov, lineage_id, 2)?);

    // seq 3 — Passed (deadline reached with quorum).
    gov.refresh_status(200);
    steps.push(anchor(&gov, lineage_id, 3)?);

    // seq 4 — Passed, timelock scheduled.
    gov.schedule_action(200)?;
    steps.push(anchor(&gov, lineage_id, 4)?);

    // seq 5 — Executed (terminal → CLOSE).
    gov.execute(250)?;
    steps.push(anchor(&gov, lineage_id, 5)?);

    Ok((steps, lineage_id))
}

fn status_label(status: ProposalStatus) -> &'static str {
    match status {
        ProposalStatus::Pending => "Pending",
        ProposalStatus::Active => "Active",
        ProposalStatus::Passed => "Passed",
        ProposalStatus::Rejected => "Rejected",
        ProposalStatus::Executed => "Executed",
        ProposalStatus::Cancelled => "Cancelled",
    }
}

fn class_label(event_class: u8) -> &'static str {
    match event_class {
        0x00 => "GENESIS",
        0x02 => "CLOSE",
        _ => "APPEND",
    }
}

fn main() -> Result<(), BoxError> {
    println!("Kii Governance — lineage anchoring + auditor verification (in-library)");
    println!("{}", "=".repeat(72));

    let (steps, lineage_id) = publish_run()?;

    println!(
        "\n[1] Publisher anchored {} governor snapshots into lineage {}…",
        steps.len(),
        hex::encode(&lineage_id[..8])
    );
    println!(
        "    (each on-chain payload is the {}-byte KCPSL sealed-lineage wire form)",
        steps[0].payload.encode().len()
    );
    for s in &steps {
        println!(
            "    seq {}  {:<7}  {:<9}  commitment={}…",
            s.payload.seq,
            class_label(s.payload.event_class),
            status_label(s.state.status),
            hex::encode(&s.payload.commitment[..8])
        );
    }

    println!("\n[2] Auditor independently verifies the disclosed chain off-chain:");
    verify_governor_lineage(&steps)?;
    println!("    => VERIFY PASS (structure + identity + commitments + lattice)");

    println!("\n[3] Tamper detection — doctor one disclosed state (raise the threshold):");
    let mut tampered = publish_run()?.0;
    // The on-chain commitment is unchanged — that is what makes the tamper
    // detectable: the disclosed config no longer derives the anchored lineage_id.
    tampered[3].state.vote.threshold = 3;
    match verify_governor_lineage(&tampered) {
        Ok(()) => return Err("tampered chain must NOT verify".into()),
        Err(GovernanceError::LineageIdentityMismatch { index }) => {
            println!("    detected: lineage identity mismatch at step {index}");
            println!("    => VERIFY FAIL (config drift detected — correct)");
        }
        Err(e) => return Err(format!("unexpected error: {e}").into()),
    }

    println!("\nHonest scope:");
    println!("    Default anchoring is plain pay-to-address — the chain invariants and every");
    println!("    check above are validated OFF-CHAIN only. Consensus rejects a malformed");
    println!("    successor ONLY under the separate covenant-bound lineage ([KCP-SL-003]),");
    println!("    which no library API auto-wires. Obtain the payloads from that chain");
    println!("    independently of the discloser; this verifier only proves the disclosed");
    println!("    (state, blind) bundle is internally consistent with its payloads.");
    println!("    Not proven on-chain at all: quorum→Passed, timelock→Executed, signatory");
    println!("    legality — a dedicated Governor covenant is the remaining gap.");
    println!("\nv0 · unaudited · testnet-only · no node, no keys, no funds.");
    Ok(())
}
