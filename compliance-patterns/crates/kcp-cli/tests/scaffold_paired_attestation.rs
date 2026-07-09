// Integration tests for `kcp scaffold paired-attestation`.

use kcp::scaffold::paired_attestation::{generate, PairedAttestationConfig};
use std::fs;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("kcp-test-pa-{name}"));
    if d.exists() {
        fs::remove_dir_all(&d).unwrap();
    }
    d
}

fn workspace_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn generates_expected_paired_attestation_files() {
    let out = tmp_dir("basic");
    let cfg = PairedAttestationConfig {
        subject_label: "ServiceAgreement".to_string(),
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    generate(&cfg).unwrap();

    let main = fs::read_to_string(out.join("src/main.rs")).unwrap();
    let smoke = fs::read_to_string(out.join("tests/attestation_smoke.rs")).unwrap();
    let cargo = fs::read_to_string(out.join("Cargo.toml")).unwrap();
    let readme = fs::read_to_string(out.join("README.md")).unwrap();

    // Subject label embedded.
    assert!(
        main.contains("ServiceAgreement"),
        "subject_label must appear in main.rs"
    );
    assert!(
        readme.contains("ServiceAgreement"),
        "subject_label must appear in README"
    );

    // Core API usage.
    assert!(
        main.contains("kcp_paired_attestation"),
        "must import kcp_paired_attestation"
    );
    assert!(
        main.contains("AttestationRecord::new"),
        "must call AttestationRecord::new"
    );
    assert!(
        main.contains("attestation_id("),
        "must call attestation_id()"
    );
    assert!(main.contains("commit("), "must call commit()");
    assert!(
        main.contains("negotiate_blind("),
        "must call negotiate_blind()"
    );
    assert!(
        main.contains("build_mate_proof("),
        "must call build_mate_proof()"
    );
    assert!(main.contains("verify_mate("), "must call verify_mate()");

    // Smoke tests.
    assert!(
        smoke.contains("valid_proof_verifies"),
        "positive test must be present"
    );
    assert!(
        smoke.contains("negotiate_blind_is_symmetric"),
        "symmetry test must be present"
    );
    assert!(
        smoke.contains("proof_with_wrong_commit_a_fails"),
        "tamper-a test must be present"
    );
    assert!(
        smoke.contains("proof_with_wrong_commit_b_fails"),
        "tamper-b test must be present"
    );
    assert!(
        smoke.contains("attestation_id_deterministic"),
        "determinism test must be present"
    );
    assert!(
        smoke.contains("different_nonce_different_id"),
        "nonce-isolation test must be present"
    );

    // Cargo.toml
    assert!(
        cargo.contains("kcp-paired-attestation"),
        "Cargo.toml must reference kcp-paired-attestation"
    );
    assert!(cargo.contains("hex"), "Cargo.toml must include hex dep");
    assert!(
        cargo.contains("publish = false"),
        "Cargo.toml must have publish = false"
    );

    // 4 files created.
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("src/main.rs").exists());
    assert!(out.join("tests/attestation_smoke.rs").exists());
    assert!(out.join("README.md").exists());
}

#[test]
fn paired_attestation_refuses_empty_subject() {
    let out = tmp_dir("empty-subject");
    let cfg = PairedAttestationConfig {
        subject_label: "  ".to_string(),
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    let err = generate(&cfg).unwrap_err();
    assert!(
        err.to_string().contains("subject_label must not be empty"),
        "expected subject error, got: {err}"
    );
}

#[test]
fn paired_attestation_refuses_to_overwrite() {
    let out = tmp_dir("overwrite");
    let cfg = PairedAttestationConfig {
        subject_label: "ServiceAgreement".to_string(),
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    generate(&cfg).unwrap();
    let err = generate(&cfg).unwrap_err();
    assert!(
        err.to_string().contains("already exists"),
        "expected overwrite error, got: {err}"
    );
}
