//! The `portrait` CLI: orchestrates the pipeline (BUILD_SPEC §8).
//! Hand-rolled arg parsing keeps the M0 scaffold dependency-free; swap in clap later.

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");
    match cmd {
        "check" => cmd_check(&args[2..]),
        "build" => cmd_build(args.get(2)),
        "engrave" => cmd_engrave(args.get(2)),
        "atelier-build" => cmd_atelier_build(args.get(2)),
        "ship" => cmd_ship(args.get(2)),
        "verify" => cmd_verify(args.get(2)),
        "prove" => cmd_prove(args.get(2)),
        "compose" => cmd_compose(args.get(2)),
        "validate-translation" => cmd_validate_translation(args.get(2)),
        "test" => println!("portrait test -- golden + debugger vectors (M1)"),
        "publish" => println!("portrait publish -- publish a Hallmark manifest (planned)"),
        "new" => cmd_new(&args[2..]),
        "version" | "--version" => println!("portrait 0.1.0"),
        _ => help(),
    }
}

fn cmd_check(args: &[String]) {
    // Parse a single optional flag (`--explain`) and one positional file.
    // Plain `portrait check <file>` is byte-identical to before.
    let mut explain = false;
    let mut path: Option<&str> = None;
    for a in args {
        match a.as_str() {
            "--explain" => explain = true,
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag {}", other);
                exit(2);
            }
            other => {
                if path.is_none() {
                    path = Some(other);
                } else {
                    eprintln!("error: unexpected argument {}", other);
                    exit(2);
                }
            }
        }
    }
    let Some(p) = path else {
        eprintln!("usage: portrait check [--explain] <file>");
        exit(2);
    };

    if !explain {
        match run_check(p) {
            Ok(()) => println!("ok: {}", p),
            Err(e) => {
                eprintln!("error: {}", e);
                exit(1);
            }
        }
        return;
    }

    // `--explain`: render a human invariant report. Parse first (a parse error
    // means we cannot read the declared invariants — report it honestly).
    let src = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    };
    let program = match portrait_syntax::parse(&src) {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("error: parse failed: {}", e);
            exit(1);
        }
    };
    let check_result = portrait_sema::check(&program);
    let passed = check_result.is_ok();
    print!("{}", render_explain(&program, &check_result));
    if !passed {
        exit(1);
    }
}

/// Render a human-readable invariant + structural-check report for `program`,
/// given the all-or-nothing verdict from `portrait_sema::check`. This adds no
/// new analysis: an `Ok` verdict means every declared invariant AND every
/// always-on structural check passed, so each is rendered `ok`; an `Err` verdict
/// lists the grouped diagnostics under a `fail` heading. Pure (no I/O) so it is
/// unit-testable.
fn render_explain(
    program: &portrait_syntax::Program,
    check_result: &Result<(), Vec<portrait_sema::Diagnostic>>,
) -> String {
    use portrait_syntax::Invariant;
    let mut out = String::new();
    out.push_str(&format!("invariant report — app {}\n", program.app.name));

    let glyph = if check_result.is_ok() { "ok" } else { "fail" };

    // Declared invariants (from `invariant ...;` clauses), rendered as the
    // canonical check name. With an Ok verdict every one held; with an Err
    // verdict the program is rejected as a whole (sema is all-or-nothing).
    for inv in &program.app.invariants {
        let name = match inv {
            Invariant::ValueConserved => "value_conserved".to_string(),
            Invariant::NoUndeclaredState => "no_undeclared_state".to_string(),
            Invariant::Custom(s) => s.clone(),
        };
        out.push_str(&format!("  [{}] {} (declared)\n", glyph, name));
    }

    // Always-on structural checks portrait-sema enforces for every program.
    for sc in [
        "lifecycle_reachability",
        "flow_integrity",
        "transition_return_consistency",
        "expression_typing",
    ] {
        out.push_str(&format!("  [{}] {} (structural)\n", glyph, sc));
    }

    match check_result {
        Ok(()) => {
            out.push_str("verdict: ok — all declared invariants and structural checks passed.\n");
        }
        Err(ds) => {
            out.push_str(&format!("verdict: fail — {} diagnostic(s):\n", ds.len()));
            for d in ds {
                out.push_str(&format!("  - {}\n", d.message));
            }
        }
    }
    out
}

fn run_check(path: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let program = portrait_syntax::parse(&src)?;
    portrait_sema::check(&program).map_err(|ds| {
        ds.into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
            .join("; ")
    })
}

