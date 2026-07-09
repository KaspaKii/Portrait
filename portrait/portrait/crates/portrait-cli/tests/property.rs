//! Differential / property harness for the Portrait emitter (faithfulness net).
//!
//! Sibling to `golden.rs`. Where `golden.rs` pins a handful of committed
//! fixtures, this file answers the external reviewer's "how do you know the
//! emitter is faithful?" by GENERATING hundreds of well-typed covenant programs
//! and checking two emitter-faithfulness properties against each:
//!
//!   (b) FAITHFULNESS TO SILVERSCRIPT — a generated well-typed covenant program
//!       (a canonical role/lifecycle skeleton wrapping K generated Bool guards +
//!       a value-conserving return) drives the FULL pipeline
//!       (parse → sema::check → lower → project → emit) and the emitted `.sil`
//!       compiles under the REAL `silverc` with exit 0. This proves the emitter
//!       targets real silverscript syntax across the generated space, not just
//!       the committed fixtures. Skipped-with-message (never silently passed) if
//!       silverc is absent.
//!
//!   (c) GUARD PRESERVATION — for a generated covenant whose body carries EXACTLY
//!       K source `requires` clauses, the emitted `.sil` contains EXACTLY K
//!       `require(` occurrences in the entrypoint (count match). This is the
//!       automated generalization of the hand-fixed guard-drop bug (a `Stmt::Raw`
//!       guard was once silently dropped, yielding a covenant that LOOKED gated
//!       but enforced nothing).
//!
//! Property (a) — precedence/paren round-trip — lives in portrait-emit's in-crate
//! unit tests, because it must call the PRIVATE `emit_expr`. It is the decisive
//! one; (b)/(c) here complement it at the whole-program / real-compiler level.
//!
//! DETERMINISM: a hand-rolled, seeded LCG generator (no proptest/quickcheck — the
//! portrait workspace is deliberately near-zero-dependency). A fixed seed list
//! drives the recursion, so any failure reprints the exact seed + source and
//! re-runs identically. No new dependencies; no Cargo.toml edits.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use portrait_emit::emit_ctor;
use portrait_ir::CovenantModel;

// ---------------------------------------------------------------------------
// silverc location + differential compile — same contract as golden.rs
// (kept local so this file is self-contained and the skip-with-message
// behavior is identical: present-but-rejects → assert fail; absent → skip).
// ---------------------------------------------------------------------------

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

fn temp_workdir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    dir.push(format!("portrait-property-{tag}-{stamp}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Returns `Some(true)` if silverc ran and accepted, `None` if silverc absent.
/// Panics (test fail) if silverc is present but REJECTS the emitted `.sil`.
fn try_differential_compile(label: &str, model: &CovenantModel, sil: &str) -> Option<bool> {
    let silverc = find_silverc()?;
    let dir = temp_workdir(label);
    let sil_path = dir.join(format!("{}.sil", model.name));
    fs::write(&sil_path, sil).expect("write sil");
    let (ctor_name, ctor_json) = emit_ctor(model);
    let ctor_path = dir.join(ctor_name);
    fs::write(&ctor_path, ctor_json).expect("write ctor json");

    let output = Command::new(&silverc)
        .arg("--ctor")
        .arg(&ctor_path)
        .arg("-c")
        .arg(&sil_path)
        .output()
        .unwrap_or_else(|e| panic!("[{label}] failed to spawn silverc ({silverc:?}): {e}"));

    assert!(
        output.status.success(),
        "[{label}] DIFFERENTIAL FAIL: silverc rejected the GENERATED .sil (exit {:?}).\n\
         --- sil ---\n{sil}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Some(true)
}

// ---------------------------------------------------------------------------
// Seeded deterministic generator of well-typed covenant SOURCES.
// ---------------------------------------------------------------------------

/// Minimal deterministic LCG (same constants as portrait-emit's property (a)).
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed.wrapping_mul(6364136223846793005).wrapping_add(1))
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }
    fn choice<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u32) as usize]
    }
}

/// Int-typed identifiers that the skeleton declares as params + state fields.
/// NONE end in `balance` (so the value-conservation refinement is not engaged —
/// the skeleton only declares `no_undeclared_state`). Generated arithmetic over
/// these is well-typed `int`.
const INT_IDENTS: &[&str] = &["counter", "seq", "limit", "delta"];

