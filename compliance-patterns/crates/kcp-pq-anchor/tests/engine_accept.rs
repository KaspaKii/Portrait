//! Offline engine-acceptance test for the KIP-16 tag-0x21 redeem script.
//!
//! This is the soundness anchor for the cross-layer settlement story: it runs
//! the script produced by [`build_pq_anchor_redeem`] through the **real** pinned
//! consensus VM (`rusty-kaspa` tag `v2.0.0`, commit `90dbf07`):
//! `TxScriptEngine` → `OpZkPrecompile` (`0xa6`) → `R0SuccinctPrecompile::verify_zk`
//! → `risc0_zkp::verify::verify`, using a genuine RISC Zero succinct proof.
//!
//! Three checks, all run offline (no network, no wallet):
//!   * `engine_accepts_valid_proof` — a real succinct proof with matching claim,
//!     control inclusion proof, seal, journal, image id and control id is
//!     ACCEPTED by the in-consensus verifier. This proves our field order and
//!     encoding match the engine.
//!   * `engine_rejects_tampered_journal` — flipping one byte of the journal
//!     breaks the claim binding (`compute_assert_claim`) and the engine REJECTS.
//!     This is the sound negative control: a covenant gated on this script only
//!     accepts a spend because a valid proof bound *this* journal; a wrong
//!     journal cannot pass.
//!   * `engine_rejects_tampered_image_id` — same, for a wrong image id (the
//!     vProg the covenant pins).
//!
//! The proof fixtures in `tests/fixtures/succinct.*.hex` are copied verbatim
//! from the engine's own acceptance vectors
//! (`crypto/txscript/src/zk_precompiles/tests/data/` at commit `90dbf07`), so
//! the valid case is identical to upstream consensus CI — only the script
//! *assembly* under test is ours.

use kaspa_consensus_core::hashing::sighash::SigHashReusedValuesUnsync;
use kaspa_consensus_core::tx::PopulatedTransaction;
use kaspa_txscript::caches::Cache;
use kaspa_txscript::{EngineFlags, TxScriptEngine};

use kcp_pq_anchor::anchor_script::{build_pq_anchor_redeem, PqAnchorScriptFields};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let hexed =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
    hex::decode(hexed.trim()).unwrap_or_else(|e| panic!("decode fixture {name}: {e}"))
}

fn arr32(name: &str) -> [u8; 32] {
    fixture(name).try_into().expect("fixture is 32 bytes")
}

/// Load the real succinct proof fields into a `PqAnchorScriptFields`.
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

/// Run a standalone script through the real consensus VM with covenants enabled.
fn run_engine(script: &[u8]) -> Result<(), String> {
    let sig_cache = Cache::new(0);
    let reused = SigHashReusedValuesUnsync::new();
    let mut vm = TxScriptEngine::<PopulatedTransaction, SigHashReusedValuesUnsync>::from_script(
        script,
        &reused,
        &sig_cache,
        EngineFlags {
            covenants_enabled: true,
            ..Default::default()
        },
    );
    vm.execute().map_err(|e| format!("{e:?}"))
}

#[test]
fn engine_accepts_valid_proof() {
    let fields = real_proof_fields();
    let script = build_pq_anchor_redeem(&fields).expect("assemble redeem script");
    run_engine(&script).expect("real succinct proof must be ACCEPTED by the v2.0.0 engine");
}

#[test]
fn engine_rejects_tampered_journal() {
    let mut fields = real_proof_fields();
    // Tamper a single byte of the journal: the claim binding in the receipt
    // (compute_assert_claim) is over the original journal, so this must fail.
    fields.journal[0] ^= 0x01;
    let script = build_pq_anchor_redeem(&fields).expect("assemble redeem script");
    assert!(
        run_engine(&script).is_err(),
        "tampered journal must be REJECTED by the engine, but it was accepted"
    );
}

#[test]
fn engine_rejects_tampered_image_id() {
    let mut fields = real_proof_fields();
    fields.image_id[0] ^= 0x01;
    let script = build_pq_anchor_redeem(&fields).expect("assemble redeem script");
    assert!(
        run_engine(&script).is_err(),
        "wrong image id must be REJECTED by the engine"
    );
}