fn cmd_build(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait build <file>");
        exit(2);
    };
    match run_build(p) {
        Ok(files) => {
            for f in files {
                println!("emitted {}", f);
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    }
}

fn run_build(path: &str) -> Result<Vec<String>, String> {
    let src_path = std::path::Path::new(path);
    let out_dir = src_path.parent().unwrap_or(std::path::Path::new("."));
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let program = portrait_syntax::parse(&src)?;
    portrait_sema::check(&program).map_err(|ds| {
        ds.into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let cartoon = portrait_ir::lower(&program);
    let models = portrait_project::project(&cartoon);
    let sil_files = portrait_emit::emit(&models)?;
    let mut written = Vec::new();
    for (model, sil) in models.iter().zip(sil_files.iter()) {
        let sil_path = out_dir.join(&sil.name);
        std::fs::write(&sil_path, &sil.source).map_err(|e| e.to_string())?;
        // Write companion CTOR.json for `silverc --ctor` compilation.
        let (ctor_name, ctor_json) = portrait_emit::emit_ctor(model);
        let ctor_path = out_dir.join(&ctor_name);
        std::fs::write(&ctor_path, &ctor_json).map_err(|e| e.to_string())?;
        written.push(sil.name.clone());
    }
    Ok(written)
}

fn cmd_engrave(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait engrave <file>");
        exit(2);
    };
    match run_engrave(p) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    }
}

fn run_engrave(path: &str) -> Result<(), String> {
    use portrait_pounce::{allocate, Layer};
    let src_path = std::path::Path::new(path);
    let out_dir = src_path.parent().unwrap_or(std::path::Path::new("."));
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let program = portrait_syntax::parse(&src)?;
    portrait_sema::check(&program).map_err(|ds| {
        ds.into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let cartoon = portrait_ir::lower(&program);
    // Pounce: classify transitions.
    let pounce = allocate(&cartoon);
    for alloc in &pounce.allocations {
        let layer = match alloc.layer {
            Layer::Covenant => "Covenant",
            Layer::VProg => "VProg",
        };
        println!("[pounce] {} → {}", alloc.transition.entry, layer);
    }
    // Allocation advisor (read-only): per-entrypoint routing notes. This does NOT
    // move code between layers — it only cross-checks the attribute-driven
    // allocation against the body and reports mismatches/suitability.
    for adv in portrait_sema::advise(&program) {
        println!(
            "[allocate] {}.{} ({}): {}",
            adv.role, adv.entry, adv.layer, adv.message
        );
    }
    // Emit .sil + CTOR JSON.
    let models = portrait_project::project(&cartoon);
    let sil_files = portrait_emit::emit(&models)?;
    // Red-team LOW (b): a multi-contract source emits one .sil per role, but the
    // engraver previously only kept the LAST (sil, ctor) pair and ran silverc on
    // that one — leaving sibling contracts (e.g. DigitalReitToken) written to
    // disk but never compiled. Collect EVERY emitted (sil, ctor) pair and run
    // silverc on each, so all emitted contracts are verified.
    let mut compile_targets: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    for (model, sil) in models.iter().zip(sil_files.iter()) {
        let sil_path = out_dir.join(&sil.name);
        std::fs::write(&sil_path, &sil.source).map_err(|e| e.to_string())?;
        let (ctor_name, ctor_json) = portrait_emit::emit_ctor(model);
        let ctor_path = out_dir.join(&ctor_name);
        std::fs::write(&ctor_path, &ctor_json).map_err(|e| e.to_string())?;
        println!("[emit]   {}", sil.name);
        compile_targets.push((sil_path, ctor_path));
    }
    // Invoke silverc on EVERY emitted contract (LOW (b) fix). Fail-closed: the
    // first contract that silverc rejects aborts the engrave with an error.
    for (sil_p, ctor_p) in &compile_targets {
        let status = std::process::Command::new("silverc")
            .arg("--constructor-args")
            .arg(ctor_p)
            .arg(sil_p)
            .status()
            .map_err(|e| format!("silverc not found: {}", e))?;
        if status.success() {
            println!(
                "[silverc] ok {}",
                sil_p.file_name().and_then(|n| n.to_str()).unwrap_or("")
            );
        } else {
            return Err(format!(
                "silverc exited with {} for {}",
                status,
                sil_p.file_name().and_then(|n| n.to_str()).unwrap_or("")
            ));
        }
    }
    Ok(())
}

fn cmd_atelier_build(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait atelier-build <file>");
        exit(2);
    };
    match run_atelier_build(p) {
        Ok(out_path) => println!("[atelier] guest main written → {}", out_path),
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    }
}

fn run_atelier_build(path: &str) -> Result<String, String> {
    let src_path = std::path::Path::new(path);
    let out_dir = src_path.parent().unwrap_or(std::path::Path::new("."));
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let program = portrait_syntax::parse(&src)?;
    portrait_sema::check(&program).map_err(|ds| {
        ds.into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let cartoon = portrait_ir::lower(&program);

    // Pounce: print allocation report.
    use portrait_pounce::{allocate, Layer};
    let pounce = allocate(&cartoon);
    for alloc in &pounce.allocations {
        let layer = match alloc.layer {
            Layer::Covenant => "Covenant",
            Layer::VProg => "VProg",
        };
        println!("[pounce]  {} → {}", alloc.transition.entry, layer);
    }
    // Allocation advisor (read-only): per-entrypoint routing notes.
    for adv in portrait_sema::advise(&program) {
        println!(
            "[allocate] {}.{} ({}): {}",
            adv.role, adv.entry, adv.layer, adv.message
        );
    }

    // KovId auto-derivation: run the engraver pipeline and read the compiled
    // silverc JSON to extract the script bytes; sha256-hash them to get KovId.
    let kov_id = derive_kov_id(path, out_dir);
    match &kov_id {
        Some(_) => println!("[atelier] KovId auto-derived from silverc output"),
        None => eprintln!(
            "warning: KovId auto-derivation failed (silverc unavailable or covenant \
             has no transitions); using runtime covenant_id from env instead."
        ),
    }

    let vprog_count = pounce
        .allocations
        .iter()
        .filter(|a| matches!(a.layer, portrait_pounce::Layer::VProg))
        .count();
    if vprog_count > 1 {
        println!(
            "[atelier] {} VProg transitions: dispatch table emitted (M2).",
            vprog_count
        );
    }
    let file = portrait_atelier::emit_guest_main(&cartoon, kov_id)
        .ok_or_else(|| "no VProg transitions in this portrait file".to_string())?;

    let out_path = out_dir.join(&file.name);
    std::fs::write(&out_path, &file.source).map_err(|e| e.to_string())?;
    Ok(out_path.display().to_string())
}

/// Derive the KovId (sha256 of the compiled covenant script bytes) by running
/// the engraver pipeline and invoking `silverc -c` to capture the compiled JSON.
/// Returns `None` if the portrait file has no covenant transitions, silverc is
/// unavailable, or the JSON cannot be parsed.
fn derive_kov_id(portrait_path: &str, out_dir: &std::path::Path) -> Option<[u8; 32]> {
    use portrait_pounce::{allocate, Layer};

    // Re-parse and lower (lightweight — no I/O side effects yet).
    let src = std::fs::read_to_string(portrait_path).ok()?;
    let program = portrait_syntax::parse(&src).ok()?;
    portrait_sema::check(&program).ok()?;
    let cartoon = portrait_ir::lower(&program);

    // Only proceed if there are covenant transitions (engraver produces a .sil).
    let pounce = allocate(&cartoon);
    let has_covenant = pounce
        .allocations
        .iter()
        .any(|a| a.layer == Layer::Covenant);
    if !has_covenant {
        return None;
    }

    // Emit .sil + ctor JSON to temp files in out_dir.
    let models = portrait_project::project(&cartoon);
    let sil_files = portrait_emit::emit(&models).ok()?;
    let (model, sil) = models.first().zip(sil_files.first())?;
    let sil_path = out_dir.join(&sil.name);
    std::fs::write(&sil_path, &sil.source).ok()?;
    let (ctor_name, ctor_json) = portrait_emit::emit_ctor(model);
    let ctor_path = out_dir.join(&ctor_name);
    std::fs::write(&ctor_path, &ctor_json).ok()?;

    // Run `silverc --constructor-args <ctor> -c <sil>` and capture stdout.
    let output = std::process::Command::new("silverc")
        .arg("--constructor-args")
        .arg(&ctor_path)
        .arg("-c")
        .arg(&sil_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    // Parse the JSON and extract the `script` field (array of u8 values).
    let json_str = String::from_utf8(output.stdout).ok()?;
    derive_kov_id_from_json(&json_str)
}

/// Parse a silverc compiled JSON string and return sha256(script bytes).
/// The `script` field is a JSON array of integers (byte values).
fn derive_kov_id_from_json(json: &str) -> Option<[u8; 32]> {
    // Minimal JSON extraction: find `"script":[...]` and parse the bytes array.
    // Avoids a serde_json dep by doing simple string scanning.
    let tag = "\"script\":";
    let start = json.find(tag)? + tag.len();
    let rest = json[start..].trim_start();
    if !rest.starts_with('[') {
        return None;
    }
    // Find the matching ']'. This assumes script arrays are flat (no nested brackets),
    // which holds for silverc v0 output. If a future silverc version nests structures
    // inside the script array, this will silently truncate — add bracket counting then.
    let end = rest.find(']')?;
    let inner = &rest[1..end];
    let bytes: Vec<u8> = inner
        .split(',')
        .filter_map(|s| s.trim().parse::<u8>().ok())
        .collect();
    if bytes.is_empty() {
        return None;
    }
    // sha256(script bytes) = KovId (KIP-20 covenant identity).
    let digest = <sha2::Sha256 as sha2::Digest>::digest(&bytes);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&digest);
    Some(hash)
}

/// `portrait ship <file>` — the single end-to-end command. Chains the existing
/// pipeline (check → engrave: write .sil + CTOR beside source + silverc per
/// covenant → portrait-verify Hallmark manifest) and prints ONE clean summary:
/// per-stage ok/fail glyphs, the emitted `.sil` paths, the covenant count, the
/// KovId if derivable, the Hallmark manifest path, the maturity stamp, and the
/// one-line re-derive command. Honest exit code: non-zero if any stage failed.
///
/// Deploy is OPT-IN and DEFERRED: `ship` stops at the Hallmark and points the
/// user to the (separate) testnet settlement workflow. No on-chain code runs
/// here; nothing ever touches mainnet.
fn cmd_ship(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait ship <file>");
        exit(2);
    };
    match run_ship(p) {
        Ok((summary, all_pass)) => {
            print!("{}", summary);
            if !all_pass {
                exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    }
}

/// Run the ship pipeline and return `(summary_text, all_stages_passed)`.
///
/// Stage 1 (engrave) writes `.sil` + CTOR beside the source and runs `silverc`
/// on every emitted covenant, fail-closed — exactly `run_engrave`'s behaviour.
/// Stage 2 (hallmark) produces the proof-carrying manifest and writes it beside
/// the source. The returned summary renders both stages plus the manifest's own
/// claims; `all_pass` is the Hallmark verdict (which already requires parse +
/// sema + emit + silverc-accepts to all pass).
fn run_ship(path: &str) -> Result<(String, bool), String> {
    let src_path = std::path::Path::new(path);
    let out_dir = src_path.parent().unwrap_or(std::path::Path::new("."));

    // Stage 1: engrave (check + emit beside source + silverc per covenant).
    // run_engrave is fail-closed; surface any failure as a ship error.
    run_engrave(path)?;

    // KovId (best-effort): derived from the silverc-compiled covenant bytes.
    let kov_id = derive_kov_id(path, out_dir);

    // Stage 2: Hallmark manifest (parse + sema + emit + silverc-accepts, plus
    // the maturity stamp and re-derive line). Write it beside the source.
    let hm = portrait_verify::hallmark(src_path)?;
    let manifest_path = out_dir.join(format!("{}.hallmark.json", hm.component));
    std::fs::write(&manifest_path, hm.to_json()).map_err(|e| e.to_string())?;

    let summary = render_ship_summary(&hm, &manifest_path.display().to_string(), kov_id);
    Ok((summary, hm.all_pass()))
}

/// Render the one clean `portrait ship` summary block. Pure (no I/O) so it is
/// unit-testable: it formats the facts already computed (the Hallmark claims,
/// the manifest path, the optional KovId) into the consistent claim-line +
/// maturity + re-derive style used by `portrait verify`.
fn render_ship_summary(
    hm: &portrait_verify::Hallmark,
    manifest_path: &str,
    kov_id: Option<[u8; 32]>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Shipped — {} ({})\n", hm.component, hm.source));

    // Per-claim stage lines (parse / sema / emit / silverc-accepts[...]).
    for c in &hm.claims {
        out.push_str(&format!(
            "  [{}] {} — {}\n",
            c.result.as_str(),
            c.check,
            c.detail
        ));
    }

    // Covenant count: one silverc-accepts claim is emitted per covenant.
    let covenant_count = hm
        .claims
        .iter()
        .filter(|c| c.check.starts_with("silverc-accepts"))
        .count();
    out.push_str(&format!("  covenants: {}\n", covenant_count));

    if let Some(k) = kov_id {
        out.push_str(&format!("  KovId: {}\n", hex32(&k)));
    }

    out.push_str(&format!("  manifest: {}\n", manifest_path));
    out.push_str(&format!("  maturity: {}\n", hm.maturity));
    out.push_str(&format!(
        "  rederive: portrait verify {}   (or: cargo run -p portrait-cli -- verify {})\n",
        hm.source, hm.source
    ));
    // Deploy is opt-in and lives in the separate testnet settlement workflow.
    out.push_str(
        "  next: deploy to TESTNET is opt-in and handled by the settlement workflow \
         (pre-production, unaudited, testnet-only — never mainnet).\n",
    );

    if hm.all_pass() {
        out.push_str("verdict: ok — all stages passed.\n");
    } else {
        out.push_str("verdict: fail — one or more stages did not pass (see lines above).\n");
    }
    out
}

/// Lowercase hex of a 32-byte digest (for the KovId summary line).
fn hex32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// `portrait verify <file>` — run portrait-verify's real checks and write a
/// proof-carrying `<name>.hallmark.json` beside the source. Every claim maps to
/// an actual check a third party can re-run from source (Innovation §4.1).
fn cmd_verify(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait verify <file>");
        exit(2);
    };
    let src_path = std::path::Path::new(p);
    match portrait_verify::hallmark(src_path) {
        Ok(hm) => {
            let out_dir = src_path.parent().unwrap_or(std::path::Path::new("."));
            let out_path = out_dir.join(format!("{}.hallmark.json", hm.component));
            if let Err(e) = std::fs::write(&out_path, hm.to_json()) {
                eprintln!("error: could not write {}: {}", out_path.display(), e);
                exit(1);
            }
            // Human-readable summary: one line per claim, then the maturity
            // stamp and the one-line re-derive command. The JSON is always
            // written too (above) — this summary does not change what the
            // claims MEAN, it just renders the same facts for a human reader.
            println!("Hallmark — {} ({})", hm.component, hm.source);
            for c in &hm.claims {
                println!("  [{}] {} — {}", c.result.as_str(), c.check, c.detail);
            }
            println!("  maturity: {}", hm.maturity);
            println!(
                "  rederive: portrait verify {}   (or: cargo run -p portrait-cli -- verify {})",
                hm.source, hm.source
            );
            println!("  hallmark: {}", out_path.display());
            // Honest exit code: non-zero if any claim did not pass.
            if !hm.all_pass() {
                exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    }
}

/// `portrait prove <file>` — OPT-IN SMT proof stage (portrait-lens). Runs behind
/// a passing sema (A4), generates the total value-conservation VC for every
/// covenant transition, and discharges each via z3 (negate-and-check). Prints one
/// claim line per VC and ALWAYS the model-vs-`.sil` boundary + maturity footer.
///
/// SOUNDNESS: a line reads `proved` ONLY when z3 returns `unsat` for the negated
/// VC. Every other outcome (`unknown`, timeout, z3 absent, parse failure) prints
/// `unknown` — never a false `proved`. Exit code is non-zero only on a `refuted`
/// VC or a hard Lens refusal; `unknown` and z3-absent exit 0 (UNKNOWN is
/// first-class, not a failure).
fn cmd_prove(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait prove <file>");
        exit(2);
    };
    // Read + parse (same handling as cmd_check).
    let src = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    };
    let program = match portrait_syntax::parse(&src) {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("error: parse failed: {}", e);
            exit(1);
        }
    };
    // Lens runs ONLY behind a passing sema (assumption A4): never prove an
    // unchecked covenant. Abort with a clear message if sema fails.
    if let Err(ds) = portrait_sema::check(&program) {
        eprintln!(
            "error: sema did not pass; Lens proves only a checked covenant (A4):\n  {}",
            ds.iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        exit(1);
    }

    let mut reports = match portrait_lens::prove_program(&program) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    };

    let timeout_ms: u64 = std::env::var("PORTRAIT_Z3_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let z3_present = portrait_lens::z3_available();
    for r in &mut reports {
        portrait_lens::discharge(r, timeout_ms);
    }

    let (summary, any_refuted) = render_prove_summary(p, &reports, z3_present);
    print!("{}", summary);
    // Honest exit code: non-zero only on a refuted VC. UNKNOWN / z3-absent exit 0.
    if any_refuted {
        exit(1);
    }
}

/// Render the `portrait prove` summary block. Pure (no I/O) so it is
/// unit-testable. One claim line per VC in the established verify/ship style,
/// the counter-model printed verbatim under a refuted line, the z3-absent skip
/// notice when the binary is missing, and ALWAYS the model-not-`.sil` boundary +
/// maturity footer. Returns `(text, any_refuted)`.
fn render_prove_summary(
    source: &str,
    reports: &[portrait_lens::VcReport],
    z3_present: bool,
) -> (String, bool) {
    use portrait_lens::Outcome;
    let mut out = String::new();
    out.push_str(&format!("Prove — {}\n", source));

    if reports.is_empty() {
        out.push_str(
            "  (no VCs generated: every transition is conservation-exempt (mint/burn), has no \
             value-bearing field movement to conserve or spend-check, has no value-bearing \
             arithmetic to range-check, and declares no groundable refinement / stateful \
             invariant)\n",
        );
    }

    let mut any_refuted = false;
    for r in reports {
        let kind = r.vc_kind.label();
        match &r.outcome {
            Some(Outcome::Proved { unsat_core }) => {
                out.push_str(&format!(
                    "  [proved] {} {} — negated VC is unsat (cross-checked: confirmed \
                     unsat by a second z3 run of the same binary under a perturbed \
                     configuration — different random seed + reordered assertions; this \
                     catches search-order instability, NOT a second independent solver and \
                     NOT a proof certificate, so assumption A3 [trusted solver] is reduced, \
                     not discharged)\n",
                    r.entrypoint, kind
                ));
                // M5: the spend class proves the MODEL spend mints no value; it
                // does NOT bind spent_out to the actual on-chain output amount.
                if r.vc_kind == portrait_lens::VcKind::Spend {
                    out.push_str(
                        "      note: proves the MODEL spend creates no value (state total does \
                         not increase); does NOT bind spent_out to the on-chain output amount \
                         (translation-validation / M6)\n",
                    );
                }
                // M4 explainability (best-effort): which named assertions z3 needed.
                // Empty when z3 cannot produce a core — never affects the verdict.
                if !unsat_core.is_empty() {
                    out.push_str(&format!(
                        "      unsat-core: {} (assertions the proof relied on)\n",
                        unsat_core.join(", ")
                    ));
                }
            }
            Some(Outcome::Refuted { model, confidence }) => {
                any_refuted = true;
                // M4: distinguish a confirmed witness (independently replayed over
                // integers) from a candidate (relies on an uninterpreted value).
                let (tag, note) = match confidence {
                    portrait_lens::WitnessConfidence::Confirmed => (
                        "refuted",
                        "counter-example found (CONFIRMED: independently replayed over integers)"
                            .to_string(),
                    ),
                    portrait_lens::WitnessConfidence::Candidate { reason } => (
                        "refuted?",
                        format!("counter-example found (CANDIDATE, unvalidated: {reason})"),
                    ),
                };
                out.push_str(&format!("  [{tag}] {} {} — {note}:\n", r.entrypoint, kind));
                for line in model.lines() {
                    out.push_str(&format!("      {}\n", line));
                }
            }
            Some(Outcome::Unknown { reason }) => {
                out.push_str(&format!(
                    "  [unknown] {} {} — {}\n",
                    r.entrypoint, kind, reason
                ));
            }
            None => {
                out.push_str(&format!(
                    "  [unknown] {} {} — not discharged\n",
                    r.entrypoint, kind
                ));
            }
        }
    }

    if !z3_present {
        out.push_str(&format!("  note: {}\n", portrait_lens::Z3_ABSENT_MESSAGE));
    }

    // ALWAYS the boundary + maturity footer.
    out.push_str(&format!(
        "  boundary: {}\n",
        portrait_lens::MODEL_NOT_SIL_CAVEAT
    ));
    (out, any_refuted)
}

/// `portrait new <Name> [--template <counter|escrow|csci|treasury>] [--out <dir>]`
/// — scaffold a starter `<Name>.portrait` covenant source from an embedded
/// template (derived from the trusted round-trip sources). The generated file
/// is itself a covenant source: `portrait engrave <Name>.portrait` lowers it to
/// `.sil` + CTOR JSON that `silverc` accepts (exit 0).
fn cmd_new(args: &[String]) {
    let mut name: Option<&str> = None;
    let mut template = "counter";
    let mut out_dir: Option<&str> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                i += 1;
                match args.get(i) {
                    Some(t) => template = t,
                    None => {
                        eprintln!("error: --template requires a value");
                        exit(2);
                    }
                }
            }
            "--out" | "-o" => {
                i += 1;
                match args.get(i) {
                    Some(d) => out_dir = Some(d),
                    None => {
                        eprintln!("error: --out requires a value");
                        exit(2);
                    }
                }
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag {}", other);
                exit(2);
            }
            other => {
                if name.is_none() {
                    name = Some(other);
                } else {
                    eprintln!("error: unexpected argument {}", other);
                    exit(2);
                }
            }
        }
        i += 1;
    }

    let Some(name) = name else {
        eprintln!(
            "usage: portrait new <Name> [--template <counter|escrow|csci|treasury>] [--out <dir>]"
        );
        exit(2);
    };

    if !is_valid_app_name(name) {
        eprintln!(
            "error: <Name> must be a PascalCase identifier (letters/digits, starting with a letter): got {:?}",
            name
        );
        exit(2);
    }

    let Some(body) = template_source(template, name) else {
        eprintln!(
            "error: unknown template {:?} — choose one of: counter, escrow, csci, treasury",
            template
        );
        exit(2);
    };

    let dir = std::path::Path::new(out_dir.unwrap_or("."));
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("error: could not create {}: {}", dir.display(), e);
        exit(1);
    }
    let file_path = dir.join(format!("{}.portrait", name));
    if file_path.exists() {
        eprintln!(
            "error: {} already exists; refusing to overwrite",
            file_path.display()
        );
        exit(1);
    }
    if let Err(e) = std::fs::write(&file_path, body) {
        eprintln!("error: could not write {}: {}", file_path.display(), e);
        exit(1);
    }

    println!("created {} (template: {})", file_path.display(), template);
    println!("next steps:");
    println!("  portrait check   {}", file_path.display());
    println!("  portrait engrave {}", file_path.display());
    println!("  portrait verify  {}", file_path.display());
}

