/// Integration tests for the vault scaffold generator.
///
/// `generates_expected_files` — always runs; fast (no network, no cargo).
/// `generated_project_cargo_check` — shells out to `cargo check` on the
///   generated project, which needs a populated cargo cache (and network on a
///   cold one, to fetch the rusty-kaspa git dep). Opt in with
///   `KCP_GATE_SCAFFOLD_BUILD=1`, which `_harness/ci.sh` sets; without it the
///   test SKIPS so an offline clone degrades to a skip rather than a hard fail.
use std::path::PathBuf;

/// Whether to run the generated-project compile checks. See the module docs.
fn scaffold_build_gate_enabled() -> bool {
    std::env::var_os("KCP_GATE_SCAFFOLD_BUILD").is_some()
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/kcp-cli; go up two levels → workspace root
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn generates_expected_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::vault::VaultConfig {
        threshold: 2,
        keys: vec!["KEY1".into(), "KEY2".into()],
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::vault::generate(&cfg).expect("generate must succeed");
    assert!(out.join("Cargo.toml").exists(), "Cargo.toml must exist");
    assert!(out.join("src/main.rs").exists(), "src/main.rs must exist");
    assert!(
        out.join("tests/vault_smoke.rs").exists(),
        "tests/vault_smoke.rs must exist"
    );
    assert!(out.join("README.md").exists(), "README.md must exist");

    // Sanity-check Cargo.toml content
    let cargo_content = std::fs::read_to_string(out.join("Cargo.toml")).unwrap();
    assert!(
        cargo_content.contains("kcp-vault"),
        "Cargo.toml must reference kcp-vault"
    );
    assert!(
        cargo_content.contains(r#"features = ["wrpc"]"#),
        "Cargo.toml must enable wrpc feature"
    );
    assert!(
        cargo_content.contains("[workspace]"),
        "Cargo.toml must declare standalone workspace"
    );

    // Sanity-check main.rs content
    let main_content = std::fs::read_to_string(out.join("src/main.rs")).unwrap();
    assert!(
        main_content.contains("SpendCondition::MultiSig"),
        "must use MultiSig condition"
    );
    assert!(
        main_content.contains("verify_p2sh_spend_offline"),
        "must call engine preflight"
    );
    assert!(
        main_content.contains("threshold: 2"),
        "must embed the requested threshold"
    );
    // Confirm k-of-n correctness: signs with threshold sigs, not all-n.
    assert!(
        main_content.contains("first 2 key(s)"),
        "generated main.rs must document that only threshold sigs are needed"
    );
}

#[test]
fn refuses_to_overwrite_existing_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::vault::VaultConfig {
        threshold: 2,
        keys: vec!["KEY1".into(), "KEY2".into()],
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    // First generation must succeed.
    kcp::scaffold::vault::generate(&cfg).expect("first generate must succeed");
    // Second generation must fail (Cargo.toml already exists).
    let err = kcp::scaffold::vault::generate(&cfg)
        .expect_err("second generate must fail with clobber error");
    assert!(
        err.to_string().contains("already exists"),
        "error must mention 'already exists'"
    );
}

#[test]
fn generated_project_cargo_check() {
    if !scaffold_build_gate_enabled() {
        eprintln!("SKIP generated_project_cargo_check: set KCP_GATE_SCAFFOLD_BUILD=1 to run it");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::vault::VaultConfig {
        threshold: 2,
        keys: vec!["KEY1".into(), "KEY2".into()],
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::vault::generate(&cfg).expect("generate must succeed");

    // Build the generated standalone workspace into the parent workspace's
    // target directory: it reuses the already-compiled rusty-kaspa artifacts,
    // which is what keeps this check seconds instead of a ~10-minute cold
    // build of the whole engine tree.
    let status = std::process::Command::new("cargo")
        .env("CARGO_TARGET_DIR", workspace_root().join("target"))
        .args(["check", "--manifest-path"])
        .arg(out.join("Cargo.toml"))
        .status()
        .expect("cargo check must be runnable");

    assert!(
        status.success(),
        "generated vault project must pass `cargo check`"
    );
}
