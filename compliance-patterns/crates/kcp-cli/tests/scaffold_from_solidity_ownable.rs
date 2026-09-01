/// Integration tests for the `kcp new ownable` Solidity-migration scaffold.
///
/// `generates_expected_files` — always runs; fast (no network, no cargo).
/// `generated_project_cargo_test` — shells out to `cargo test` on the generated
///   project. This is the scaffold whose generated `main.rs` called `hex::encode`
///   while its generated `Cargo.toml` omitted `hex`, so the very first
///   `cargo run` in a fresh project failed to compile; only actually building
///   the output catches that class of defect. Opt in with
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

fn generate_into(out: &std::path::Path) {
    let cfg = kcp::scaffold::from_solidity_ownable::FromSolidityOwnableConfig {
        out_dir: out.to_path_buf(),
        workspace_path: workspace_root(),
    };
    kcp::scaffold::from_solidity_ownable::generate(&cfg).expect("generate must succeed");
}

#[test]
fn generates_expected_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    generate_into(tmp.path());

    for f in [
        "Cargo.toml",
        "src/main.rs",
        "tests/ownable_smoke.rs",
        "README.md",
    ] {
        assert!(tmp.path().join(f).exists(), "missing generated file: {f}");
    }
}

#[test]
fn generated_manifest_declares_every_dependency_main_uses() {
    let tmp = tempfile::tempdir().expect("tempdir");
    generate_into(tmp.path());

    let manifest = std::fs::read_to_string(tmp.path().join("Cargo.toml")).expect("read manifest");
    let main_rs = std::fs::read_to_string(tmp.path().join("src/main.rs")).expect("read main.rs");

    for krate in ["hex", "kii_solidity_compat"] {
        if main_rs.contains(&format!("{krate}::")) {
            let dep = krate.replace('_', "-");
            assert!(
                manifest.contains(&format!("\n{dep} =")) || manifest.contains(&format!("\n{dep} ")),
                "main.rs uses `{krate}::` but Cargo.toml does not declare `{dep}`"
            );
        }
    }
}

#[test]
fn generated_project_cargo_test() {
    if !scaffold_build_gate_enabled() {
        eprintln!("SKIP generated_project_cargo_test: set KCP_GATE_SCAFFOLD_BUILD=1 to run it");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    generate_into(tmp.path());

    // Build the generated standalone project into the parent workspace's target
    // directory so it reuses the already-compiled dependencies. `cargo test`
    // rather than `cargo check`: it also builds the binary, which is where the
    // missing-`hex` defect surfaced.
    let status = std::process::Command::new("cargo")
        .env("CARGO_TARGET_DIR", workspace_root().join("target"))
        .args(["test", "--manifest-path"])
        .arg(tmp.path().join("Cargo.toml"))
        .status()
        .expect("cargo test must be runnable");

    assert!(
        status.success(),
        "generated ownable project must build and pass its own tests"
    );
}
