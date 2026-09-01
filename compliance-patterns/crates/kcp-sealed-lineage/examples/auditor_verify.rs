//! Auditor-facing verification of a sealed reserve-attestation lineage.
//!
//! This is the in-library (Rust) counterpart to an earlier standalone Python
//! prototype (internal, pre-library). The difference that matters:
//! it verifies against the **real** `kcp_common::canonical` form and the real
//! 87-byte on-chain [`payload::Payload`] codec, so the commitments it checks are
//! byte-identical to what a live anchor carries. (The Python demo deliberately
//! used flat attestations to dodge canonicalisation edge cases; this closes that
//! gap — a production auditor client would share exactly this code path.)
//!
//! ## What it demonstrates
//!
//! 1. A synthetic custodian feed is sealed into an append-only lineage:
//!    `commitment = SHA-256(canonical_json(attestation) || blind)`
//!    (`record::commitment`), then packed into the on-chain wire payload
//!    (`Payload::encode`).
//! 2. An **auditor** is handed the on-chain payload bytes plus, off-band, the
//!    disclosed `(attestation, blind)` pairs. It independently:
//!    - decodes the wire payloads (`Payload::decode`);
//!    - checks lineage invariants L-1..L-4 (`invariants::validate_chain`);
//!    - binds each disclosed attestation to its anchored commitment by
//!      recomputing the seal;
//!    - confirms the anchored `lineage_id` derives from the genesis identity.
//! 3. Tamper detection: a doctored attestation no longer reproduces its
//!    anchored commitment, so the auditor rejects it.
//!
//! ## The firewall (what it deliberately does NOT do)
//!
//! It never holds, gates, or moves value — there is no withdrawal or
//! reserve-ratio logic. The on-chain enforcement that *only the oracle may
//! append* and that *sequence/identity cannot be forged* is the anchor-only
//! reserve covenant, engine-proven separately (pattern-library FACTS
//! `KCP-RE-001`). This example is the auditor's off-chain verifier; the Kaspa
//! anchor is the tamper-evidence layer beneath it.
//!
//! ## Usage
//! ```text
//! cargo run -p kcp-sealed-lineage --example auditor_verify
//! ```
//! No node, no keys, no funds. SYNTHETIC DATA ONLY.
//!
//! Status: **v0 — unaudited — synthetic data only.**

use kcp_sealed_lineage::invariants::{self, APPEND, GENESIS};
use kcp_sealed_lineage::payload::Payload;
use kcp_sealed_lineage::record;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type BoxError = Box<dyn std::error::Error>;

/// One lineage step as the publisher holds it: the on-chain wire bytes plus the
/// off-band disclosure `(attestation, blind)` an auditor is later handed.
struct Step {
    wire: Vec<u8>,
    attestation: Value,
    blind: [u8; 32],
}

/// Deterministic per-step blind for the demo. A real publisher draws this from
/// a CSPRNG and discloses it off-band; here it is reproducible so the run is
/// stable.
fn demo_blind(seq: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"kcp-reserve-demo-blind");
    h.update(seq.to_le_bytes());
    h.finalize().into()
}

/// Emit `n` SYNTHETIC reserve attestations. Numbers are fabricated and labelled.
fn synthetic_attestation(i: u64) -> Value {
    json!({
        "_synthetic": true,
        "as_of_day": i,
        "custodian": "SYNTHETIC-CUSTODIAN-A",
        "reserve_value_usd": 100_000_000u64 + i * 250_000,
        "token_supply": 99_500_000u64 + i * 200_000,
        "note": "SYNTHETIC — not real reserve data",
    })
}

