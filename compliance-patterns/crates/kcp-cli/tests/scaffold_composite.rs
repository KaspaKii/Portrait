/// Integration tests for the composite scaffold generator.
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn generates_expected_composite_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::composite::CompositeConfig {
        deadline: 1_000_000,
        threshold: 2,
        n: 3,
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::composite::generate(&cfg).expect("generate must succeed");

    assert!(out.join("Cargo.toml").exists(), "Cargo.toml must exist");
    assert!(out.join("src/main.rs").exists(), "src/main.rs must exist");
    assert!(
        out.join("tests/composite_smoke.rs").exists(),
        "tests/composite_smoke.rs must exist"
    );
    assert!(out.join("README.md").exists(), "README.md must exist");

    // Cargo.toml sanity
    let cargo = std::fs::read_to_string(out.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("kcp-vault"), "must reference kcp-vault");
    assert!(
        cargo.contains(r#"features = ["wrpc"]"#),
        "must enable wrpc feature"
    );
    assert!(
        cargo.contains("[workspace]"),
        "must be standalone workspace"
    );

    // main.rs sanity
    let main = std::fs::read_to_string(out.join("src/main.rs")).unwrap();
    assert!(
        main.contains("TimelockHeight"),
        "must use TimelockHeight condition"
    );
    assert!(main.contains("MultiSig"), "must use MultiSig condition");
    assert!(
        main.contains("SpendCondition::All"),
        "must use All composite"
    );
    assert!(
        main.contains("verify_p2sh_spend_offline"),
        "must call engine preflight"
    );
    assert!(
        main.contains("1000000"),
        "must embed the requested deadline"
    );
    assert!(
        main.contains("compile_condition_p2sh"),
        "must use compile_condition_p2sh (Kaspa CLTV pops the deadline)"
    );
    assert!(
        !main.contains("compile_condition(&"),
        "must NOT use compile_condition — incorrect for Kaspa P2SH CLTV"
    );
    // Satisfier order comment must be present
    assert!(
        main.contains("ctrl_sig must be on top"),
        "must document satisfier ordering"
    );

    // smoke test sanity — negative test must be present
    let smoke = std::fs::read_to_string(out.join("tests/composite_smoke.rs")).unwrap();
    assert!(
        smoke.contains("composite_rejected_before_deadline"),
        "must include negative CLTV test"
    );
    assert!(
        smoke.contains("compile_condition_p2sh"),
        "smoke test must use compile_condition_p2sh"
    );
}

#[test]
fn composite_refuses_zero_deadline() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg = kcp::scaffold::composite::CompositeConfig {
        deadline: 0,
        threshold: 2,
        n: 3,
        out_dir: tmp.path().to_path_buf(),
        workspace_path: workspace_root(),
    };
    let err = kcp::scaffold::composite::generate(&cfg).expect_err("zero deadline must fail");
    assert!(
        err.to_string().contains("deadline must be > 0"),
        "error must mention deadline constraint"
    );
}

#[test]
fn composite_refuses_bad_threshold() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg = kcp::scaffold::composite::CompositeConfig {
        deadline: 1_000_000,
        threshold: 5,
        n: 3,
        out_dir: tmp.path().to_path_buf(),
        workspace_path: workspace_root(),
    };
    let err = kcp::scaffold::composite::generate(&cfg).expect_err("t > n must fail");
    assert!(
        err.to_string().contains("threshold"),
        "error must mention threshold constraint"
    );
}

#[test]
fn composite_refuses_to_overwrite() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::composite::CompositeConfig {
        deadline: 1_000_000,
        threshold: 2,
        n: 3,
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::composite::generate(&cfg).expect("first generate must succeed");
    let err = kcp::scaffold::composite::generate(&cfg).expect_err("second generate must fail");
    assert!(
        err.to_string().contains("already exists"),
        "error must mention 'already exists'"
    );
}

#[test]
#[ignore = "requires network (fetches rusty-kaspa git dep); run with --ignored to opt in"]
fn generated_composite_project_cargo_check() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::composite::CompositeConfig {
        deadline: 1_000_000,
        threshold: 2,
        n: 3,
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::composite::generate(&cfg).expect("generate must succeed");

    let status = std::process::Command::new("cargo")
        .args(["check", "--manifest-path"])
        .arg(out.join("Cargo.toml"))
        .status()
        .expect("cargo check must be runnable");

    assert!(
        status.success(),
        "generated composite project must pass `cargo check`"
    );
}
