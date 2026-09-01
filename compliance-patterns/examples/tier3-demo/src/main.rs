//! Tier 3 Demo — cross-layer binding between L1 covenant and vProg STARK.
//!
//! This binary demonstrates the three-stage Tier 3 pipeline:
//!
//!   Stage A — Covenant (L1):
//!     ComplianceToken.portrait → portrait engrave → ComplianceToken.sil → silverc → KovId
//!
//!   Stage B — vProg (off-L1):
//!     ComplianceToken.portrait → portrait atelier-build → compliancetoken_guest_main.rs
//!     → cargo build (in kii-csci-prover guest) → RISC Zero STARK proof
//!
//!   Stage C — Binding:
//!     STARK proof journal (104 bytes) → CovIdBinding → verify_kov_id + verify_seq_advance
//!
//! This demo runs Stage C offline using a simulated DEV_MODE journal, exercising the
//! kcp-csci binding types without requiring a funded wallet or live STARK proof.
//!
//! Pre-production, unaudited, testnet-only.

use hex;
use kcp_csci::{CovIdBinding, KovId};
use sha2::{Digest, Sha256};

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

fn main() {
    println!("=== Tier 3 Cross-Layer Binding Demo ===");
    println!("  Stage A: Covenant (L1) — ComplianceToken.portrait → portrait engrave → silverc");
    println!("  Stage B: vProg (off-L1) — portrait atelier-build → guest_main.rs → STARK proof");
    println!("  Stage C: Binding — journal[104] → CovIdBinding → verify");
    println!();

    // ─── Stage A: KovId ──────────────────────────────────────────────────────
    // In production: KovId = sha256(silverc_bytecode || ctor_args_cbor)
    // silverc does not yet expose a --output-id flag; derive the real KovId by
    // reading the `bytecode` field from the compiled JSON (`silverc -c <file>`)
    // and sha256-hashing it. This is flagged as a gap in KNOWN-ISSUES.md.
    // Here we use a representative placeholder to demonstrate the binding concept.
    let covenant_bytecode_placeholder = b"ComplianceToken_v0.1.0_testnet";
    let kov_id_bytes = sha256(covenant_bytecode_placeholder);
    let kov_id = KovId(kov_id_bytes);

    println!("[Stage A] KovId (sha256 of covenant bytecode):");
    println!("  {}", hex::encode(kov_id_bytes));
    println!();

    // ─── Stage B: vProg journal ───────────────────────────────────────────────
    // Simulate the 104-byte journal that the RISC Zero guest produces.
    // In production: read from proof-export/dev/journal.hex (DEV_MODE) or
    // extract from the succinct STARK receipt (full proof).
    //
    // Journal schema: covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]

    let initial_balance: i64 = 1_000;
    let transfer_amount: i64 = 150;
    let new_balance: i64 = initial_balance - transfer_amount;

    let new_state_bytes = new_balance.to_le_bytes();
    let new_state_hash = sha256(&new_state_bytes);

    let rule_name = b"verify_compliance";
    let rule_hash = sha256(rule_name);

    let seq: u64 = 1;

    let mut journal = [0u8; 104];
    journal[0..32].copy_from_slice(&kov_id_bytes);
    journal[32..64].copy_from_slice(&new_state_hash);
    journal[64..96].copy_from_slice(&rule_hash);
    journal[96..104].copy_from_slice(&seq.to_le_bytes());

    println!("[Stage B] Simulated vProg journal (DEV_MODE equivalent):");
    println!("  covenant_id    = {}", hex::encode(&journal[0..32]));
    println!("  new_state_hash = {}", hex::encode(&journal[32..64]));
    println!("    (sha256(balance={} LE bytes))", new_balance);
    println!("  rule_hash      = {}", hex::encode(&journal[64..96]));
    println!("    (sha256(\"{}\"))", std::str::from_utf8(rule_name).unwrap());
    println!("  seq            = {}", seq);
    println!("  total          = {} bytes", journal.len());
    println!();

    // ─── Stage C: Binding verification ───────────────────────────────────────
    let binding = CovIdBinding::from_journal(&journal);

    println!("[Stage C] CovIdBinding parsed:");
    println!("  kov_id OK:       {}", binding.verify_kov_id(&kov_id));
    println!("  seq advance OK:  {}", binding.verify_seq_advance(0)); // 1 > 0 ✓
    println!("  seq replay FAIL: {}", binding.verify_seq_advance(1)); // 1 > 1 ✗ (expected false)
    println!();

    // Negative control: wrong KovId should fail.
    let wrong_kov_id = KovId([0xFF; 32]);
    let binding_fails_wrong_id = !binding.verify_kov_id(&wrong_kov_id);
    println!("[Negative control] Wrong KovId rejected: {}", binding_fails_wrong_id);
    println!();

    // Roundtrip: to_journal() should reproduce the original.
    let j2 = binding.to_journal();
    let roundtrip_ok = j2 == journal;
    println!("[Roundtrip] journal → CovIdBinding → journal: {}", roundtrip_ok);
    println!();

    // ─── Summary ─────────────────────────────────────────────────────────────
    let all_ok = binding.verify_kov_id(&kov_id)
        && binding.verify_seq_advance(0)
        && !binding.verify_seq_advance(1)
        && binding_fails_wrong_id
        && roundtrip_ok;

    if all_ok {
        println!("✓ PASSED — Tier 3 cross-layer binding demo: all checks correct");
        println!();
        println!("Next steps to complete the Tier 3 pipeline:");
        println!("  1. Fund wallet kaspatest:qqh73x8... (operational step)");
        println!("  2. cd kii-csci-prover && cargo run --release");
        println!("     → generates real STARK proof (5-15 min CPU)");
        println!("  3. Deploy ComplianceToken covenant to TN10");
        println!("     → records KovId on-chain");
        println!("  4. Submit STARK proof + journal to covenant input");
        println!("     → OpZkPrecompile tag 0x21 verifies binding");
    } else {
        eprintln!("✗ FAILED — one or more binding checks failed");
        std::process::exit(1);
    }
}
