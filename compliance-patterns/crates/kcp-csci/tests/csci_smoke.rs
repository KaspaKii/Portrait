//! Smoke tests for the CSCI state encoding and transition logic.

use kcp_csci::{CsciState, CsciStateTransition};
use sha2::{Digest, Sha256};

fn owner(b: u8) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0] = b;
    k
}

const RULE_HASH: [u8; 32] = [0xAAu8; 32];
const COVENANT_ID: [u8; 32] = [0xBBu8; 32];

fn genesis() -> CsciState {
    CsciState::new_genesis(owner(0x01), 1_000_000, RULE_HASH, COVENANT_ID)
}

// ── State encoding ─────────────────────────────────────────────────────────

#[test]
fn genesis_seq_is_zero() {
    assert_eq!(genesis().seq, 0);
}

#[test]
fn encode_length_is_50() {
    assert_eq!(genesis().encode().len(), 50);
}

#[test]
fn encode_seq_at_offset_42() {
    let state = genesis();
    let enc = state.encode();
    let seq_bytes: [u8; 8] = enc[42..50].try_into().unwrap();
    assert_eq!(u64::from_le_bytes(seq_bytes), 0);
}

#[test]
fn state_hash_is_sha256_of_encode() {
    let state = genesis();
    let expected: [u8; 32] = Sha256::digest(state.encode()).into();
    assert_eq!(state.state_hash(), expected);
}

// ── Transfer ───────────────────────────────────────────────────────────────

#[test]
fn transfer_increments_seq() {
    let prev = genesis();
    let tx = CsciStateTransition::transfer(&prev, owner(0x02), 500_000, RULE_HASH).unwrap();
    assert_eq!(tx.new_state.seq, 1);
}

#[test]
fn transfer_updates_owner() {
    let prev = genesis();
    let new_owner = owner(0x02);
    let tx = CsciStateTransition::transfer(&prev, new_owner, 500_000, RULE_HASH).unwrap();
    assert_eq!(tx.new_state.ktt.owner_identifier, new_owner);
}

#[test]
fn transfer_zero_amount_rejected() {
    let prev = genesis();
    assert!(matches!(
        CsciStateTransition::transfer(&prev, owner(0x02), 0, RULE_HASH),
        Err(kcp_csci::CsciError::ZeroAmount)
    ));
}

#[test]
fn transfer_excess_amount_rejected() {
    let prev = genesis(); // amount = 1_000_000
    assert!(matches!(
        CsciStateTransition::transfer(&prev, owner(0x02), 2_000_000, RULE_HASH),
        Err(kcp_csci::CsciError::InsufficientBalance { .. })
    ));
}

#[test]
fn transfer_wrong_rule_hash_rejected() {
    let prev = genesis();
    let wrong_rule = [0xFFu8; 32];
    assert!(matches!(
        CsciStateTransition::transfer(&prev, owner(0x02), 100, wrong_rule),
        Err(kcp_csci::CsciError::RuleHashMismatch { .. })
    ));
}

// ── Journal encoding ───────────────────────────────────────────────────────

#[test]
fn journal_bytes_length_is_104() {
    let prev = genesis();
    let tx = CsciStateTransition::transfer(&prev, owner(0x02), 500_000, RULE_HASH).unwrap();
    assert_eq!(tx.journal_bytes().len(), 104);
}

#[test]
fn journal_hash_matches_sha256_of_bytes() {
    let prev = genesis();
    let tx = CsciStateTransition::transfer(&prev, owner(0x02), 500_000, RULE_HASH).unwrap();
    let bytes = tx.journal_bytes();
    let expected: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(tx.journal_hash(), expected);
}

#[test]
fn journal_bytes_encodes_covenant_id_at_offset_0() {
    let prev = genesis();
    let tx = CsciStateTransition::transfer(&prev, owner(0x02), 500_000, RULE_HASH).unwrap();
    let bytes = tx.journal_bytes();
    let cid: [u8; 32] = bytes[0..32].try_into().unwrap();
    assert_eq!(cid, COVENANT_ID);
}