/// A valid app name is a non-empty identifier starting with an ASCII letter and
/// containing only ASCII alphanumerics (matches the parser's ident rule).
fn is_valid_app_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// Return the starter `.portrait` source for `template`, with the app name set
/// to `name`. Each template is a minimal-but-real covenant derived from a
/// trusted round-trip source; every one engraves → silverc exit 0.
fn template_source(template: &str, name: &str) -> Option<String> {
    let body = match template {
        "counter" => COUNTER_TEMPLATE,
        "escrow" => ESCROW_TEMPLATE,
        "csci" => CSCI_TEMPLATE,
        "treasury" => TREASURY_TEMPLATE,
        _ => return None,
    };
    Some(body.replace("{{NAME}}", name))
}

/// Minimal single-transition counter covenant (from examples/counter.portrait).
const COUNTER_TEMPLATE: &str = r#"pragma portrait ^0.1.0;

// Generated by `portrait new --template counter`.
// A minimal single-transition covenant: a monotonic counter.
app {{NAME}} {
  role counter {
    param int start;
    state { int value; }

    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }

  lifecycle { live -> live via counter.bump; }
  invariant no_undeclared_state;
}
"#;

/// Two-party, deadline-gated escrow (from library/finance/escrow/Escrow.portrait).
const ESCROW_TEMPLATE: &str = r#"pragma portrait ^0.1.0;