/// Build the sealed lineage the way the publisher would, returning the on-chain
/// wire bytes + off-band disclosures for each step.
fn build_lineage(n: u64) -> Result<Vec<Step>, BoxError> {
    let genesis_identity = json!({ "product": "kii-reserve-demo", "lineage": "synthetic-A" });
    let lineage_id = record::lineage_id(&genesis_identity)?;
    let t0: u64 = 1_750_000_000; // fixed synthetic epoch for a stable run

    let mut steps = Vec::new();
    for seq in 0..n {
        let attestation = synthetic_attestation(seq);
        let blind = demo_blind(seq);
        let commitment = record::commitment(&attestation, &blind)?;
        let payload = Payload {
            lineage_id,
            seq,
            event_class: if seq == 0 { GENESIS } else { APPEND },
            t_bucket: t0 + seq * 86_400, // one day apart — within the L-4 envelope
            commitment,
        };
        steps.push(Step {
            wire: payload.encode(),
            attestation,
            blind,
        });
    }
    Ok(steps)
}

/// The auditor's independent off-chain verification of a disclosed chain.
/// Returns Ok(()) iff the chain is well-formed AND every disclosed attestation
/// reproduces its anchored commitment.
fn auditor_verify(steps: &[Step], expected_lineage_id: &[u8; 32]) -> Result<(), BoxError> {
    // 1. Decode the on-chain wire payloads (what the auditor reads off-chain).
    let payloads: Vec<Payload> = steps
        .iter()
        .map(|s| Payload::decode(&s.wire))
        .collect::<Result<_, _>>()?;

    // 2. Lineage invariants L-1..L-4 (sequence, identity, event-class, temporal).
    invariants::validate_chain(&payloads)?;

    // 3. Anchored lineage_id must derive from the genesis identity.
    if &payloads[0].lineage_id != expected_lineage_id {
        return Err("lineage_id does not match the genesis identity".into());
    }

    // 4. Bind each disclosed attestation to its anchored commitment.
    for (i, (s, p)) in steps.iter().zip(payloads.iter()).enumerate() {
        let recomputed = record::commitment(&s.attestation, &s.blind)?;
        if recomputed != p.commitment {
            return Err(format!(
                "SEAL MISMATCH @ seq {i}: disclosed attestation does not reproduce the anchored commitment"
            )
            .into());
        }
        println!(
            "    seq {i}: invariants OK  commitment={}…  reserve=${}",
            hex::encode(&p.commitment[..8]),
            s.attestation["reserve_value_usd"]
        );
    }
    Ok(())
}

fn main() -> Result<(), BoxError> {
    println!("Kii Reserve — auditor verification (in-library, real canonical form)");
    println!("{}", "=".repeat(72));

    let genesis_identity = json!({ "product": "kii-reserve-demo", "lineage": "synthetic-A" });
    let lineage_id = record::lineage_id(&genesis_identity)?;
    let steps = build_lineage(4)?;

    println!(
        "\n[1] Publisher sealed {} synthetic attestations into lineage {}…",
        steps.len(),
        hex::encode(&lineage_id[..8])
    );
    println!(
        "    (each on-chain payload is the {}-byte KCPSL wire form)",
        steps[0].wire.len()
    );

    println!("\n[2] Auditor independently verifies the disclosed chain off-chain:");
    auditor_verify(&steps, &lineage_id)?;
    println!("    => VERIFY PASS");

    println!("\n[3] Tamper detection — flip one attestation's reserve figure:");
    let mut tampered = build_lineage(4)?;
    // The auditor is handed a doctored attestation for seq 2 (the on-chain
    // commitment is unchanged — that is what makes the tamper detectable).
    tampered[2].attestation["reserve_value_usd"] = json!(
        tampered[2].attestation["reserve_value_usd"]
            .as_u64()
            .unwrap()
            + 1
    );
    match auditor_verify(&tampered, &lineage_id) {
        Ok(()) => return Err("tampered chain must NOT verify".into()),
        Err(e) => println!("    detected: {e}\n    => VERIFY FAIL (tampering detected — correct)"),
    }

    println!("\nOn-chain layer (out of scope for this off-chain verifier):");
    println!("    oracle-only append + unforgeable sequence/identity are enforced by the");
    println!("    anchor-only reserve covenant (engine-proven; FACTS KCP-RE-001).");
    println!("\nSYNTHETIC DATA ONLY · v0 · unaudited · no funds held or moved.");
    Ok(())
}
