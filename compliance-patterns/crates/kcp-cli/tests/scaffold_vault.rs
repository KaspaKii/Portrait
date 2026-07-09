/// Integration tests for the vault scaffold generator.
///
/// `generates_expected_files` — always runs; fast (no network, no cargo).
/// `generated_project_cargo_check` — marked `#[ignore]`; requires network
///   (fetches rusty-kaspa git dep on first run). Opt in with:
///   `cargo test -p kcp --test scaffold_vault -- --ignored`
use std::path::PathBuf;

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
#[ignore = "requires network (fetches rusty-kaspa git dep); run with --ignored to opt in"]
fn generated_project_cargo_check() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::vault::VaultConfig {
        threshold: 2,
        keys: vec!["KEY1".into(), "KEY2".into()],
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::vault::generate(&cfg).expect("generate must succeed");

    let status = std::process::Command::new("cargo")
        .args(["check", "--manifest-path"])
        .arg(out.join("Cargo.toml"))
        .status()
        .expect("cargo check must be runnable");

    assert!(
        status.success(),
        "generated vault project must pass `cargo check`"
    );
}