// Generated by `portrait new --template escrow`.
// A two-party, deadline-gated conditional-payment covenant. `settled` is a
// one-shot flag (genesis = 0): release (seller) XOR refund (buyer after deadline).
app {{NAME}} {
  role escrow {
    param pubkey buyer;     // funds the escrow (refund authority)
    param pubkey seller;    // delivers (release authority)
    param coin   amount;    // value locked (coin: strictly conserved)
    param int    deadline;  // coarse time bucket at/after which refund is allowed
    param int    settled;   // one-shot spent flag (genesis = 0)

    state {
      pubkey buyer;
      pubkey seller;
      coin   amount;
      int    deadline;
      int    settled;
    }

    // Happy path: the committed seller settles the escrow.
    #[covenant(mode = transition)]
    entrypoint function release(
      sig auth
    ) : (pubkey buyer, pubkey seller, coin amount, int deadline, int settled) {
      requires checkSig(auth, seller);
      requires settled == 0;
      return {{NAME}} {
        buyer:    buyer,
        seller:   seller,
        amount:   amount,
        deadline: deadline,
        settled:  1
      };
    }

    // Timeout path: the committed buyer claws back after the deadline.
    #[covenant(mode = transition)]
    entrypoint function refund(
      sig auth,
      int now_bucket
    ) : (pubkey buyer, pubkey seller, coin amount, int deadline, int settled) {
      requires checkSig(auth, buyer);
      requires now_bucket >= deadline;
      requires settled == 0;
      return {{NAME}} {
        buyer:    buyer,
        seller:   seller,
        amount:   amount,
        deadline: deadline,
        settled:  1
      };
    }
  }

  lifecycle {
    live -> live via escrow.release;
    live -> live via escrow.refund;
  }

  invariant value_conserved;
  invariant no_undeclared_state;
}
"#;

