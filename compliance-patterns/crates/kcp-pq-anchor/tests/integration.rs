use kcp_pq_anchor::{
    anchor_script::{build_pq_anchor_redeem, PqAnchorScriptFields},
    journal_spec::JournalSpec,
    sigop::sigop_count_for_pq_verify,
};

fn minimal_fields() -> PqAnchorScriptFields {
    PqAnchorScriptFields {
        claim: vec![0xABu8; 32],
        control_index: 0,
        control_digests: vec![0xCDu8; 64], // two 32-byte digests
        seal: vec![0xEFu8; 128],
        journal: [0x01u8; 32],
        image_id: [0x02u8; 32],
        control_id: [0x03u8; 32],
    }
}

#[test]
fn anchor_script_roundtrip() {
    let fields = minimal_fields();
    let script = build_pq_anchor_redeem(&fields).expect("build must succeed");
    // Script must be non-empty
    assert!(!script.is_empty());
    // Script must end with OpZkPrecompile (0xa6) — the in-consensus verifier
    // invocation in rusty-kaspa v2.0.0 (NOT OP_0). The five bytes before it are
    // the hashfn push (0x01 0x01) and the tag push (0x01 0x21).
    let n = script.len();
    assert_eq!(
        &script[n - 5..n],
        &[0x01, 0x01, 0x01, 0x21, 0xa6],
        "must end with <hashfn push> <tag push> OpZkPrecompile"
    );
}

#[test]
fn hashfn_pushed_as_single_byte() {
    let fields = minimal_fields();
    let script = build_pq_anchor_redeem(&fields).expect("build must succeed");
    // hashfn (Poseidon2 = 1) is pushed as a 1-byte data push: 0x01 0x01.
    // The engine's parse_hashfn requires exactly a 1-byte push (test vector
    // succinct.hashfn.hex = "01"); a numeric OP_1 (0x51) would be wrong.
    assert!(
        script.windows(2).any(|w| w == [0x01, 0x01]),
        "script must contain a 1-byte data push of hashfn id 1 (0x01 0x01)"
    );
}

#[test]
fn journal_spec_determinism() {
    let spec_a = JournalSpec::PairedAttestation {
        attestation_id: [0xAAu8; 32],
        spend_outpoint: [0xBBu8; 36],
    };
    let spec_b = JournalSpec::PairedAttestation {
        attestation_id: [0xAAu8; 32],
        spend_outpoint: [0xBBu8; 36],
    };
    let spec_c = JournalSpec::PairedAttestation {
        attestation_id: [0xCCu8; 32],
        spend_outpoint: [0xBBu8; 36],
    };
    assert_eq!(
        spec_a.journal_hash(),
        spec_b.journal_hash(),
        "same inputs → same hash"
    );
    assert_ne!(
        spec_a.journal_hash(),
        spec_c.journal_hash(),
        "different inputs → different hash"
    );
}

#[test]
fn sigop_count_is_255() {
    assert_eq!(sigop_count_for_pq_verify(), 255u8);
}

#[test]
fn control_digests_must_be_multiple_of_32() {
    let mut fields = minimal_fields();
    fields.control_digests = vec![0xFFu8; 33]; // invalid: not a multiple of 32
    assert!(build_pq_anchor_redeem(&fields).is_err());
}

// ── VProgStateTransition and CsciTransition ───────────────────────────────────

#[test]
fn vprog_state_transition_journal_bytes_length() {
    let spec = JournalSpec::VProgStateTransition {
        covenant_id: [0x01u8; 32],
        prev_state_hash: [0x02u8; 32],
        next_state_hash: [0x03u8; 32],
        vprog_image_id: [0x04u8; 32],
        seq: 7,
    };
    let bytes = spec.journal_bytes().unwrap();
    assert_eq!(
        bytes.len(),
        136,
        "covenant_id(32)+prev(32)+next(32)+image_id(32)+seq(8)=136"
    );
    // seq at offset 128
    assert_eq!(&bytes[128..136], &7u64.to_le_bytes());
}

#[test]
fn vprog_state_transition_hash_determinism() {
    let make = |seq: u64| JournalSpec::VProgStateTransition {
        covenant_id: [0xABu8; 32],
        prev_state_hash: [0x11u8; 32],
        next_state_hash: [0x22u8; 32],
        vprog_image_id: [0x33u8; 32],
        seq,
    };
    assert_eq!(make(1).journal_hash(), make(1).journal_hash());
    assert_ne!(make(1).journal_hash(), make(2).journal_hash());
    // journal_hash == sha256(journal_bytes)
    use sha2::{Digest, Sha256};
    let bytes = make(5).journal_bytes().unwrap();
    let expected: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(make(5).journal_hash(), expected);
}

#[test]
fn csci_transition_journal_bytes_length() {
    let spec = JournalSpec::CsciTransition {
        covenant_id: [0x01u8; 32],
        new_state_hash: [0x02u8; 32],
        rule_hash: [0x03u8; 32],
        seq: 1,
    };
    let bytes = spec.journal_bytes().unwrap();
    assert_eq!(
        bytes.len(),
        104,
        "covenant_id(32)+state(32)+rule(32)+seq(8)=104"
    );
    // seq at offset 96
    assert_eq!(&bytes[96..104], &1u64.to_le_bytes());
}

#[test]
fn csci_transition_hash_matches_sha256_of_bytes() {
    use sha2::{Digest, Sha256};
    let spec = JournalSpec::CsciTransition {
        covenant_id: [0xCAu8; 32],
        new_state_hash: [0xFEu8; 32],
        rule_hash: [0xBEu8; 32],
        seq: 42,
    };
    let bytes = spec.journal_bytes().unwrap();
    let expected: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(spec.journal_hash(), expected);
}

#[test]
fn custom_returns_none_for_journal_bytes() {
    let spec = JournalSpec::Custom([0xFFu8; 32]);
    assert!(spec.journal_bytes().is_none());
    assert_eq!(spec.journal_hash(), [0xFFu8; 32]);
}
