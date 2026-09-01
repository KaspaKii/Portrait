//! z3 dispatch (M2). The `z3` binary is invoked as an external runtime process —
//! it is NOT a Cargo dependency — so the default build/test stays dep-free and
//! green whether or not z3 is installed.
//!
//! SOUNDNESS: a `PROVED` requires TWO solver verdicts: the negated VC must be
//! `unsat` AND the transition relation `T` alone must be `sat` (the entrypoint is
//! reachable — the guards are not self-contradictory). A vacuously-`unsat` query
//! (over a contradictory `T`) maps to [`Outcome::Unknown`], NEVER `Proved`. `sat`
//! for the negated VC maps to [`Outcome::Refuted`] with the captured `get-model`
//! text. EVERY other path — z3 absent, spawn error, non-verdict exit, `unknown`,
//! timeout, or a parse failure — maps to [`Outcome::Unknown`].

use std::io::Write;
use std::process::{Command, Stdio};

use crate::sexpr::{parse_all, Sexpr};
use crate::validate::validate_refutation;
use crate::{Outcome, Z3_ABSENT_MESSAGE};

/// The raw `check-sat` verdict from one z3 invocation, before it is mapped to an
/// [`Outcome`]. Keeps the vacuity probe (over `T` alone) and the conservation
/// query (over `T ∧ ¬VC`) sharing one parser.
#[derive(Debug)]
enum Verdict {
    /// `unsat`.
    Unsat,
    /// `sat`, with the captured `get-model` body (possibly empty).
    Sat { model: String },
    /// `unknown`, timeout, an error line, empty output, or an unexpected token —
    /// carries a human-readable reason.
    Indecisive { reason: String },
    /// z3 was absent or the process could not be run — carries a reason.
    Unavailable { reason: String },
}

/// The `z3` binary path: `$PORTRAIT_Z3` if set, else `z3` (resolved on `PATH`).
fn z3_path() -> String {
    std::env::var("PORTRAIT_Z3").unwrap_or_else(|_| "z3".to_string())
}