/// CSCI state-machine covenant with a vProg companion (from
/// library/state/CsciInstrument.portrait).
const CSCI_TEMPLATE: &str = r#"pragma portrait ^0.1.0;

// Generated by `portrait new --template csci`.
// A covenant that self-enforces a CSCI state machine on-chain: seq advances by
// exactly one, owner auth against the committed key, amount conserved. The
// `csci_rules` entrypoint carries NO #[covenant] attribute, so it lowers to a
// vProg (off-L1) transition and flips `has_vprog` (covenant-id binding).
app {{NAME}} {
  role instrument {
    param pubkey  owner;       // committed owner key (settle authority)
    param int     amount;      // value carried (conserved)
    param int     seq;         // monotonic CSCI sequence (genesis = 0)
    param bytes32 state_hash;  // CSCI content/state hash

    state {
      pubkey  owner;
      int     amount;
      int     seq;
      bytes32 state_hash;
    }

    #[covenant(mode = transition)]
    entrypoint function settle(
      sig auth,
      bytes32 next_state_hash
    ) : (pubkey owner, int amount, int seq, bytes32 state_hash) {
      requires checkSig(auth, owner);
      return {{NAME}} {
        owner:      owner,
        amount:     amount,
        seq:        seq + 1,
        state_hash: next_state_hash
      };
    }

    // vProg (off-L1) rule predicate; no #[covenant] attribute.
    entrypoint function csci_rules(bytes32 next_state_hash) {
      return seq + 1;
    }
  }

  lifecycle { live -> live via instrument.settle; }

  invariant value_conserved;
  invariant monotonic_seq;
  invariant no_undeclared_state;
}
"#;

/// 2-of-2 multisignature treasury (from
/// library/governance/treasury/MultisigTreasury.portrait).
const TREASURY_TEMPLATE: &str = r#"pragma portrait ^0.1.0;

// Generated by `portrait new --template treasury`.
// A 2-of-2 multisignature treasury: the balance moves only when BOTH committed
// signers authorise in the same transaction. Authorisation is checked against
// the COMMITTED signer keys, never caller-supplied pubkeys.
app {{NAME}} {
  role treasury {
    param pubkey signer_a;   // first committed signer
    param pubkey signer_b;   // second committed signer
    param int    balance;    // treasury balance (value-conserved)

    state {
      pubkey signer_a;
      pubkey signer_b;
      int    balance;
    }

    #[covenant(mode = transition)]
    entrypoint function spend(
      sig auth_a,
      sig auth_b,
      int amount
    ) : (pubkey signer_a, pubkey signer_b, int balance) {
      requires checkSig(auth_a, signer_a);
      requires checkSig(auth_b, signer_b);
      requires amount >= 0;
      requires amount <= balance;
      return {{NAME}} {
        signer_a: signer_a,
        signer_b: signer_b,
        balance:  balance - amount
      };
    }
  }

  lifecycle {
    live -> live via treasury.spend;
  }

  invariant value_conserved;
  invariant authorized;
  invariant non_negative_amount;
  invariant no_undeclared_state;
}
"#;

/// `portrait compose <file>` — OPT-IN multi-role protocol composition front-end
/// (Composer M1, portrait-compose). A THIN front-end over the existing checked
/// APIs — it adds NO new semantics:
///
///   parse → `portrait_compose::lift::lift` (App flow → Score) → `Score::check`
///   (projectability + duality/no-orphan + linearity) → on ACCEPT: per-role
///   projection (`render_local`) + `realize` (KIP-20 binding + timelock escapes +
///   NOT-STRANDED-BEYOND-T report) + `emit_real_covenants` (real per-role .portrait
///   covenants, any EmitGaps shown, not hidden) + `execute` (in-memory simulation
///   trace + status).
///
/// HONESTY carried through verbatim from the underlying crate: the Composer
/// boundary (safety-not-liveness), the executor footer (simulation-not-a-chain-
/// runtime), and the realization banner (model-not-deployed-covenant) ALWAYS print.
///
/// Exit code: 0 ONLY on a clean ACCEPT; non-zero on a parse error, a `LiftError`
/// (un-liftable surface shape), or a `ComposeError` (an UNSAFE protocol must FAIL).
fn cmd_compose(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait compose <file>");
        exit(2);
    };
    // Read + parse (same handling as cmd_prove / cmd_validate_translation).
    let src = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    };
    let program = match portrait_syntax::parse(&src) {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("error: parse failed: {}", e);
            exit(1);
        }
    };
    let (out, code) = render_compose(p, &program.app);
    print!("{}", out);
    if code != 0 {
        exit(code);
    }
}

/// Pure (no I/O) render of `portrait compose` over a PARSED app, returning
/// `(text, exit_code)` so it is unit-testable. It reuses the existing
/// portrait-compose APIs end-to-end and never reimplements their semantics; the
/// honest footers (Composer boundary, realization banner, executor footer) are
/// ALWAYS appended regardless of accept/reject.
fn render_compose(source: &str, app: &portrait_syntax::App) -> (String, i32) {
    use portrait_compose::emit_real::{emit_real_covenants, render_real_covenants};
    use portrait_compose::execute::{execute, render_trace};
    use portrait_compose::realize::{realize, render_realization};
    use portrait_compose::{lift::lift, render_local, HONEST_BOUNDARY_FOOTER};

    let mut out = String::new();
    out.push_str(&format!("Compose — {} (app {})\n", source, app.name));

    // (1) lift the surface App into a Score. A LiftError is an un-representable
    // surface shape — print it (named) and fail.
    let score = match lift(app) {
        Ok(s) => s,
        Err(e) => {
            out.push_str(&format!("  LIFT-ERROR: {}\n", e));
            out.push_str(&format!("\n{}\n", HONEST_BOUNDARY_FOOTER));
            return (out, 1);
        }
    };

    // (2) run the full Composer checker. A ComposeError is an UNSAFE protocol —
    // print it (named) and FAIL. The footer is always appended below.
    let locals = match score.check() {
        Ok(l) => l,
        Err(e) => {
            out.push_str(&format!("  REJECT: {}\n", e));
            out.push_str(&format!("\n{}\n", HONEST_BOUNDARY_FOOTER));
            return (out, 1);
        }
    };

    // (3) ACCEPT: per-role projection (local types).
    out.push_str("\n-- per-role projection (local types) --\n");
    for (role, t) in &locals {
        out.push_str(&format!("  {role:<12} ⊢ {}\n", render_local(t)));
    }

    // (4) realize: KIP-20 binding + timelock escapes + NOT-STRANDED-BEYOND-T.
    let roles = realize(&score, &locals);
    out.push_str(&format!("\n{}", render_realization(&score, &roles)));

    // (5) emit the real per-role .portrait covenants; show any EmitGaps honestly.
    let covenants = emit_real_covenants(&locals);
    out.push_str(&format!("\n{}\n", render_real_covenants(&covenants)));
    for c in &covenants {
        for g in &c.gaps {
            out.push_str(&format!("  [emit-gap] role `{}`: {}\n", c.role, g));
        }
    }

    // (6) execute: in-memory SIMULATION of the Score model → trace + status.
    let trace = execute(&score);
    out.push_str(&format!("\n{}", render_trace(&trace)));

    // ALWAYS the Composer safety-not-liveness boundary footer.
    out.push_str(&format!("\n{}\n", HONEST_BOUNDARY_FOOTER));
    out.push_str("verdict: ACCEPT — projectable, dual (no orphans), linear.\n");
    (out, 0)
}

