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

fn main() {
    println!("kaspa-compliance-patterns — Compliance Credential Lifecycle");
    println!("=============================================================");
    println!("Pre-production, unaudited, testnet-only.\n");

    // ── [1] Bilateral attestation ────────────────────────────────────────────
    println!("[1] KYC credential — parties A and B attest bilaterally");

    let subject: [u8; 32] = [0x01u8; 32];
    let terms_hash: [u8; 32] = [0xABu8; 32];
    let nonce: u64 = 1_700_000_001;

    let record = AttestationRecord::new(subject, terms_hash, nonce);
    let att_id = attestation_id(&record).expect("attestation_id");

    let blind_a: [u8; 32] = [0xAAu8; 32];
    let blind_b: [u8; 32] = [0xBBu8; 32];
    let commit_a = commit(&record, &blind_a).expect("commit_a");
    let commit_b = commit(&record, &blind_b).expect("commit_b");
    // In production: call negotiate_blind(blind_a, blind_b) to derive a shared blinding
    // factor when both parties must commit to an identical combined blind.

    // ── [2] Mate proof ───────────────────────────────────────────────────────
    println!("[2] Building and verifying mate proof");

    let proof =
        build_mate_proof(&record, blind_a, blind_b, commit_a, commit_b).expect("build_mate_proof");
    verify_mate(&proof).expect("mate proof must verify");
    println!("    \u{2713} Mate proof verified \u{2014} bilateral commitment confirmed");

    // ── [3] Seal into evidence lineage ───────────────────────────────────────
    println!("[3] Sealing attestation into an evidence lineage");

    let lid =
        lineage_id(&serde_json::json!({ "subject": hex::encode(subject) })).expect("lineage_id");
    let commit_genesis = sl_commitment(
        &serde_json::json!({ "subject": hex::encode(subject) }),
        &blind_a,
    )
    .expect("commit_genesis");
    let commit_append = sl_commitment(
        &serde_json::json!({ "attestation_id": hex::encode(att_id) }),
        &blind_b,
    )
    .expect("commit_append");

    sl_validate(&[
        Payload {
            lineage_id: lid,
            seq: 0,
            event_class: GENESIS,
            t_bucket: 0,
            commitment: commit_genesis,
        }, // t_bucket: 0 = epoch-start; APPEND must be ≥ GENESIS value (L-4 temporal)
        Payload {
            lineage_id: lid,
            seq: 1,
            event_class: APPEND,
            t_bucket: 1,
            commitment: commit_append,
        },
    ])
    .expect("sealed lineage must satisfy L-1..L-4");
    println!("    \u{2713} GENESIS + APPEND \u{2014} lineage validated (L-1 monotone, L-2 identity, L-3 class, L-4 temporal)");

    // ── [4] Transfer credential record ──────────────────────────────────────
    println!("[4] Transferring credential record to subject controller");

    let genesis_controller: [u8; 32] = [0xAAu8; 32];
    let new_controller: [u8; 32] = [0xCCu8; 32];

    let rec_id =
        record_id(&serde_json::json!({ "subject": hex::encode(subject) })).expect("record_id");
    let rec_commitment =
        tr_commitment(&serde_json::json!({ "attestation_id": hex::encode(att_id) }))
            .expect("tr_commitment");

    tr_validate(
        &genesis_controller,
        &[TransferEvent {
            seq: 1,
            record_id: rec_id,
            controller_xonly: new_controller,
            commitment: rec_commitment,
        }],
    )
    .expect("transfer chain must satisfy TR-1..TR-3");
    println!("    \u{2713} Controller transferred \u{2014} chain validated (TR-1 seq, TR-2 identity, TR-3 commitment)");

    // ── [5] Mint regulated compliance token ──────────────────────────────────
    println!("[5] Minting regulated compliance token");

    let minter_owner: [u8; 32] = [0xAAu8; 32];
    let holder_owner: [u8; 32] = [0xCCu8; 32];

    let minter_input = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: minter_owner,
        amount: 0,
        is_minter: true,
    };
    let minted_output = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: holder_owner,
        amount: 1_000_000,
        is_minter: false,
    };
    let persisted_minter = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: minter_owner,
        amount: 0,
        is_minter: true,
    };
    let auth = AuthContext {
        authorised_owners: vec![minter_owner],
    };

    mint(&minter_input, &minted_output, &persisted_minter, &auth, 0)
        .expect("mint must satisfy KTT-1..KTT-4");
    println!("    \u{2713} 1,000,000 tokens minted \u{2014} KTT-1 supply conservation + KTT-3 minter auth verified");

    // ── [6] Summary ──────────────────────────────────────────────────────────
    println!("\n\u{2500}\u{2500} Summary \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!("attestation_id  : {}", hex::encode(att_id));
    println!("lineage_id      : {}", hex::encode(lid));
    println!("record_id       : {}", hex::encode(rec_id));
    println!(
        "token minted    : {} units to {}",
        minted_output.amount,
        hex::encode(holder_owner)
    );
    println!();
    println!("All invariants verified offline against the real kcp pattern APIs.");
    println!("Before live use: replace synthetic blinds with CSPRNG bytes;");
    println!("replace synthetic controller keys with real Schnorr x-only pubkeys.");
}
