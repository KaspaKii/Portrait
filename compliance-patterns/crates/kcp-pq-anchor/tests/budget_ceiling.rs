//! Does a tag-0x21 spend actually fit the budget a version-0 input can commit?
//!
//! `sigop_count_for_pq_verify()` returns 255 because that is the **maximum** a
//! `u8` `SigopCount` can express, capping execution at
//! `MAX_COMMITTABLE_SCRIPT_UNITS` = 25,509,999 script units. Nothing measured
//! that until this test: `engine_accept.rs` runs the redeem as a standalone
//! script (`from_script`), with no transaction, no P2SH wrap and no compute
//! commitment, so it cannot see the cost the node would charge.
//!
//! This test runs the shipped `tests/fixtures/succinct.*` proof through the
//! pinned v2.0.0 VM as a **real P2SH transaction input** and prints the margin.
//! It matters because the proof fields live inside the redeem script, so they
//! are inside the P2SH address: a spend that cannot be budgeted has no
//! alternative path and the funds are permanently unrecoverable.
//!
//! Run with `cargo test -p kcp-pq-anchor --test budget_ceiling -- --nocapture`
//! to see the measured cost and headroom.

use kcp_pq_anchor::anchor_script::{build_pq_anchor_redeem, PqAnchorScriptFields};
use kcp_pq_anchor::sigop::{
    fits_pq_verify_budget, measure_pq_anchor_units, sigop_count_for_pq_verify,
    MAX_COMMITTABLE_SCRIPT_UNITS,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let hexed =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
    hex::decode(hexed.trim()).unwrap_or_else(|e| panic!("decode fixture {name}: {e}"))
}

fn arr32(name: &str) -> [u8; 32] {
    fixture(name).try_into().expect("fixture is 32 bytes")
}

fn real_proof_fields() -> PqAnchorScriptFields {
    PqAnchorScriptFields {
        claim: fixture("succinct.claim.hex"),
        control_index: u32::from_le_bytes(
            fixture("succinct.control_index.hex")
                .try_into()
                .expect("control_index is 4 bytes"),
        ),
        control_digests: fixture("succinct.control_digests.hex"),
        seal: fixture("succinct.seal.hex"),
        journal: arr32("succinct.journal.hex"),
        image_id: arr32("succinct.image.hex"),
        control_id: arr32("succinct.control_id.hex"),
    }
}

#[test]
fn max_committable_units_is_the_u8_sigop_ceiling() {
    assert_eq!(sigop_count_for_pq_verify(), u8::MAX);
    assert_eq!(MAX_COMMITTABLE_SCRIPT_UNITS, 25_509_999);
}

#[test]
fn reference_proof_fits_the_committable_budget_with_measured_margin() {
    let redeem = build_pq_anchor_redeem(&real_proof_fields()).expect("assemble redeem script");
    let used = measure_pq_anchor_units(&redeem).expect("engine must accept the reference proof");

    let margin = MAX_COMMITTABLE_SCRIPT_UNITS.saturating_sub(used);
    let pct = (margin as f64) * 100.0 / (MAX_COMMITTABLE_SCRIPT_UNITS as f64);
    println!(
        "tag-0x21 reference spend: {used} script units used, ceiling \
         {MAX_COMMITTABLE_SCRIPT_UNITS}, margin {margin} ({pct:.2}% headroom); \
         redeem script {} bytes",
        redeem.len()
    );

    assert!(
        fits_pq_verify_budget(used),
        "the reference proof costs {used} units, above the {MAX_COMMITTABLE_SCRIPT_UNITS} \
         a u8 SigopCount can commit — a spend of this shape could never be budgeted"
    );
}

#[test]
fn a_spend_over_the_ceiling_is_reported_as_unbudgetable() {
    assert!(!fits_pq_verify_budget(MAX_COMMITTABLE_SCRIPT_UNITS + 1));
    assert!(fits_pq_verify_budget(MAX_COMMITTABLE_SCRIPT_UNITS));
}
