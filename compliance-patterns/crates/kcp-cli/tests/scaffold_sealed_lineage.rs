// Integration tests for `kcp scaffold sealed-lineage`.
//
// These tests call the scaffold generator directly and inspect the generated
// files; they do not compile the generated projects.
// To compile and run a generated project, use:
//   cargo run -p kcp -- scaffold sealed-lineage --workspace-path $PWD --out /tmp/my-sl

use kcp::scaffold::sealed_lineage::{generate, SealedLineageConfig};
use std::fs;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("kcp-test-sl-{name}"));
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
fn generates_expected_sealed_lineage_files() {
    let out = tmp_dir("basic");
    let cfg = SealedLineageConfig {
        subject: "TestLineage".to_string(),
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    generate(&cfg).unwrap();

    let main = fs::read_to_string(out.join("src/main.rs")).unwrap();
    let smoke = fs::read_to_string(out.join("tests/lineage_smoke.rs")).unwrap();
    let cargo = fs::read_to_string(out.join("Cargo.toml")).unwrap();
    let readme = fs::read_to_string(out.join("README.md")).unwrap();

    // Subject is embedded.
    assert!(
        main.contains("TestLineage"),
        "subject must appear in main.rs"
    );
    assert!(
        readme.contains("TestLineage"),
        "subject must appear in README"
    );

    // Core API usage.
    assert!(
        main.contains("kcp_sealed_lineage"),
        "must import kcp_sealed_lineage"
    );
    assert!(main.contains("lineage_id("), "must call lineage_id()");
    assert!(main.contains("commitment("), "must call commitment()");
    assert!(
        main.contains("validate_chain("),
        "must call validate_chain()"
    );
    assert!(main.contains("GENESIS"), "must reference GENESIS constant");
    assert!(main.contains("APPEND"), "must reference APPEND constant");

    // Payload struct used.
    assert!(main.contains("Payload {"), "must construct Payload structs");

    // Smoke test contains positive and negative tests.
    assert!(
        smoke.contains("valid_chain_passes"),
        "positive test must be present"
    );
    assert!(
        smoke.contains("chain_with_wrong_seq_rejected"),
        "L-1 negative test must be present"
    );
    assert!(
        smoke.contains("chain_with_mismatched_lineage_id_rejected"),
        "L-2 negative test must be present"
    );
    assert!(
        smoke.contains("payload_encode_decode_round_trip"),
        "encode/decode test must be present"
    );
    assert!(
        smoke.contains("closed_chain_rejects_append"),
        "L-3 negative test must be present"
    );

    // CLOSE constant used in tests.
    assert!(
        smoke.contains("CLOSE"),
        "must reference CLOSE constant in smoke test"
    );

    // Cargo.toml.
    assert!(
        cargo.contains("kcp-sealed-lineage"),
        "Cargo.toml must reference kcp-sealed-lineage"
    );
    assert!(cargo.contains("hex"), "Cargo.toml must include hex dep");
    assert!(
        cargo.contains("publish = false"),
        "Cargo.toml must have publish = false"
    );

    // 4 files created.
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("src/main.rs").exists());
    assert!(out.join("tests/lineage_smoke.rs").exists());
    assert!(out.join("README.md").exists());
}

#[test]
fn sealed_lineage_refuses_empty_subject() {
    let out = tmp_dir("empty-subject");
    let cfg = SealedLineageConfig {
        subject: "  ".to_string(),
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    let err = generate(&cfg).unwrap_err();
    assert!(
        err.to_string().contains("subject must not be empty"),
        "expected subject error, got: {err}"
    );
}

#[test]
fn sealed_lineage_refuses_to_overwrite() {
    let out = tmp_dir("overwrite");
    let cfg = SealedLineageConfig {
        subject: "TestLineage".to_string(),
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