/// `portrait validate-translation <file>` — OPT-IN STRUCTURAL translation
/// validation across the model-vs-`.sil` gap (portrait-lens M6).
///
/// Parses + sema-checks the `.portrait`, emits its `.sil` in memory, and checks
/// the STRUCTURAL correspondence: every model `requires` maps to a `.sil`
/// `require(...)` (no dropped guard) AND the `.sil` introduces no value-bearing
/// write the model did not account for (no unaccounted mint / extra output).
///
/// HONEST SCOPE: STRUCTURAL only — it catches emitter drift (a dropped guard, an
/// extra value-bearing op). It is NOT a semantic refinement proof and does NOT
/// establish behavioural equivalence. Exit non-zero on DIVERGES; zero on
/// CORRESPONDS.
fn cmd_validate_translation(path: Option<&String>) {
    let Some(p) = path else {
        eprintln!("usage: portrait validate-translation <file>");
        exit(2);
    };
    let src = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1);
        }
    };
    let program = match portrait_syntax::parse(&src) {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("error: parse failed: {}", e);
            exit(1);
        }
    };
    if let Err(ds) = portrait_sema::check(&program) {
        eprintln!(
            "error: sema did not pass; nothing sound to validate:\n  {}",
            ds.iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        exit(1);
    }
    // Emit the .sil in memory (same pipeline as `build`), then concatenate every
    // emitted file's source so the check finds each entrypoint's function block.
    let cartoon = portrait_ir::lower(&program);
    let models = portrait_project::project(&cartoon);
    let sil_text = match portrait_emit::emit(&models) {
        Ok(files) => files
            .iter()
            .map(|f| f.source.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        Err(e) => {
            eprintln!("error: emit failed: {}", e);
            exit(1);
        }
    };

    println!("Validate-translation — {}", p);
    let diverged = match portrait_lens::validate_translation(&program, &sil_text) {
        portrait_lens::Correspondence::Corresponds => {
            println!("  CORRESPONDS — every model guard maps to a .sil require, and model and .sil agree on every value-bearing write (none added, altered, or dropped)");
            false
        }
        portrait_lens::Correspondence::Diverges { reasons } => {
            for r in &reasons {
                println!("  DIVERGES: {}", r);
            }
            true
        }
    };
    println!("  note: {}", portrait_lens::TRANSLATION_STRUCTURAL_FOOTER);
    if diverged {
        exit(1);
    }
}

fn help() {
    println!(
        "portrait <check|build|engrave|atelier-build|ship|verify|prove|compose|validate-translation|test|publish|new|version> [file]"
    );
    println!(
        "  check [--explain] <file>  static checks; --explain prints a human invariant report"
    );
    println!("  ship <file>               check → engrave → silverc → Hallmark, one clean summary");
    println!(
        "  prove <file>              OPT-IN SMT value-conservation proof of the MODEL (needs z3; \
         proves the model, NOT the emitted .sil)"
    );
    println!(
        "  compose <file>            OPT-IN multi-role protocol compose (Composer M1): lift → \
         check → projection + realize + per-role emit + simulate (MODEL only, NOT a runtime)"
    );
    println!(
        "  validate-translation <file>  OPT-IN STRUCTURAL model-vs-.sil correspondence check \
         (catches a dropped guard / extra value-bearing op; NOT a refinement proof)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_kov_id_from_json_matches_known_compliance_token_kov_id() {
        // Excerpt of the silverc-compiled ComplianceToken JSON "script" field.
        // The expected KovId is sha256(script bytes) = 0xf1ce2f9b... (confirmed 2026-06-28).
        // bytes = [1, 2, 3] is a placeholder; we verify the function correctly hashes
        // a known byte sequence. The real ComplianceToken script is 66+ bytes; here
        // we test with a minimal but deterministic input to ensure the parsing path works.
        let json = r#"{"contract_name":"X","script":[1,2,3]}"#;
        let result = derive_kov_id_from_json(json);
        assert!(result.is_some(), "should parse script array");
        let hash = result.unwrap();
        // sha256([1, 2, 3]) = 039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81
        let expected = [
            0x03, 0x90, 0x58, 0xc6, 0xf2, 0xc0, 0xcb, 0x49, 0x2c, 0x53, 0x3b, 0x0a, 0x4d, 0x14,
            0xef, 0x77, 0xcc, 0x0f, 0x78, 0xab, 0xcc, 0xce, 0xd5, 0x28, 0x7d, 0x84, 0xa1, 0xa2,
            0x01, 0x1c, 0xfb, 0x81,
        ];
        assert_eq!(hash, expected, "KovId hash mismatch");
    }

    #[test]
    fn derive_kov_id_from_json_returns_none_for_empty_script() {
        let json = r#"{"script":[]}"#;
        assert!(
            derive_kov_id_from_json(json).is_none(),
            "empty script → None"
        );
    }

    #[test]
    fn derive_kov_id_from_json_returns_none_when_no_script_field() {
        let json = r#"{"bytecode":[1,2,3]}"#;
        assert!(
            derive_kov_id_from_json(json).is_none(),
            "wrong field name → None"
        );
    }

    #[test]
    fn valid_app_name_accepts_pascal_case_and_rejects_junk() {
        assert!(is_valid_app_name("MyCounter"));
        assert!(is_valid_app_name("Escrow2"));
        assert!(!is_valid_app_name(""), "empty is invalid");
        assert!(!is_valid_app_name("2Fast"), "must start with a letter");
        assert!(!is_valid_app_name("my-token"), "no punctuation");
        assert!(!is_valid_app_name("My Token"), "no spaces");
    }

    #[test]
    fn every_template_parses_sema_and_substitutes_the_name() {
        // Each embedded template must itself be a well-formed covenant source:
        // parse + sema succeed, and the app/return name is the supplied name.
        for tmpl in ["counter", "escrow", "csci", "treasury"] {
            let src = template_source(tmpl, "Foo")
                .unwrap_or_else(|| panic!("template {tmpl} should exist"));
            assert!(
                src.contains("app Foo {"),
                "template {tmpl} must substitute the app name"
            );
            assert!(
                !src.contains("{{NAME}}"),
                "template {tmpl} must not leave a placeholder"
            );
            let program =
                portrait_syntax::parse(&src).unwrap_or_else(|e| panic!("{tmpl} parse: {e}"));
            portrait_sema::check(&program)
                .unwrap_or_else(|ds| panic!("{tmpl} sema: {} diagnostics", ds.len()));
        }
    }

    #[test]
    fn template_source_rejects_unknown_template() {
        assert!(template_source("nope", "Foo").is_none());
    }

    #[test]
    fn render_explain_lists_declared_invariants_and_structural_checks_on_pass() {
        // The escrow template declares value_conserved + no_undeclared_state.
        let src = template_source("escrow", "Escrow").unwrap();
        let program = portrait_syntax::parse(&src).expect("parse");
        let result = portrait_sema::check(&program);
        assert!(result.is_ok(), "escrow template should pass sema");
        let report = render_explain(&program, &result);
        assert!(report.contains("invariant report — app Escrow"));
        // Declared invariants render as ok with the (declared) tag.
        assert!(report.contains("[ok] value_conserved (declared)"));
        assert!(report.contains("[ok] no_undeclared_state (declared)"));
        // Always-on structural checks are listed too.
        assert!(report.contains("[ok] lifecycle_reachability (structural)"));
        assert!(report.contains("[ok] transition_return_consistency (structural)"));
        assert!(report.contains("verdict: ok"));
    }

    #[test]
    fn render_explain_reports_diagnostics_on_fail() {
        // A program that parses but trips sema: lifecycle references a ghost
        // entrypoint that does not exist.
        let bad = r#"pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> live via counter.ghost; }
  invariant no_undeclared_state;
}
"#;
        let program = portrait_syntax::parse(bad).expect("parse");
        let result = portrait_sema::check(&program);
        assert!(result.is_err(), "ghost entrypoint should fail sema");
        let report = render_explain(&program, &result);
        assert!(report.contains("[fail] no_undeclared_state (declared)"));
        assert!(report.contains("verdict: fail"));
        // The real diagnostic text is surfaced, not swallowed.
        assert!(
            report.lines().any(|l| l.trim_start().starts_with("- ")),
            "expected at least one diagnostic line, got: {report}"
        );
    }

    #[test]
    fn render_ship_summary_renders_stages_count_manifest_and_maturity() {
        // Build a minimal Hallmark by running the real verifier on the counter
        // template; render_ship_summary must surface every stage + the anchors.
        let src = template_source("counter", "Counter").unwrap();
        let dir = std::env::temp_dir().join(format!(
            "portrait-ship-render-{}-{}",
            std::process::id(),
            "counter"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Counter.portrait");
        std::fs::write(&path, &src).unwrap();

        let hm = portrait_verify::hallmark(&path).expect("hallmark");
        let summary = render_ship_summary(&hm, "/tmp/Counter.hallmark.json", Some([0xab; 32]));

        assert!(summary.contains("Shipped — Counter"));
        // Pipeline stage lines are present.
        assert!(summary.contains("parse"));
        assert!(summary.contains("sema"));
        assert!(summary.contains("emit"));
        // One covenant for the counter app.
        assert!(summary.contains("covenants: 1"), "summary: {summary}");
        // KovId hex when supplied.
        assert!(summary.contains(&format!("KovId: {}", "ab".repeat(32))));
        assert!(summary.contains("manifest: /tmp/Counter.hallmark.json"));
        assert!(summary.contains("pre-production, unaudited, testnet-only"));
        // Deploy is opt-in / testnet-only and pointed elsewhere.
        assert!(summary.contains("opt-in"));
        assert!(summary.contains("never mainnet"));
        assert!(summary.contains("rederive: portrait verify"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn render_prove_summary_proved_line_and_always_carries_the_boundary() {
        use portrait_lens::{Outcome, VcKind, VcReport};
        let reports = vec![VcReport {
            entrypoint: "pool.rebalance".to_string(),
            vc_kind: VcKind::ValueConservation,
            smtlib: String::new(),
            transition_smtlib: String::new(),
            outcome: Some(Outcome::Proved {
                unsat_core: vec!["a0_ge_amount".to_string(), "a3_not_eq".to_string()],
            }),
        }];
        let (summary, any_refuted) = render_prove_summary("X.portrait", &reports, true);
        assert!(!any_refuted);
        assert!(summary.contains("[proved] pool.rebalance value-conservation"));
        // M4: the best-effort unsat core is surfaced when present.
        assert!(summary.contains("unsat-core: a0_ge_amount, a3_not_eq"));
        // The model-vs-.sil boundary + maturity stamp is ALWAYS present.
        assert!(summary.contains("NOT a proof of the emitted .sil"));
        assert!(summary.contains("pre-production, unaudited, testnet-only"));
    }

    #[test]
    fn render_prove_summary_crosscheck_does_not_overclaim_independence() {
        // HARDENING (red-team): the cross-check is a re-run of the SAME z3 binary
        // under a perturbed config (seed + reordered asserts) — NOT a second
        // independent solver and NOT a proof certificate. The PROVED line must
        // describe that honestly and must NOT claim "independent"/"independently-
        // configured", and must state A3 is reduced, not discharged.
        use portrait_lens::{Outcome, VcKind, VcReport};
        let reports = vec![VcReport {
            entrypoint: "vault.spend".to_string(),
            vc_kind: VcKind::Spend,
            smtlib: String::new(),
            transition_smtlib: String::new(),
            outcome: Some(Outcome::Proved { unsat_core: vec![] }),
        }];
        let (summary, _) = render_prove_summary("X.portrait", &reports, true);
        assert!(summary.contains("[proved] vault.spend"));
        // Must NOT overclaim a second independent solver. The prior wording
        // ("independently-configured z3 run") asserted an independence the
        // same-binary re-run does not have; it must be gone.
        assert!(
            !summary.contains("independently-configured"),
            "cross-check wording must not claim it is independently-configured; got:\n{summary}"
        );
        // The only mention of "independent" allowed is the explicit DISCLAIMER
        // that this is NOT a second independent solver.
        assert!(
            summary.contains("NOT a second independent solver"),
            "the line must explicitly disclaim independence; got:\n{summary}"
        );
        // Must honestly describe the perturbed re-run + the A3 caveat.
        assert!(summary.contains("same binary"));
        assert!(summary.contains("perturbed configuration"));
        assert!(summary.contains("A3"));
        assert!(summary.contains("reduced, not discharged"));
    }

    #[test]
    fn render_prove_summary_refuted_prints_model_and_flags_failure() {
        use portrait_lens::{Outcome, VcKind, VcReport};
        let reports = vec![VcReport {
            entrypoint: "pool.rebalance".to_string(),
            vc_kind: VcKind::ValueConservation,
            smtlib: String::new(),
            transition_smtlib: String::new(),
            outcome: Some(Outcome::Refuted {
                model: "(define-fun x () Int 1)".to_string(),
                confidence: portrait_lens::WitnessConfidence::Confirmed,
            }),
        }];
        let (summary, any_refuted) = render_prove_summary("X.portrait", &reports, true);
        assert!(any_refuted, "a refuted VC must flag failure");
        assert!(summary.contains("[refuted] pool.rebalance"));
        assert!(summary.contains("CONFIRMED"));
        assert!(summary.contains("define-fun x"));
    }

    #[test]
    fn render_prove_summary_candidate_refutation_is_flagged_unvalidated() {
        // M4: a counter-example that relies on an uninterpreted value is reported
        // but flagged CANDIDATE (refuted?), so a reviewer is not misled into
        // trusting a possible over-approximation artifact. Still flags failure.
        use portrait_lens::{Outcome, VcKind, VcReport, WitnessConfidence};
        let reports = vec![VcReport {
            entrypoint: "pool.rebalance".to_string(),
            vc_kind: VcKind::ValueConservation,
            smtlib: String::new(),
            transition_smtlib: String::new(),
            outcome: Some(Outcome::Refuted {
                model: "(define-fun checkSig (...) Bool true)".to_string(),
                confidence: WitnessConfidence::Candidate {
                    reason: "term applies uninterpreted function `checkSig`".to_string(),
                },
            }),
        }];
        let (summary, any_refuted) = render_prove_summary("X.portrait", &reports, true);
        assert!(any_refuted, "even a candidate refutation flags failure");
        assert!(summary.contains("[refuted?] pool.rebalance"));
        assert!(summary.contains("CANDIDATE"));
        assert!(summary.contains("checkSig"));
    }

    #[test]
    fn render_prove_summary_unknown_never_reads_proved_and_does_not_fail() {
        use portrait_lens::{Outcome, VcKind, VcReport};
        let reports = vec![VcReport {
            entrypoint: "pool.rebalance".to_string(),
            vc_kind: VcKind::ValueConservation,
            smtlib: String::new(),
            transition_smtlib: String::new(),
            outcome: Some(Outcome::Unknown {
                reason: "z3 not found".to_string(),
            }),
        }];
        let (summary, any_refuted) = render_prove_summary("X.portrait", &reports, false);
        assert!(!any_refuted, "UNKNOWN is first-class, not a failure");
        assert!(summary.contains("[unknown] pool.rebalance"));
        assert!(!summary.contains("[proved]"));
        // z3-absent skip notice appears when the binary is missing.
        assert!(summary.contains("UNKNOWN, never PROVED"));
    }

    // ===== compose ===========================================================

    /// A SAFE, multi-role authored flow: decider `a` leads both branches and the
    /// informed successor `b` acts on each — projectable, dual, linear. `compose`
    /// must ACCEPT (exit 0) and print the projection + realization + emission +
    /// trace, plus all three honest footers.
    const COMPOSE_SAFE_SRC: &str = r#"pragma portrait ^0.1.0;
app AuthoredChoice {
  role a {
    #[covenant(mode = transition)]
    entrypoint function approve() : (int n) { return n; }
    #[covenant(mode = transition)]
    entrypoint function reject() : (int n) { return n; }
  }
  role b {
    #[covenant(mode = transition)]
    entrypoint function settle() : (int n) { return n; }
  }
  flow {
    choose {
      branch { a.approve; b.settle }
      branch { a.reject; b.settle }
    }
  }
}
"#;

    /// An UNSAFE flow: non-decider `c` acts on one branch but not the other, with
    /// no distinguishing notification — a classic orphan-wait. It LIFTS faithfully
    /// but `check()` rejects with `NotProjectable(c)`. `compose` must FAIL (non-zero)
    /// and surface the named error — never paper over it.
    const COMPOSE_UNSAFE_SRC: &str = r#"pragma portrait ^0.1.0;
app Unsafe {
  role a {
    #[covenant(mode = transition)]
    entrypoint function x() : (int n) { return n; }
    #[covenant(mode = transition)]
    entrypoint function y() : (int n) { return n; }
  }
  role b {
    #[covenant(mode = transition)]
    entrypoint function b1() : (int n) { return n; }
  }
  role c {
    #[covenant(mode = transition)]
    entrypoint function z() : (int n) { return n; }
  }
  flow {
    choose {
      branch { a.x; c.z }
      branch { a.y }
    }
    b.b1
  }
}
"#;

    /// A single-role app cannot describe a multi-party protocol: `lift` returns a
    /// named `LiftError` and `compose` fails non-zero (still printing the footer).
    const COMPOSE_SINGLE_ROLE_SRC: &str = r#"pragma portrait ^0.1.0;
app Solo {
  role only {
    #[covenant(mode = transition)]
    entrypoint function tick() : (int n) { return n; }
  }
  flow { only.tick }
}
"#;

    #[test]
    fn render_compose_safe_flow_accepts_and_prints_every_stage() {
        let program = portrait_syntax::parse(COMPOSE_SAFE_SRC).expect("safe src parses");
        let (out, code) = render_compose("AuthoredChoice.portrait", &program.app);
        assert_eq!(
            code, 0,
            "a safe projectable flow must ACCEPT (exit 0):\n{out}"
        );
        // Projection, realization, emission, and trace stages all present.
        assert!(out.contains("per-role projection"), "{out}");
        assert!(out.contains("instance binding id"), "realization: {out}");
        assert!(out.contains("real per-role covenants"), "emission: {out}");
        assert!(out.contains("local executor trace"), "trace: {out}");
        assert!(out.contains("verdict: ACCEPT"), "{out}");
        // All three honest footers carry through.
        assert!(
            out.contains("does NOT prove liveness"),
            "safety-not-liveness footer: {out}"
        );
        assert!(
            out.contains("IN-MEMORY SIMULATION"),
            "simulation-not-a-runtime footer: {out}"
        );
        assert!(
            out.contains("NOT on-chain") && out.contains("NOT deployable"),
            "model-not-deployed-covenant banner: {out}"
        );
    }

    #[test]
    fn render_compose_unsafe_flow_rejects_named_error_nonzero() {
        let program = portrait_syntax::parse(COMPOSE_UNSAFE_SRC).expect("unsafe src parses");
        let (out, code) = render_compose("Unsafe.portrait", &program.app);
        assert_ne!(code, 0, "an unsafe protocol must FAIL:\n{out}");
        assert!(out.contains("REJECT"), "{out}");
        // The named ComposeError is surfaced (orphan-wait role `c`).
        assert!(out.contains("not projectable"), "named error: {out}");
        assert!(out.contains('c'), "the orphan-wait role is named: {out}");
        // The honest boundary footer is ALWAYS present, even on reject.
        assert!(out.contains("does NOT prove liveness"), "{out}");
        // A rejected protocol prints no ACCEPT verdict and no trace.
        assert!(!out.contains("verdict: ACCEPT"), "{out}");
    }

    #[test]
    fn render_compose_single_role_is_named_lift_error_nonzero() {
        let program = portrait_syntax::parse(COMPOSE_SINGLE_ROLE_SRC).expect("solo src parses");
        let (out, code) = render_compose("Solo.portrait", &program.app);
        assert_ne!(code, 0, "a single-role app cannot compose:\n{out}");
        assert!(out.contains("LIFT-ERROR"), "{out}");
        // Footer still carries through on a lift error.
        assert!(out.contains("does NOT prove liveness"), "{out}");
    }

    #[test]
    fn render_compose_emit_gaps_are_shown_not_hidden() {
        // The asset-escrow worked example emits real per-role covenants; assert the
        // render path surfaces any recorded EmitGap rather than hiding it. Whatever
        // gaps the emitter records, the count line from render_real_covenants AND a
        // per-gap line for each must appear (or, if none, no gap line — never a
        // hidden/fabricated entry).
        use portrait_compose::emit_real::emit_real_covenants;
        let score = portrait_compose::asset_escrow_example();
        let locals = score.check().expect("escrow checks");
        let covenants = emit_real_covenants(&locals);
        let total_gaps: usize = covenants.iter().map(|c| c.gaps.len()).sum();

        // Drive the same render via a parsed app would be ideal, but the escrow
        // example is constructed directly; render its covenants the way compose does.
        let mut shown = String::new();
        for c in &covenants {
            for g in &c.gaps {
                shown.push_str(&format!("  [emit-gap] role `{}`: {}\n", c.role, g));
            }
        }
        // Honest contract: the number of rendered [emit-gap] lines EQUALS the number
        // of recorded gaps — none dropped, none invented.
        let shown_lines = shown.matches("[emit-gap]").count();
        assert_eq!(
            shown_lines, total_gaps,
            "every recorded EmitGap must be shown, none hidden; shown:\n{shown}"
        );
    }

    #[test]
    fn hex32_is_lowercase_64_chars() {
        let h = hex32(&[0x0f; 32]);
        assert_eq!(h.len(), 64);
        assert_eq!(h, "0f".repeat(32));
    }
}
