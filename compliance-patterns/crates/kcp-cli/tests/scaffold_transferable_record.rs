// Integration tests for `kcp scaffold transferable-record`.

use kcp::scaffold::transferable_record::{generate, TransferableRecordConfig};
use std::fs;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("kcp-test-tr-{name}"));
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
fn generates_expected_transferable_record_files() {
    let out = tmp_dir("basic");
    let cfg = TransferableRecordConfig {
        record_type: "LandTitle".to_string(),
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    generate(&cfg).unwrap();

    let main = fs::read_to_string(out.join("src/main.rs")).unwrap();
    let smoke = fs::read_to_string(out.join("tests/record_smoke.rs")).unwrap();
    let cargo = fs::read_to_string(out.join("Cargo.toml")).unwrap();
    let readme = fs::read_to_string(out.join("README.md")).unwrap();

    // Record type embedded.
    assert!(
        main.contains("LandTitle"),
        "record_type must appear in main.rs"
    );
    assert!(
        readme.contains("LandTitle"),
        "record_type must appear in README"
    );

    // Core API usage.
    assert!(
        main.contains("kcp_transferable_record"),
        "must import kcp_transferable_record"
    );
    assert!(main.contains("record_id("), "must call record_id()");
    assert!(main.contains("commitment("), "must call commitment()");
    assert!(
        main.contains("validate_chain("),
        "must call validate_chain()"
    );
    assert!(
        main.contains("TransferEvent {"),
        "must construct TransferEvent structs"
    );

    // Smoke tests cover all invariants.
    assert!(
        smoke.contains("valid_chain_passes"),
        "positive test must be present"
    );
    assert!(
        smoke.contains("empty_chain_passes"),
        "empty chain test must be present"
    );
    assert!(
        smoke.contains("chain_with_seq_gap_rejected"),
        "TR-1 negative test must be present"
    );
    assert!(
        smoke.contains("chain_with_mismatched_record_id_rejected"),
        "TR-2 negative test must be present"
    );
    assert!(
        smoke.contains("chain_with_zero_commitment_rejected"),
        "TR-3 negative test must be present"
    );
    assert!(
        smoke.contains("payload_encode_decode_round_trip"),
        "encode/decode test must be present"
    );

    // Cargo.toml
    assert!(
        cargo.contains("kcp-transferable-record"),
        "Cargo.toml must reference kcp-transferable-record"
    );
    assert!(cargo.contains("hex"), "Cargo.toml must include hex dep");
    assert!(
        cargo.contains("publish = false"),
        "Cargo.toml must have publish = false"
    );

    // 4 files created.
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("src/main.rs").exists());
    assert!(out.join("tests/record_smoke.rs").exists());
    assert!(out.join("README.md").exists());
}

#[test]
fn transferable_record_refuses_empty_type() {
    let out = tmp_dir("empty-type");
    let cfg = TransferableRecordConfig {
        record_type: "  ".to_string(),
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    let err = generate(&cfg).unwrap_err();
    assert!(
        err.to_string().contains("record_type must not be empty"),
        "expected type error, got: {err}"
    );
}

#[test]
fn transferable_record_refuses_to_overwrite() {
    let out = tmp_dir("overwrite");
    let cfg = TransferableRecordConfig {
        record_type: "LandTitle".to_string(),
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