/// Probe once whether a usable `z3` binary is reachable. Runs `z3 --version`;
/// `true` only on a clean spawn + success.
pub fn z3_available() -> bool {
    Command::new(z3_path())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Discharge a conservation VC by the negate-and-check protocol, GUARDED by a
/// vacuity probe.
///
/// `transition_smtlib` asserts the transition relation `T` alone; `full_smtlib`
/// adds the negated VC. SOUNDNESS:
/// - `full` is `sat`  ⇒ [`Outcome::Refuted`] (a concrete counter-example).
/// - `full` is `unsat` ⇒ the negated VC is unsatisfiable. Before mapping this to
///   PROVED we probe `T` ALONE: only if `T` is `sat` (the entrypoint is
///   reachable) is the result genuinely [`Outcome::Proved`]. If `T` is `unsat`
///   the guards are self-contradictory and the query was vacuously `unsat` —
///   [`Outcome::Unknown`] ("vacuous transition"). If the probe is itself
///   indecisive/unavailable we cannot certify non-vacuity ⇒ [`Outcome::Unknown`].
/// - any other `full` verdict ⇒ [`Outcome::Unknown`].
pub fn discharge(transition_smtlib: &str, full_smtlib: &str, timeout_ms: u64) -> Outcome {
    match run_z3(full_smtlib, timeout_ms) {
        Verdict::Sat { model } => {
            // A REFUTED must carry a real witness. If z3 produced no model body,
            // fall to UNKNOWN rather than report an empty counter-example.
            if model.is_empty() {
                Outcome::Unknown {
                    reason: "z3 returned sat but produced no counter-model".to_string(),
                }
            } else {
                // M4: do NOT blindly trust the `sat`. Independently replay the
                // model's concrete integer/boolean assignment in Rust and either
                // CONFIRM the violation or flag it CANDIDATE (uninterpreted-value
                // dependent / unreproducible). This NEVER touches PROVED/UNKNOWN.
                let confidence = validate_refutation(full_smtlib, &model);
                Outcome::Refuted { model, confidence }
            }
        }
        Verdict::Unsat => {
            // The negated VC is unsat. Guard against a VACUOUS proof: probe T alone.
            match run_z3(transition_smtlib, timeout_ms) {
                // T is satisfiable ⇒ the entrypoint is reachable ⇒ a candidate
                // PROVED. M5 SOLVER CROSS-CHECK (reduces assumption A3 = "trust z3
                // once" in a LIMITED sense — see `crosscheck_unsat`): before we
                // report PROVED, re-run the SAME negated VC query through the SAME
                // z3 binary under a perturbed configuration (different random seed +
                // a reordered query) and require it to ALSO return `unsat`. Two
                // agreeing runs catch search-order instability but NOT a
                // deterministic soundness bug. If the cross-check does NOT confirm
                // `unsat`, DOWNGRADE
                // to UNKNOWN — we will not stand a PROVED on a single solver run
                // that a second config could not reproduce. (Still trusts z3, not a
                // verified kernel; A3 is reduced, not discharged.)
                Verdict::Sat { .. } => match crosscheck_unsat(full_smtlib, timeout_ms) {
                    CrossCheck::Confirmed => Outcome::Proved {
                        // M4: best-effort unsat core (explainability ONLY — never
                        // changes the soundness of PROVED; empty if z3 cannot
                        // produce one).
                        unsat_core: unsat_core(full_smtlib, timeout_ms),
                    },
                    CrossCheck::Disagreed { reason } => Outcome::Unknown {
                        reason: format!(
                            "negated VC unsat on the primary run, but the independent \
                             cross-check run ({reason}) did not confirm unsat; \
                             reporting UNKNOWN, never PROVED"
                        ),
                    },
                },
                // T is self-contradictory ⇒ the unsat was vacuous. NEVER PROVED.
                Verdict::Unsat => Outcome::Unknown {
                    reason: "vacuous transition: guard conjunction unsatisfiable \
                             (entrypoint unreachable); reporting UNKNOWN, never PROVED"
                        .to_string(),
                },
                // The probe could not decide / z3 vanished ⇒ cannot certify
                // non-vacuity ⇒ fall closed to UNKNOWN, never PROVED.
                Verdict::Indecisive { reason } | Verdict::Unavailable { reason } => {
                    Outcome::Unknown {
                        reason: format!(
                            "negated VC unsat but vacuity probe was indecisive ({reason}); \
                             reporting UNKNOWN, never PROVED"
                        ),
                    }
                }
            }
        }
        Verdict::Indecisive { reason } => Outcome::Unknown { reason },
        Verdict::Unavailable { reason } => Outcome::Unknown { reason },
    }
}

/// The result of the M5 independent cross-check of a candidate PROVED.
#[derive(Debug)]
enum CrossCheck {
    /// A second z3 run of the SAME binary, under a perturbed configuration
    /// (different random seed + reordered assertions), ALSO returned `unsat` for
    /// the negated VC. This agrees on a *re-run*, not via an independent solver or
    /// a proof certificate, so it catches search-order instability but NOT a
    /// deterministic solver-soundness bug (which would reproduce identically). The
    /// PROVED stands; A3 is reduced, not discharged.
    Confirmed,
    /// The independent run did not confirm `unsat` (it returned `sat`, `unknown`,
    /// timed out, errored, or z3 vanished). Carries a reason; the verdict must
    /// downgrade to UNKNOWN.
    Disagreed { reason: String },
}

/// M5 SOLVER CROSS-CHECK: re-discharge the already-`unsat` negated VC through z3
/// under a PERTURBED configuration and require it to ALSO return `unsat`. Two
/// agreeing runs reduce assumption A3 ("trust z3 once") only in a LIMITED sense:
/// the second run is the SAME z3 binary on the SAME theory, so it can surface
/// search-order *instability* (a wrong answer that depends on seed/assertion
/// order) but CANNOT surface a deterministic solver-soundness bug, which would
/// reproduce identically in both runs. This is NOT a second independent solver
/// and NOT a proof-certificate check; A3 is reduced, not discharged.
///
/// The second run differs from the primary in TWO ways:
/// 1. **Different solver randomness** — `sat.random_seed` AND `smt.random_seed`
///    are set to a non-default value via command-line params, exploring a
///    different search order.
/// 2. **Reordered assertions** — the `(assert ...)` forms are emitted in REVERSE
///    order, so the solver ingests the constraints in a different sequence.
///
/// `Confirmed` only on a literal `unsat`. Any other outcome (`sat`, `unknown`,
/// timeout, error, z3 absent, or a document we cannot reorder) ⇒ `Disagreed`,
/// which the caller maps to UNKNOWN (never a single-run PROVED).
fn crosscheck_unsat(full_smtlib: &str, timeout_ms: u64) -> CrossCheck {
    let Some(reordered) = reorder_assertions(full_smtlib) else {
        return CrossCheck::Disagreed {
            reason: "could not reorder the query for an independent run".to_string(),
        };
    };
    // Genuinely different config: non-default random seeds + reordered assertions.
    let opts = [
        "sat.random_seed=42".to_string(),
        "smt.random_seed=42".to_string(),
    ];
    match run_z3_with_opts(&reordered, timeout_ms, &opts) {
        Verdict::Unsat => CrossCheck::Confirmed,
        Verdict::Sat { .. } => CrossCheck::Disagreed {
            reason: "different-seed + reordered run returned sat".to_string(),
        },
        Verdict::Indecisive { reason } | Verdict::Unavailable { reason } => {
            CrossCheck::Disagreed { reason }
        }
    }
}

/// Re-render an SMT-LIB document with its `(assert ...)` forms in REVERSE order
/// (every other form — logic, declarations, check-sat — kept in place). Used by
/// the cross-check to feed z3 the constraints in a different sequence. Returns
/// `None` if the document cannot be parsed.
fn reorder_assertions(smtlib: &str) -> Option<String> {
    let forms = parse_all(smtlib).ok()?;
    // Collect the rendered assert forms, then re-emit the document substituting
    // the asserts in reverse order at the positions the asserts occupied.
    let assert_rendered: Vec<String> = forms
        .iter()
        .filter(|f| matches!(f, Sexpr::List(parts) if head_is(parts, "assert")))
        .map(Sexpr::render)
        .collect();
    let mut remaining = assert_rendered.into_iter().rev();
    let mut out = String::new();
    for form in &forms {
        match form {
            Sexpr::List(parts) if head_is(parts, "assert") => {
                // Substitute the next reversed assert at this position.
                if let Some(a) = remaining.next() {
                    out.push_str(&a);
                    out.push('\n');
                }
            }
            other => {
                out.push_str(&other.render());
                out.push('\n');
            }
        }
    }
    Some(out)
}

/// Run z3 on one SMT-LIB document and parse its `check-sat` verdict.
fn run_z3(smtlib: &str, timeout_ms: u64) -> Verdict {
    run_z3_with_opts(smtlib, timeout_ms, &[])
}

/// Run z3 with extra command-line `key=value` configuration params (e.g.
/// `smt.random_seed=42`). The default discharge passes none; the M5 cross-check
/// passes a genuinely different configuration.
fn run_z3_with_opts(smtlib: &str, timeout_ms: u64, opts: &[String]) -> Verdict {
    // Feed the document on stdin; bound the solver per-VC with `-T:<seconds>`.
    // z3's `-T` is wall-clock seconds (min 1) — round up from the ms budget.
    let timeout_s = timeout_ms.div_ceil(1000).max(1);
    let mut cmd = Command::new(z3_path());
    cmd.arg("-smt2").arg("-in").arg(format!("-T:{timeout_s}"));
    for opt in opts {
        cmd.arg(opt);
    }
    let mut child = match cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        // Binary absent / spawn error ⇒ Unavailable (soundness must not depend on
        // the solver being present).
        Err(_) => {
            return Verdict::Unavailable {
                reason: Z3_ABSENT_MESSAGE.to_string(),
            }
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(smtlib.as_bytes()).is_err() {
            return Verdict::Unavailable {
                reason: "failed to write SMT-LIB to z3 stdin".to_string(),
            };
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => {
            return Verdict::Unavailable {
                reason: "z3 process error while waiting for output".to_string(),
            }
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_verdict(&stdout)
}

/// Run z3 on one document and return its raw stdout (or `None` if z3 is absent /
/// errored). Used by the best-effort unsat-core extraction, which needs the lines
/// AFTER the verdict.
fn run_z3_raw(smtlib: &str, timeout_ms: u64) -> Option<String> {
    let timeout_s = timeout_ms.div_ceil(1000).max(1);
    let mut child = Command::new(z3_path())
        .arg("-smt2")
        .arg("-in")
        .arg(format!("-T:{timeout_s}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(smtlib.as_bytes()).ok()?;
    }
    let output = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// M4 EXPLAINABILITY (best-effort, never affects soundness): re-ask z3 for the
/// `(get-unsat-core)` of the already-`unsat` document, returning the NAMES of the
/// assertions it needed. Returns an empty vec on ANY failure (z3 absent, the
/// document cannot be re-named, not `unsat` on the re-run, no core produced) — an
/// absent core is a missing explanation, never a soundness signal.
fn unsat_core(full_smtlib: &str, timeout_ms: u64) -> Vec<String> {
    let Some(named) = name_assertions_for_core(full_smtlib) else {
        return Vec::new();
    };
    let Some(stdout) = run_z3_raw(&named, timeout_ms) else {
        return Vec::new();
    };
    parse_unsat_core(&stdout)
}

/// Rewrite a Lens SMT-LIB document so each `(assert X)` becomes
/// `(assert (! X :named <label>))`, with `:produce-unsat-cores` enabled, the
/// `(get-model)` dropped, and a trailing `(get-unsat-core)`. Labels carry the
/// asserted term's shape so the reported core is human-meaningful
/// (e.g. `g_amount_ge_0`, the guard it needed). Returns `None` if the document
/// cannot be parsed (the caller then omits the core, best-effort).
fn name_assertions_for_core(full_smtlib: &str) -> Option<String> {
    let forms = parse_all(full_smtlib).ok()?;
    let mut out = String::from("(set-option :produce-unsat-cores true)\n");
    let mut counter = 0usize;
    for form in &forms {
        match form {
            Sexpr::List(parts) if head_is(parts, "assert") && parts.len() == 2 => {
                let label = core_label(&parts[1], counter);
                counter += 1;
                out.push_str(&format!(
                    "(assert (! {} :named {}))\n",
                    parts[1].render(),
                    label
                ));
            }
            // Drop get-model (irrelevant to an unsat core) and any prior
            // get-unsat-core; keep everything else (logic, decls, check-sat) as-is.
            Sexpr::List(parts) if head_is(parts, "get-model") => {}
            Sexpr::List(parts) if head_is(parts, "get-unsat-core") => {}
            other => {
                out.push_str(&other.render());
                out.push('\n');
            }
        }
    }
    out.push_str("(get-unsat-core)\n");
    Some(out)
}

/// `true` if the list's head atom equals `name`.
fn head_is(parts: &[Sexpr], name: &str) -> bool {
    matches!(parts.first(), Some(Sexpr::Atom(a)) if a == name)
}

/// A human-meaningful, SMT-LIB-legal label for one asserted term. Encodes the
/// term's leading operator + operands so the reported core names *which* guard /
/// axiom / negated-VC carried the proof, with a numeric suffix for uniqueness.
fn core_label(term: &Sexpr, n: usize) -> String {
    let hint = label_hint(term);
    format!("a{n}_{hint}")
}

fn label_hint(term: &Sexpr) -> String {
    let raw = match term {
        Sexpr::Atom(a) => a.clone(),
        Sexpr::List(parts) => {
            let toks: Vec<String> = parts.iter().take(3).map(token_word).collect();
            toks.join("_")
        }
    };
    // Keep labels to an SMT-LIB-legal, compact symbol.
    let mut s: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.truncate(40);
    if s.is_empty() {
        s.push('t');
    }
    s
}

fn token_word(s: &Sexpr) -> String {
    match s {
        Sexpr::Atom(a) => op_word(a),
        Sexpr::List(_) => "expr".to_string(),
    }
}

/// Map SMT operators to readable words for labels.
fn op_word(a: &str) -> String {
    match a {
        "not" => "not".to_string(),
        "=" => "eq".to_string(),
        "distinct" => "ne".to_string(),
        ">=" => "ge".to_string(),
        "<=" => "le".to_string(),
        ">" => "gt".to_string(),
        "<" => "lt".to_string(),
        "+" => "add".to_string(),
        "-" => "sub".to_string(),
        "*" => "mul".to_string(),
        "and" => "and".to_string(),
        "or" => "or".to_string(),
        other => other.to_string(),
    }
}

/// Parse the labels out of a `(get-unsat-core)` reply. z3 prints the verdict
/// (`unsat`) then a list `(label1 label2 ...)`. Returns the labels, or an empty
/// vec if the verdict was not `unsat` or no core list followed.
fn parse_unsat_core(stdout: &str) -> Vec<String> {
    let forms = match parse_all(stdout) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    // Expect a leading `unsat` atom, then a list of labels.
    let mut saw_unsat = false;
    for form in forms {
        match form {
            Sexpr::Atom(a) if a == "unsat" => saw_unsat = true,
            Sexpr::List(items) if saw_unsat => {
                return items
                    .into_iter()
                    .filter_map(|i| match i {
                        Sexpr::Atom(a) => Some(a),
                        Sexpr::List(_) => None,
                    })
                    .collect();
            }
            _ => {}
        }
    }
    Vec::new()
}

/// Parse z3's stdout. The FIRST non-empty line is the `check-sat` verdict; any
/// remaining text is the `get-model` body (captured for a `sat`/REFUTED witness).
fn parse_verdict(stdout: &str) -> Verdict {
    let mut lines = stdout.lines();
    let verdict = lines
        .by_ref()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    match verdict {
        "unsat" => Verdict::Unsat,
        "sat" => {
            let model: String = lines.collect::<Vec<_>>().join("\n").trim().to_string();
            Verdict::Sat { model }
        }
        // `unknown`, timeout, error lines, garbage ⇒ indecisive.
        other => Verdict::Indecisive {
            reason: format!("z3 did not return a decisive verdict (first token: {other:?})"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z3_here() -> bool {
        crate::z3_available()
    }

    #[test]
    fn reorder_assertions_reverses_only_the_asserts() {
        // The cross-check feeds z3 the same constraints in a DIFFERENT order. The
        // reorder must reverse the assert forms while leaving logic/decls/check-sat
        // in place, and must remain semantically equivalent (same symbols).
        let doc = "(set-logic QF_LIA)\n(declare-const x Int)\n(assert (>= x 0))\n\
                   (assert (<= x 5))\n(check-sat)\n(get-model)\n";
        let out = reorder_assertions(doc).expect("parseable");
        let i_lo = out.find("(>= x 0)").expect("lo present");
        let i_hi = out.find("(<= x 5)").expect("hi present");
        assert!(i_hi < i_lo, "asserts must be reversed; got:\n{out}");
        assert!(out.contains("(set-logic QF_LIA)"));
        assert!(out.contains("(declare-const x Int)"));
        assert!(out.contains("(check-sat)"));
    }

    #[test]
    fn crosscheck_confirms_a_genuinely_unsat_query() {
        // Two agreeing runs (same binary, perturbed config — NOT independent
        // solvers): a genuinely unsat document must be CONFIRMED by the
        // differently-configured cross-check run.
        if !z3_here() {
            eprintln!("SKIP crosscheck_confirms_a_genuinely_unsat_query: z3 absent");
            return;
        }
        let unsat = "(set-logic QF_LIA)\n(declare-const x Int)\n(assert (>= x 1))\n\
                     (assert (<= x 0))\n(check-sat)\n(get-model)\n";
        assert!(matches!(
            crosscheck_unsat(unsat, 10_000),
            CrossCheck::Confirmed
        ));
    }

    #[test]
    fn crosscheck_disagrees_when_the_independent_run_finds_sat() {
        // The downgrade trigger: if the independent run does NOT confirm unsat (here
        // it finds the query satisfiable), the cross-check DISAGREES, and `discharge`
        // maps that to UNKNOWN — never trusting a single run's unsat.
        if !z3_here() {
            eprintln!("SKIP crosscheck_disagrees_when_the_independent_run_finds_sat: z3 absent");
            return;
        }
        let sat = "(set-logic QF_LIA)\n(declare-const x Int)\n(assert (>= x 0))\n\
                   (check-sat)\n(get-model)\n";
        match crosscheck_unsat(sat, 10_000) {
            CrossCheck::Disagreed { .. } => {}
            other => panic!("a satisfiable cross-check run must DISAGREE, got {other:?}"),
        }
    }

    #[test]
    fn discharge_proves_only_when_the_crosscheck_also_confirms_unsat() {
        // The cross-check sits ON the PROVED path: a reachable transition (T alone
        // sat) whose negated VC is unsat on the primary run is reported PROVED ONLY
        // after the independent, differently-configured run ALSO confirms unsat. The
        // downgrade contract — a non-confirming cross-check ⇒ UNKNOWN — is pinned
        // deterministically by `crosscheck_disagrees_when_the_independent_run_finds_sat`
        // (a genuine z3-vs-z3 disagreement on a decidable QF_LIA query is not
        // reproducible, so the downgrade is unit-tested at the `crosscheck_unsat`
        // boundary rather than forced through a real solver split here).
        if !z3_here() {
            eprintln!(
                "SKIP discharge_proves_only_when_the_crosscheck_also_confirms_unsat: z3 absent"
            );
            return;
        }
        // T alone (reachable): x >= 0 is satisfiable.
        let t_only = "(set-logic QF_LIA)\n(declare-const x Int)\n(assert (>= x 0))\n\
                      (check-sat)\n(get-model)\n";
        // Full negated VC: x >= 0 AND x < 0 is unsat (the negated VC contradicts T).
        let full = "(set-logic QF_LIA)\n(declare-const x Int)\n(assert (>= x 0))\n\
                    (assert (< x 0))\n(check-sat)\n(get-model)\n";
        assert!(matches!(
            discharge(t_only, full, 10_000),
            Outcome::Proved { .. }
        ));
    }

    #[test]
    fn unsat_parses_to_unsat() {
        // `unsat` is necessary-but-not-sufficient for PROVED: the negate-and-check
        // protocol only certifies PROVED after the vacuity probe also passes (see
        // `discharge`). At the parse layer it is simply `Unsat`.
        assert!(matches!(parse_verdict("unsat\n"), Verdict::Unsat));
    }

    #[test]
    fn sat_with_model_carries_the_witness() {
        let out = "sat\n(\n  (define-fun x () Int 1)\n)";
        match parse_verdict(out) {
            Verdict::Sat { model } => assert!(model.contains("x")),
            other => panic!("expected Sat with model, got {other:?}"),
        }
    }

    #[test]
    fn sat_without_model_is_sat_with_empty_body() {
        // The empty-model→UNKNOWN soundness guard lives in `discharge`, not the
        // parser: the parser reports `Sat` with an empty body.
        match parse_verdict("sat\n") {
            Verdict::Sat { model } => assert!(model.is_empty()),
            other => panic!("expected Sat with empty model, got {other:?}"),
        }
    }

    #[test]
    fn unknown_token_is_indecisive_never_unsat() {
        // SOUNDNESS: nothing but the literal `unsat` parses to `Unsat` (the only
        // gateway to PROVED). `unknown`, empty output, and error lines are all
        // indecisive.
        assert!(matches!(
            parse_verdict("unknown\n"),
            Verdict::Indecisive { .. }
        ));
        assert!(matches!(parse_verdict(""), Verdict::Indecisive { .. }));
        assert!(matches!(
            parse_verdict("(error \"line 1\")\n"),
            Verdict::Indecisive { .. }
        ));
    }
}