#[test]
fn journal_bytes_encodes_rule_hash_at_offset_64() {
    let prev = genesis();
    let tx = CsciStateTransition::transfer(&prev, owner(0x02), 500_000, RULE_HASH).unwrap();
    let bytes = tx.journal_bytes();
    let rh: [u8; 32] = bytes[64..96].try_into().unwrap();
    assert_eq!(rh, RULE_HASH);
}

#[test]
fn journal_bytes_encodes_seq_at_offset_96() {
    let prev = genesis();
    let tx = CsciStateTransition::transfer(&prev, owner(0x02), 500_000, RULE_HASH).unwrap();
    let bytes = tx.journal_bytes();
    let seq_bytes: [u8; 8] = bytes[96..104].try_into().unwrap();
    assert_eq!(u64::from_le_bytes(seq_bytes), 1);
}

// ── Guest ↔ library agreement (csci-guest in kii-csci-prover) ────────────────
//
// The RISC Zero guest (kii-csci-prover/methods/csci-guest) computes the CSCI
// transition INSIDE the zkVM and commits the 104-byte journal. It MUST produce
// byte-identical new_state_hash and rule_hash to this library's CsciState /
// CsciStateTransition for the SAME inputs, or the on-chain journal the proof
// binds would not match what the library believes the state is. These vectors
// pin the agreement for the flagship inputs (genesis transfer to the funded
// TN10 wallet pubkey, amount 500_000, rule "csci-v0-...-partial-transfer").
// If the guest's state encoding ever drifts, regenerate the proof and update
// both sides together — never just one.
#[test]
fn guest_state_and_rule_hash_match_library() {
    // Funded TN10 wallet x-only pubkey (kaspatest:qzxvmfv4...).
    let owner_xonly: [u8; 32] =
        hex_to_32("8ccda595c46ca04b0867e0e80f84ca0d10a6ae40d871346c189b4fa4c571c6c8");
    let rule_bytes = b"csci-v0-compliance-rule-allow-partial-transfer";
    let rule_hash: [u8; 32] = Sha256::digest(rule_bytes).into();

    // Genesis -> first transfer (seq 0 -> 1), the flagship transition.
    let prev = CsciState::new_genesis(owner_xonly, 1_000_000, rule_hash, [0u8; 32]);
    let tx = CsciStateTransition::transfer(&prev, owner_xonly, 500_000, rule_hash).unwrap();

    // These are the values the guest committed (observed in the DEV_MODE run
    // and reproduced by the real prover). The covenant_id (journal[0..32]) is
    // supplied per-instance, so it is NOT pinned here — only the state/rule.
    assert_eq!(
        tx.new_state.seq, 1,
        "flagship transition advances seq 0 -> 1"
    );
    assert_eq!(
        hex::encode(tx.new_state.state_hash()),
        "62995096aff92337d2b62bca92bda41c5d6130c5f67208de39edec908d96bf0b",
        "guest new_state_hash must equal CsciState::state_hash() (50-byte encoding)"
    );
    assert_eq!(
        hex::encode(rule_hash),
        "59edbfe782213621b0f4407c19d7bb2718729e35cf3f21b38d928ef78e83da8d",
        "rule_hash = sha256(rule bytes)"
    );
}

fn hex_to_32(s: &str) -> [u8; 32] {
    let v = hex::decode(s).expect("valid hex");
    v.try_into().expect("32 bytes")
}

// ── Chained transfers ──────────────────────────────────────────────────────

#[test]
fn chained_transfers_increment_seq() {
    let s0 = genesis();
    let t1 = CsciStateTransition::transfer(&s0, owner(0x02), 500_000, RULE_HASH).unwrap();
    assert_eq!(t1.new_state.seq, 1);
    let t2 = CsciStateTransition::transfer(&t1.new_state, owner(0x03), 100_000, RULE_HASH).unwrap();
    assert_eq!(t2.new_state.seq, 2);
}
