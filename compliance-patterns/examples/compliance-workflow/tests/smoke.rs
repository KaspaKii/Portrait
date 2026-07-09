use kcp_ktt_token::{
    state::{IdentifierType, KttState},
    token::{mint, AuthContext},
};
use kcp_paired_attestation::{
    mate::{build_mate_proof, verify_mate},
    record::{attestation_id, commit, AttestationRecord},
};
use kcp_sealed_lineage::{
    invariants::{validate_chain as sl_validate, APPEND, GENESIS},
    payload::Payload,
    record::{commitment as sl_commitment, lineage_id},
};
use kcp_transferable_record::{
    lineage::{validate_chain as tr_validate, TransferEvent},
    record::{commitment as tr_commitment, record_id},
};

fn synthetic_workflow() {
    let subject = [0x01u8; 32];
    let terms_hash = [0xABu8; 32];
    let record = AttestationRecord::new(subject, terms_hash, 42);
    let att_id = attestation_id(&record).expect("attestation_id must succeed");
    assert_ne!(att_id, [0u8; 32], "attestation_id must be non-zero");

    let blind_a = [0x11u8; 32];
    let blind_b = [0x22u8; 32];
    let commit_a = commit(&record, &blind_a).unwrap();
    let commit_b = commit(&record, &blind_b).unwrap();
    let proof = build_mate_proof(&record, blind_a, blind_b, commit_a, commit_b)
        .expect("build_mate_proof must succeed");
    verify_mate(&proof).expect("mate proof verifies");

    let lid = lineage_id(&serde_json::json!({"subject": hex::encode(subject)}))
        .expect("lineage_id must succeed");
    assert_ne!(lid, [0u8; 32], "lineage_id must be non-zero");

    let c0 = sl_commitment(
        &serde_json::json!({"subject": hex::encode(subject)}),
        &blind_a,
    )
    .unwrap();
    let c1 = sl_commitment(&serde_json::json!({"att": hex::encode(att_id)}), &blind_b).unwrap();
    sl_validate(&[
        Payload {
            lineage_id: lid,
            seq: 0,
            event_class: GENESIS,
            t_bucket: 0,
            commitment: c0,
        }, // t_bucket: 0 = epoch-start; APPEND must be ≥ GENESIS value (L-4 temporal)
        Payload {
            lineage_id: lid,
            seq: 1,
            event_class: APPEND,
            t_bucket: 1,
            commitment: c1,
        },
    ])
    .expect("lineage valid");

    let genesis_ctrl = [0xAAu8; 32];
    let rec_id = record_id(&serde_json::json!({"subject": hex::encode(subject)})).unwrap();
    let rec_c = tr_commitment(&serde_json::json!({"att": hex::encode(att_id)})).unwrap();
    tr_validate(
        &genesis_ctrl,
        &[TransferEvent {
            seq: 1,
            record_id: rec_id,
            controller_xonly: [0xBBu8; 32],
            commitment: rec_c,
        }],
    )
    .expect("transfer chain valid");

    let minter_owner = [0xAAu8; 32];
    let holder_owner = [0xBBu8; 32];
    mint(
        &KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: minter_owner,
            amount: 0,
            is_minter: true,
        },
        &KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: holder_owner,
            amount: 500_000,
            is_minter: false,
        },
        &KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: minter_owner,
            amount: 0,
            is_minter: true,
        },
        &AuthContext {
            authorised_owners: vec![minter_owner],
        },
        0,
    )
    .expect("mint valid");
}

#[test]
fn compliance_workflow_runs_without_panic() {
    synthetic_workflow();
}

#[test]
fn attestation_id_is_deterministic() {
    let r = AttestationRecord::new([0x01u8; 32], [0x02u8; 32], 99);
    assert_eq!(attestation_id(&r).unwrap(), attestation_id(&r).unwrap(),);
}

#[test]
fn lineage_id_is_non_zero() {
    let lid = lineage_id(&serde_json::json!({"subject": "test"})).unwrap();
    assert_ne!(lid, [0u8; 32]);
}

#[test]
fn lineage_validate_rejects_bad_seq() {
    use kcp_sealed_lineage::{
        invariants::{validate_chain as sl_validate, APPEND, GENESIS},
        payload::Payload,
        record::lineage_id,
    };
    let lid = lineage_id(&serde_json::json!({"subject": "test"})).unwrap();
    let blind = [0x11u8; 32];
    let c0 =
        kcp_sealed_lineage::record::commitment(&serde_json::json!({"subject": "test"}), &blind)
            .unwrap();
    let c1 =
        kcp_sealed_lineage::record::commitment(&serde_json::json!({"event": "a"}), &blind).unwrap();
    // seq jumps from 0 to 2 — must violate L-1 monotone invariant
    let result = sl_validate(&[
        Payload {
            lineage_id: lid,
            seq: 0,
            event_class: GENESIS,
            t_bucket: 0,
            commitment: c0,
        },
        Payload {
            lineage_id: lid,
            seq: 2,
            event_class: APPEND,
            t_bucket: 0,
            commitment: c1,
        },
    ]);
    assert!(result.is_err(), "L-1 violation must be rejected");
}