/// Render a well-typed INT surface expression of bounded depth.
fn gen_int_expr(rng: &mut Lcg, depth: u32) -> String {
    if depth == 0 {
        return if rng.below(2) == 0 {
            (rng.below(20)).to_string() // non-negative literal
        } else {
            (*rng.choice(INT_IDENTS)).to_string()
        };
    }
    match rng.below(3) {
        0 => {
            let op = *rng.choice(&["+", "-", "*"]);
            // Parenthesize children freely so mixed-precedence shapes (the
            // bug-prone case) flow through the full pipeline + silverc.
            format!(
                "({} {} {})",
                gen_int_expr(rng, depth - 1),
                op,
                gen_int_expr(rng, depth - 1)
            )
        }
        _ => {
            let op = *rng.choice(&["+", "-", "*"]);
            format!(
                "{} {} {}",
                gen_int_expr(rng, depth - 1),
                op,
                gen_int_expr(rng, depth - 1)
            )
        }
    }
}

/// One comparison: `<int> <cmp> <int>`. The left operand is generated with
/// `lead = true` so it never STARTS with a `(` — a guard that begins with `(`
/// collides with the `require(expr)` surface form (the require parser would treat
/// the leading `(` as the call-paren), so the top-level guard must not start with
/// one. Inner parenthesization is still freely produced on the right.
fn gen_comparison(rng: &mut Lcg) -> String {
    let op = *rng.choice(&["==", "!=", ">=", "<=", ">", "<"]);
    format!(
        "{} {} {}",
        gen_int_expr_lead(rng, 2),
        op,
        gen_int_expr(rng, 2)
    )
}

/// Like `gen_int_expr` but guarantees the rendered string does NOT start with
/// `(`, so a guard built left-to-right from it never begins with `(`.
fn gen_int_expr_lead(rng: &mut Lcg, depth: u32) -> String {
    if depth == 0 {
        return if rng.below(2) == 0 {
            (rng.below(20)).to_string()
        } else {
            (*rng.choice(INT_IDENTS)).to_string()
        };
    }
    // Always a bare (un-parenthesized) binary so the leading token is itself a
    // lead expression; the RIGHT child may parenthesize freely.
    let op = *rng.choice(&["+", "-", "*"]);
    format!(
        "{} {} {}",
        gen_int_expr_lead(rng, depth - 1),
        op,
        gen_int_expr(rng, depth - 1)
    )
}

/// Render a well-typed BOOL guard: a comparison, optionally combined with
/// `&&` / `||`. The top-level guard never starts with `(` (see `gen_comparison`).
fn gen_bool_guard(rng: &mut Lcg, depth: u32) -> String {
    if depth == 0 {
        return gen_comparison(rng);
    }
    match rng.below(3) {
        0 => {
            let op = *rng.choice(&["&&", "||"]);
            // Left side must not start with `(`; right side may.
            format!(
                "{} {} ({})",
                gen_bool_guard(rng, depth - 1),
                op,
                gen_bool_guard(rng, depth - 1)
            )
        }
        _ => gen_comparison(rng),
    }
}

/// Build a full, well-typed covenant SOURCE with exactly `k_guards` `requires`
/// clauses over the generated guards, plus a value-conserving scalar return
/// (`return counter + delta;`). Mirrors the OwnableCounter / counter skeleton.
fn gen_program(rng: &mut Lcg, k_guards: usize) -> String {
    let mut requires = String::new();
    for _ in 0..k_guards {
        requires.push_str(&format!("      requires {};\n", gen_bool_guard(rng, 1)));
    }
    format!(
        "pragma portrait ^0.1.0;\n\
\n\
app GenCov {{\n\
  role r {{\n\
    param int counter;\n\
    param int seq;\n\
    param int limit;\n\
\n\
    state {{\n\
      int counter;\n\
      int seq;\n\
      int limit;\n\
    }}\n\
\n\
    #[covenant(mode = transition)]\n\
    entrypoint function step(int delta) : (int counter, int seq, int limit) {{\n\
{requires}\
      return GenCov {{ counter: counter + delta, seq: seq, limit: limit }};\n\
    }}\n\
  }}\n\
\n\
  lifecycle {{ live -> live via r.step; }}\n\
  invariant no_undeclared_state;\n\
}}\n"
    )
}

/// Run the full library pipeline on a source, returning (model, sil) or an Err
/// string naming the failing stage.
fn run_pipeline(src: &str) -> Result<(CovenantModel, String), String> {
    let program = portrait_syntax::parse(src).map_err(|e| format!("parse: {e}"))?;
    portrait_sema::check(&program).map_err(|ds| {
        let msgs: Vec<_> = ds.into_iter().map(|d| d.message).collect();
        format!("sema: {}", msgs.join("; "))
    })?;
    let cartoon = portrait_ir::lower(&program);
    let models = portrait_project::project(&cartoon);
    let sil_files = portrait_emit::emit(&models).map_err(|e| format!("emit: {e}"))?;
    let model = models.into_iter().next().ok_or("no model projected")?;
    let sil = sil_files.into_iter().next().ok_or("no sil emitted")?;
    Ok((model, sil.source))
}

