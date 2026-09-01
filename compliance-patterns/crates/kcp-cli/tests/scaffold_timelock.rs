/// Integration tests for the timelock scaffold generator.
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
fn generates_expected_timelock_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::timelock::TimelockConfig {
        deadline: 1_000_000,
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::timelock::generate(&cfg).expect("generate must succeed");

    assert!(out.join("Cargo.toml").exists(), "Cargo.toml must exist");
    assert!(out.join("src/main.rs").exists(), "src/main.rs must exist");
    assert!(
        out.join("tests/timelock_smoke.rs").exists(),
        "tests/timelock_smoke.rs must exist"
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
    assert!(
        main.contains("verify_p2sh_spend_offline"),
        "must call engine preflight"
    );
    assert!(
        main.contains("1000000"),
        "must embed the requested deadline"
    );
    // CLTV fields must be correct
    assert!(
        main.contains("lock_time"),
        "must set lock_time on the transaction"
    );
    // Must use compile_condition_p2sh (not compile_condition) — Kaspa CLTV pops
    assert!(
        main.contains("compile_condition_p2sh"),
        "must use compile_condition_p2sh (Kaspa CLTV pops the deadline)"
    );
    assert!(
        !main.contains("compile_condition(&"),
        "must NOT use compile_condition — incorrect for Kaspa P2SH CLTV"
    );

    // smoke test sanity — negative test must be present
    let smoke = std::fs::read_to_string(out.join("tests/timelock_smoke.rs")).unwrap();
    assert!(
        smoke.contains("timelock_rejected_before_deadline"),
        "must include negative CLTV test"
    );
}

#[test]
fn timelock_refuses_zero_deadline() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg = kcp::scaffold::timelock::TimelockConfig {
        deadline: 0,
        out_dir: tmp.path().to_path_buf(),
        workspace_path: workspace_root(),
    };
    let err = kcp::scaffold::timelock::generate(&cfg).expect_err("zero deadline must fail");
    assert!(
        err.to_string().contains("deadline must be > 0"),
        "error must mention deadline constraint"
    );
}

#[test]
fn timelock_refuses_to_overwrite() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::timelock::TimelockConfig {
        deadline: 1_000_000,
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::timelock::generate(&cfg).expect("first generate must succeed");
    let err = kcp::scaffold::timelock::generate(&cfg).expect_err("second generate must fail");
    assert!(
        err.to_string().contains("already exists"),
        "error must mention 'already exists'"
    );
}

#[test]
#[ignore = "requires network (fetches rusty-kaspa git dep); run with --ignored to opt in"]
fn generated_timelock_project_cargo_check() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().to_path_buf();
    let cfg = kcp::scaffold::timelock::TimelockConfig {
        deadline: 1_000_000,
        out_dir: out.clone(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::timelock::generate(&cfg).expect("generate must succeed");

    let status = std::process::Command::new("cargo")
        .args(["check", "--manifest-path"])
        .arg(out.join("Cargo.toml"))
        .status()
        .expect("cargo check must be runnable");

    assert!(
        status.success(),
        "generated timelock project must pass `cargo check`"
    );
}
