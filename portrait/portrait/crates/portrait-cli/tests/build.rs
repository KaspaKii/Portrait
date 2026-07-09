use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_workdir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    dir.push(format!("portrait-cli-build-{stamp}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Locate `silverc` (PATH, then `$HOME/.cargo/bin`). Mirrors `golden.rs`. The
/// tests that drive `portrait ship` (which shells out to `silverc`) use this to
/// skip cleanly when it is absent, rather than failing on a fresh clone.
fn find_silverc() -> Option<PathBuf> {
    if let Ok(output) = Command::new("silverc").arg("--version").output() {
        if output.status.success() || !output.stdout.is_empty() {
            return Some(PathBuf::from("silverc"));
        }
    }
    if let Ok(output) = Command::new("which").arg("silverc").output() {
        if output.status.success() {
            let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let pinned = PathBuf::from(home).join(".cargo/bin/silverc");
        if pinned.exists() {
            return Some(pinned);
        }
    }
    None
}

#[test]
fn build_writes_counter_sil_next_to_source() {
    let workdir = temp_workdir();
    let source = workdir.join("counter.portrait");
    fs::write(
        &source,
        include_str!("../../../../examples/counter.portrait"),
    )
    .expect("write source");

    let binary = env!("CARGO_BIN_EXE_portrait");
    let output = Command::new(binary)
        .current_dir(&workdir)
        .arg("build")
        .arg(&source)
        .output()
        .expect("run portrait build");

    assert!(
        output.status.success(),
        "build failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let emitted = workdir.join("Counter.sil");
    assert!(emitted.exists(), "expected emitted file at {:?}", emitted);
    let sil = fs::read_to_string(&emitted).expect("read emitted sil");
    assert!(
        sil.contains("pragma silverscript ^0.1.0;"),
        "missing silverscript pragma: {sil}"
    );
    assert!(
        sil.contains("contract Counter"),
        "missing contract declaration: {sil}"
    );
}

#[test]
fn ship_runs_pipeline_and_writes_hallmark_with_clean_summary() {
    // `portrait ship` is the single end-to-end command: check → engrave →
    // silverc → Hallmark. On a known-good source it exits 0, writes the .sil
    // beside the source AND the Hallmark manifest, and prints one summary block
    // with the maturity stamp and the (opt-in, testnet-only) deploy note.
    //
    // `ship` shells out to `silverc`; on a fresh clone without it we skip with a
    // loud message rather than fail (the differential golden tests do the same).
    if find_silverc().is_none() {
        eprintln!(
            "SKIP[ship]: silverc not found on PATH nor at $HOME/.cargo/bin/silverc \
             — `portrait ship` requires it. Skipped (NOT silently passed)."
        );
        return;
    }
    let workdir = temp_workdir();
    let source = workdir.join("counter.portrait");
    fs::write(
        &source,
        include_str!("../../../../examples/counter.portrait"),
    )
    .expect("write source");

    let binary = env!("CARGO_BIN_EXE_portrait");
    let output = Command::new(binary)
        .current_dir(&workdir)
        .arg("ship")
        .arg(&source)
        .output()
        .expect("run portrait ship");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "ship failed: stdout={stdout}\nstderr={stderr}"
    );

    // .sil emitted beside the source (engrave stage).
    assert!(
        workdir.join("Counter.sil").exists(),
        "expected emitted Counter.sil"
    );
    // Hallmark manifest written beside the source (named for the file stem).
    assert!(
        workdir.join("counter.hallmark.json").exists(),
        "expected counter.hallmark.json"
    );
    // One clean summary block with the load-bearing anchors. The Hallmark
    // component is the source file stem (`counter`), and the emitted covenant is
    // named for the app (`Counter`) in the silverc-accepts claim line.
    assert!(stdout.contains("Shipped — counter"), "summary: {stdout}");
    assert!(
        stdout.contains("silverc-accepts[Counter]"),
        "summary: {stdout}"
    );
    assert!(stdout.contains("covenants: 1"), "summary: {stdout}");
    assert!(
        stdout.contains("pre-production, unaudited, testnet-only"),
        "missing maturity stamp: {stdout}"
    );
    assert!(
        stdout.contains("opt-in") && stdout.contains("never mainnet"),
        "missing opt-in/testnet deploy note: {stdout}"
    );
    assert!(stdout.contains("verdict: ok"), "summary: {stdout}");
}

#[test]
fn check_explain_lists_invariants_and_plain_check_is_unchanged() {
    let workdir = temp_workdir();
    let source = workdir.join("counter.portrait");
    fs::write(
        &source,
        include_str!("../../../../examples/counter.portrait"),
    )
    .expect("write source");

    let binary = env!("CARGO_BIN_EXE_portrait");

    // `check --explain` prints a human invariant report and exits 0.
    let explained = Command::new(binary)
        .current_dir(&workdir)
        .arg("check")
        .arg("--explain")
        .arg(&source)
        .output()
        .expect("run portrait check --explain");
    let exp_out = String::from_utf8_lossy(&explained.stdout);
    assert!(
        explained.status.success(),
        "check --explain failed: {}",
        String::from_utf8_lossy(&explained.stderr)
    );
    assert!(exp_out.contains("invariant report"), "report: {exp_out}");
    assert!(
        exp_out.contains("(declared)") && exp_out.contains("(structural)"),
        "report: {exp_out}"
    );
    assert!(exp_out.contains("verdict: ok"), "report: {exp_out}");

    // Plain `check` is byte-identical to before: `ok: <path>` on stdout.
    let plain = Command::new(binary)
        .current_dir(&workdir)
        .arg("check")
        .arg(&source)
        .output()
        .expect("run portrait check");
    let plain_out = String::from_utf8_lossy(&plain.stdout);
    assert!(plain.status.success());
    assert_eq!(plain_out.trim(), format!("ok: {}", source.display()));
}