const SEEDS: u64 = 200;

// ---------------------------------------------------------------------------
// (b) FAITHFULNESS TO SILVERSCRIPT — generated covenant → pipeline → silverc.
// ---------------------------------------------------------------------------

#[test]
fn property_b_generated_covenants_compile_under_silverc() {
    let silverc_present = find_silverc().is_some();
    if !silverc_present {
        eprintln!(
            "SKIP[property_b]: silverc not found on PATH nor at $HOME/.cargo/bin/silverc \
             — generated-covenant differential check skipped (NOT silently passed)."
        );
    }
    let mut compiled = 0u64;
    for seed in 0..SEEDS {
        let mut rng = Lcg::new(seed.wrapping_add(7_000_000));
        let k = (seed % 3) as usize + 1; // 1..=3 guards
        let src = gen_program(&mut rng, k);
        let (model, sil) = run_pipeline(&src).unwrap_or_else(|e| {
            panic!("seed {seed}: generated covenant must drive the pipeline cleanly: {e}\n--- src ---\n{src}")
        });
        let label = format!("genb-{seed}");
        if try_differential_compile(&label, &model, &sil).is_some() {
            compiled += 1;
        }
    }
    if silverc_present {
        assert_eq!(
            compiled, SEEDS,
            "every generated covenant must compile under silverc when it is present"
        );
    } else {
        assert_eq!(compiled, 0, "no compiles expected when silverc is absent");
    }
}

// ---------------------------------------------------------------------------
// (c) GUARD PRESERVATION — source require-count == emitted require( count.
// ---------------------------------------------------------------------------

#[test]
fn property_c_guard_count_is_preserved() {
    let mut checked = 0u64;
    for seed in 0..SEEDS {
        let mut rng = Lcg::new(seed.wrapping_add(13_000_000));
        let k = (seed % 4) as usize + 1; // 1..=4 source guards
        let src = gen_program(&mut rng, k);
        // Sanity: the source really has exactly k `requires` clauses.
        let src_requires = src.matches("requires ").count();
        assert_eq!(
            src_requires, k,
            "seed {seed}: generator must emit exactly {k} source requires (got {src_requires})\n{src}"
        );
        let (_model, sil) = run_pipeline(&src)
            .unwrap_or_else(|e| panic!("seed {seed}: pipeline failed: {e}\n--- src ---\n{src}"));
        // The emitted .sil must carry exactly k `require(` calls (this covenant
        // has no vProg, so there is no injected OpInputCovenantId require to
        // discount). A dropped guard (the hand-fixed bug class) would show fewer.
        let emitted_requires = sil.matches("require(").count();
        assert_eq!(
            emitted_requires, k,
            "seed {seed}: GUARD-DROP — emitted require( count {emitted_requires} != source {k}.\n--- sil ---\n{sil}"
        );
        checked += 1;
    }
    assert_eq!(checked, SEEDS);
}

// ---------------------------------------------------------------------------
// NON-VACUITY for (c) — prove the guard-count net would catch a dropped guard.
// We do NOT resurrect the bug; instead we demonstrate that DELETING one require
// from a generated source drops the emitted require( count below the original,
// i.e. the equality assertion in property (c) would fire. This documents that
// (c) is the automated generalization of the hand-fixed guard-drop fix.
// ---------------------------------------------------------------------------

#[test]
fn non_vacuity_property_c_detects_a_dropped_guard() {
    let mut rng = Lcg::new(42);
    let src = gen_program(&mut rng, 3);
    let (_m, sil_full) = run_pipeline(&src).expect("baseline pipeline");
    let full_count = sil_full.matches("require(").count();
    assert_eq!(
        full_count, 3,
        "baseline must emit 3 require( calls:\n{sil_full}"
    );

    // Drop exactly ONE source `requires` line (simulating the guard-drop bug at
    // the source level) and re-run: the emitted require( count MUST fall to 2,
    // which is exactly what property (c)'s equality assertion would reject if the
    // EMITTER (rather than the source) had dropped it.
    let first = src.find("      requires ").expect("has a requires line");
    let line_end = src[first..].find('\n').expect("requires line ends") + first + 1;
    let dropped: String = format!("{}{}", &src[..first], &src[line_end..]);
    let (_m2, sil_dropped) = run_pipeline(&dropped).expect("dropped pipeline");
    let dropped_count = sil_dropped.matches("require(").count();

    assert_eq!(
        dropped_count, 2,
        "removing one source guard must reduce the emitted require( count to 2 \
         (proving property (c) would catch an emitter that dropped a guard):\n{sil_dropped}"
    );
    assert!(
        dropped_count < full_count,
        "non-vacuity: a dropped guard must be observable as a lower require( count"
    );
}
