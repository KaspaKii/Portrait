// Integration tests for `kcp scaffold ktt-token`.
//
// These tests call the scaffold generator directly and inspect the generated
// files; they do not compile the generated projects.
// To compile and run a generated project, use:
//   cargo run -p kcp -- scaffold ktt-token --workspace-path $PWD --out /tmp/my-ktt-token

use kcp::scaffold::ktt_token::{generate, KttTokenConfig};
use std::fs;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("kcp-test-ktt-{name}"));
    if d.exists() {
        fs::remove_dir_all(&d).unwrap();
    }
    d
}

fn workspace_path() -> std::path::PathBuf {
    // This file is at crates/kcp-cli/tests/; the workspace root is 3 dirs up.
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn generates_expected_ktt_token_files() {
    let out = tmp_dir("basic");
    let cfg = KttTokenConfig {
        token_name: "TestToken".to_string(),
        initial_supply: 8_000,
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    generate(&cfg).unwrap();

    let main = fs::read_to_string(out.join("src/main.rs")).unwrap();
    let smoke = fs::read_to_string(out.join("tests/ktt_smoke.rs")).unwrap();
    let cargo = fs::read_to_string(out.join("Cargo.toml")).unwrap();
    let readme = fs::read_to_string(out.join("README.md")).unwrap();

    // Token name is embedded in comments.
    assert!(
        main.contains("TestToken"),
        "token_name must appear in main.rs"
    );
    assert!(
        readme.contains("TestToken"),
        "token_name must appear in README"
    );

    // Initial supply baked in.
    assert!(
        main.contains("8000"),
        "initial_supply must appear in main.rs"
    );

    // Core API usage.
    assert!(main.contains("kcp_ktt_token"), "must import kcp_ktt_token");
    assert!(main.contains("mint("), "must call mint()");
    assert!(main.contains("transfer("), "must call transfer()");
    assert!(main.contains("burn("), "must call burn()");
    assert!(main.contains("KttState"), "must reference KttState");
    assert!(main.contains("AuthContext"), "must reference AuthContext");
    assert!(
        main.contains("IdentifierType"),
        "must reference IdentifierType"
    );

    // Smoke test contains positive and negative tests.
    assert!(
        smoke.contains("mint_transfer_burn_round_trip"),
        "positive test must be present"
    );
    assert!(
        smoke.contains("transfer_without_auth_rejected"),
        "negative auth test must be present"
    );
    assert!(
        smoke.contains("supply_conservation_enforced"),
        "negative conservation test must be present"
    );

    // Cargo.toml references kcp-ktt-token.
    assert!(
        cargo.contains("kcp-ktt-token"),
        "Cargo.toml must reference kcp-ktt-token"
    );
    assert!(
        cargo.contains("publish = false"),
        "Cargo.toml must have publish = false"
    );

    // Clobber guard — generator created 4 files.
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("src/main.rs").exists());
    assert!(out.join("tests/ktt_smoke.rs").exists());
    assert!(out.join("README.md").exists());
}

#[test]
fn ktt_token_refuses_zero_supply() {
    let out = tmp_dir("zero-supply");
    let cfg = KttTokenConfig {
        token_name: "TestToken".to_string(),
        initial_supply: 0,
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    let err = generate(&cfg).unwrap_err();
    assert!(
        err.to_string().contains("initial_supply must be > 0"),
        "expected supply error, got: {err}"
    );
}

#[test]
fn ktt_token_refuses_empty_name() {
    let out = tmp_dir("empty-name");
    let cfg = KttTokenConfig {
        token_name: "   ".to_string(),
        initial_supply: 1_000,
        out_dir: out.clone(),
        workspace_path: workspace_path(),
    };
    let err = generate(&cfg).unwrap_err();
    assert!(
        err.to_string().contains("token_name must not be empty"),
        "expected name error, got: {err}"
    );
}

#[test]
fn ktt_token_refuses_to_overwrite() {
    let out = tmp_dir("overwrite");
    let cfg = KttTokenConfig {
        token_name: "TestToken".to_string(),
        initial_supply: 1_000,
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
