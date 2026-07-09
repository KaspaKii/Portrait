//! Proof-carrying Hallmark certification for Portrait components
//! (Innovation §4.1).
//!
//! This crate runs the **real** checks that already exist in the Portrait
//! pipeline against a `.portrait` source file and records, for each one, the
//! exact check that was performed and its outcome. The result is a
//! `hallmark.json` document whose every claim maps to an actual check — so a
//! third party can re-run the same checker on the same source and re-derive the
//! Hallmark.
//!
//! ## Honesty contract
//!
//! - A claim is only emitted for a check that is **actually executed** here.
//! - A claim's `result` is `pass` only when the underlying check returned
//!   success; a rejection (e.g. a sema diagnostic, a `silverc` non-zero exit) is
//!   recorded as `fail`, never as a fabricated pass.
//! - A claim that could not be run (e.g. `silverc` binary absent) is recorded as
//!   `error` with the reason, and is **not** marked re-derivable.
//! - `rederivable` is `true` only when a third party can reproduce the result
//!   from the source alone (the four pipeline checks are pure functions of the
//!   source; the `silverc-accepts` claim additionally requires the pinned
//!   `silverc` binary, which the `rederive` note names).
//!
//! No SMT proof, formal-verification, or audit claim is made — none is performed.
//! Every Hallmark carries the maturity stamp: **pre-production, unaudited,
//! testnet-only.**

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::path::Path;
use std::process::Command;

/// Maturity stamp carried by every Hallmark. Mirrors the Foundation's
/// perishable-evidence honesty rule.
pub const MATURITY: &str = "pre-production, unaudited, testnet-only";

/// The `silverc` binary the `silverc-accepts` claim invokes. A third party
/// reproduces the Hallmark with this exact compiler.
pub const SILVERC_BIN: &str = "silverc";

/// Outcome of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The check ran and the property holds.
    Pass,
    /// The check ran and the property does **not** hold (e.g. a sema rejection).
    Fail,
    /// The check could not be run (e.g. a tool was unavailable). Not a pass.
    Error,
}

impl Outcome {
    /// The lowercase string used in the JSON document.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Fail => "fail",
            Outcome::Error => "error",
        }
    }
}

/// A single re-derivable claim in a Hallmark.
#[derive(Debug, Clone)]
pub struct Claim {
    /// Human-readable claim text — asserts *only* what the check verifies.
    pub claim: String,
    /// Stable identifier for the check (e.g. `"parse"`, `"sema"`).
    pub check: String,
    /// The function or command that was invoked to perform the check.
    pub command: String,
    /// The outcome of running the check.
    pub result: Outcome,
    /// Detail: the rejection message on `Fail`, the reason on `Error`, or a
    /// short confirming note on `Pass`.
    pub detail: String,
    /// `true` only if a third party can reproduce this exact result from the
    /// source alone (plus, for `silverc-accepts`, the pinned `silverc` binary).
    pub rederivable: bool,
}

/// A complete Hallmark for one Portrait component.
#[derive(Debug, Clone)]
pub struct Hallmark {
    /// The component name (the source file stem).
    pub component: String,
    /// The source path the checks were run against.
    pub source: String,
    /// Maturity stamp; always [`MATURITY`].
    pub maturity: String,
    /// The exact command a third party runs to reproduce this Hallmark.
    pub rederive: String,
    /// Every claim, in pipeline order.
    pub claims: Vec<Claim>,
}

impl Hallmark {
    /// `true` when every claim that ran passed (no `Fail`, no `Error`).
    pub fn all_pass(&self) -> bool {
        self.claims.iter().all(|c| c.result == Outcome::Pass)
    }

    /// Serialize to a stable, pretty-printed JSON document.
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        s.push_str("{\n");
        s.push_str(&format!(
            "  \"component\": {},\n",
            json_str(&self.component)
        ));
        s.push_str(&format!("  \"source\": {},\n", json_str(&self.source)));
        s.push_str(&format!("  \"maturity\": {},\n", json_str(&self.maturity)));
        s.push_str(&format!("  \"rederive\": {},\n", json_str(&self.rederive)));
        s.push_str("  \"claims\": [\n");
        for (i, c) in self.claims.iter().enumerate() {
            s.push_str("    {\n");
            s.push_str(&format!("      \"claim\": {},\n", json_str(&c.claim)));
            s.push_str(&format!("      \"check\": {},\n", json_str(&c.check)));
            s.push_str(&format!("      \"command\": {},\n", json_str(&c.command)));
            s.push_str(&format!(
                "      \"result\": {},\n",
                json_str(c.result.as_str())
            ));
            s.push_str(&format!("      \"detail\": {},\n", json_str(&c.detail)));
            s.push_str(&format!("      \"rederivable\": {}\n", c.rederivable));
            s.push_str(if i + 1 == self.claims.len() {
                "    }\n"
            } else {
                "    },\n"
            });
        }
        s.push_str("  ]\n");
        s.push_str("}\n");
        s
    }
}

/// Minimal JSON string escaping (no serde dependency, matching the workspace's
/// dependency-light style).
fn json_str(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Run every available check against `path` and produce a [`Hallmark`].
///
/// The pipeline is short-circuited the way the real compiler is: a stage that
/// depends on an earlier stage succeeding is recorded as `Error` ("not reached")
/// if that earlier stage failed — never as a fabricated pass. This mirrors what
/// a third party re-running the checker would observe.
pub fn hallmark(path: &Path) -> Result<Hallmark, String> {
    let component = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "component".to_string());
    let source = path.display().to_string();

    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", source, e))?;

    let mut claims: Vec<Claim> = Vec::new();

    // ── Claim 1: parse ───────────────────────────────────────────────────────
    let parsed = portrait_syntax::parse(&src);
    let program = match &parsed {
        Ok(p) => {
            claims.push(Claim {
                claim: "Source parses as a well-formed Portrait program.".into(),
                check: "parse".into(),
                command: "portrait_syntax::parse(&source)".into(),
                result: Outcome::Pass,
                detail: "parser returned Ok".into(),
                rederivable: true,
            });
            Some(p)
        }
        Err(e) => {
            claims.push(Claim {
                claim: "Source parses as a well-formed Portrait program.".into(),
                check: "parse".into(),
                command: "portrait_syntax::parse(&source)".into(),
                result: Outcome::Fail,
                detail: e.clone(),
                rederivable: true,
            });
            None
        }
    };

    // ── Claim 2: sema (real structural checks) ───────────────────────────────
    let sema_ok = match program {
        None => {
            claims.push(not_reached(
                "Program satisfies portrait-sema's structural static checks (lifecycle \
                 reachability, flow integrity, transition/return consistency, \
                 value_conserved, no_undeclared_state).",
                "sema",
                "portrait_sema::check(&program)",
                "parse failed; sema not reached",
            ));
            false
        }
        Some(p) => match portrait_sema::check(p) {
            Ok(()) => {
                claims.push(Claim {
                    claim: "Program satisfies portrait-sema's structural static checks (lifecycle \
                            reachability, flow integrity, transition/return consistency, \
                            value_conserved, no_undeclared_state)."
                        .into(),
                    check: "sema".into(),
                    command: "portrait_sema::check(&program)".into(),
                    result: Outcome::Pass,
                    detail: "all structural checks passed".into(),
                    rederivable: true,
                });
                true
            }
            Err(ds) => {
                let msg = ds
                    .into_iter()
                    .map(|d| d.message)
                    .collect::<Vec<_>>()
                    .join("; ");
                claims.push(Claim {
                    claim: "Program satisfies portrait-sema's structural static checks (lifecycle \
                            reachability, flow integrity, transition/return consistency, \
                            value_conserved, no_undeclared_state)."
                        .into(),
                    check: "sema".into(),
                    command: "portrait_sema::check(&program)".into(),
                    result: Outcome::Fail,
                    detail: msg,
                    rederivable: true,
                });
                false
            }
        },
    };

    // ── Claim 3: emit (lower → project → emit) ───────────────────────────────
    // Only attempted when parse + sema succeeded, mirroring the real pipeline.
    // Records the covenant COUNT and names so multi-role apps (e.g. DigitalReit)
    // show ALL emitted covenants in a single emit claim.
    let emitted: Option<Vec<(portrait_ir::CovenantModel, portrait_ir::SilFile)>> = if let (
        Some(p),
        true,
    ) =
        (program, sema_ok)
    {
        let cartoon = portrait_ir::lower(p);
        let models = portrait_project::project(&cartoon);
        match portrait_emit::emit(&models) {
            Err(detail) => {
                // Emit fail-loud (e.g. an unlowerable guard that would have
                // been silently dropped). Surface it as a failed emit claim.
                claims.push(Claim {
                        claim: "Program lowers and projects to at least one silverscript \
                                covenant source (portrait-ir → portrait-project → portrait-emit)."
                            .into(),
                        check: "emit".into(),
                        command:
                            "portrait_emit::emit(&portrait_project::project(&portrait_ir::lower(&program)))"
                                .into(),
                        result: Outcome::Fail,
                        detail,
                        rederivable: true,
                    });
                None
            }
            Ok(sil_files) => {
                let pairs: Vec<(portrait_ir::CovenantModel, portrait_ir::SilFile)> =
                    models.into_iter().zip(sil_files).collect();
                if pairs.is_empty() {
                    claims.push(Claim {
                claim: "Program lowers and projects to at least one silverscript \
                            covenant source (portrait-ir → portrait-project → portrait-emit)."
                    .into(),
                check: "emit".into(),
                command:
                    "portrait_emit::emit(&portrait_project::project(&portrait_ir::lower(&program)))"
                        .into(),
                result: Outcome::Fail,
                detail: "pipeline produced no covenant model".into(),
                rederivable: true,
            });
                    None
                } else {
                    let names: Vec<&str> = pairs.iter().map(|(m, _)| m.name.as_str()).collect();
                    claims.push(Claim {
                claim: "Program lowers and projects to at least one silverscript \
                            covenant source (portrait-ir → portrait-project → portrait-emit)."
                    .into(),
                check: "emit".into(),
                command:
                    "portrait_emit::emit(&portrait_project::project(&portrait_ir::lower(&program)))"
                        .into(),
                result: Outcome::Pass,
                detail: format!("emitted {} covenant(s): {}", names.len(), names.join(", ")),
                rederivable: true,
            });
                    Some(pairs)
                }
            }
        }
    } else {
        claims.push(not_reached(
            "Program lowers and projects to at least one silverscript covenant source \
                 (portrait-ir → portrait-project → portrait-emit).",
            "emit",
            "portrait_emit::emit(&portrait_project::project(&portrait_ir::lower(&program)))",
            "parse or sema failed; emit not reached",
        ));
        None
    };

    // ── Claim 4: silverc-accepts[<CovenantName>] — one per emitted covenant ──
    // Each covenant is compiled independently; a failure in one does NOT suppress
    // the others. Writing is done to a temp dir, never beside the source (M2).
    match emitted {
        Some(pairs) => {
            for (model, sil) in &pairs {
                claims.push(silverc_claim(&model.name, model, sil));
            }
        }
        None => claims.push(not_reached(
            "The pinned silverc compiler accepts the emitted .sil with a generated \
             constructor (exit 0).",
            "silverc-accepts[<covenant>]",
            "silverc --constructor-args <ctor> -c <emitted.sil>",
            "emit not reached; silverc not invoked",
        )),
    }

    let rederive = format!(
        "Clone the portrait workspace, then run \
         `cargo run -p portrait-cli -- verify {src}` (or `portrait verify {src}`). \
         This re-runs portrait_syntax::parse, portrait_sema::check, the \
         lower→project→emit pipeline, and invokes `{silverc} --constructor-args <ctor> -c <emitted.sil>`, \
         re-deriving every claim above. Do not trust this stamp — reproduce it.",
        src = source,
        silverc = SILVERC_BIN,
    );

    Ok(Hallmark {
        component,
        source,
        maturity: MATURITY.to_string(),
        rederive,
        claims,
    })
}

/// Build a `silverc-accepts[<name>]` claim by writing the emitted `.sil` + a
/// generated ctor to a **temporary directory** (never beside the source) and
/// invoking `silverc`. A missing binary is recorded as `Error` (not
/// re-derivable here), never as a fabricated pass.
fn silverc_claim(
    covenant_name: &str,
    model: &portrait_ir::CovenantModel,
    sil: &portrait_ir::SilFile,
) -> Claim {
    let claim_text =
        "The pinned silverc compiler accepts the emitted .sil with a generated constructor (exit 0).";
    let check = format!("silverc-accepts[{}]", covenant_name);
    let command = format!("{} --constructor-args <ctor> -c <emitted.sil>", SILVERC_BIN);

    // Write artifacts to a temp subdir — never next to the certified source (M2).
    // The dir is unique per invocation (pid + a process-wide atomic counter) so
    // concurrent hallmark calls that emit the same covenant name (e.g. two tests
    // certifying a "Counter") never share `<name>.sil`/`<name>_ctor.json` and race
    // each other's silverc compilation.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "portrait-verify-silverc-{}-{}-{}",
        covenant_name,
        std::process::id(),
        nonce
    ));
    if let Err(e) = std::fs::create_dir_all(&tmp) {
        return Claim {
            claim: claim_text.into(),
            check,
            command,
            result: Outcome::Error,
            detail: format!("could not create temp dir {}: {}", tmp.display(), e),
            rederivable: false,
        };
    }

    let sil_path = tmp.join(&sil.name);
    if let Err(e) = std::fs::write(&sil_path, &sil.source) {
        return Claim {
            claim: claim_text.into(),
            check,
            command,
            result: Outcome::Error,
            detail: format!("could not write {}: {}", sil_path.display(), e),
            rederivable: false,
        };
    }
    let (ctor_name, ctor_json) = portrait_emit::emit_ctor(model);
    let ctor_path = tmp.join(&ctor_name);
    if let Err(e) = std::fs::write(&ctor_path, &ctor_json) {
        return Claim {
            claim: claim_text.into(),
            check,
            command,
            result: Outcome::Error,
            detail: format!("could not write {}: {}", ctor_path.display(), e),
            rederivable: false,
        };
    }

    let outcome = Command::new(SILVERC_BIN)
        .arg("--constructor-args")
        .arg(&ctor_path)
        .arg("-c")
        .arg(&sil_path)
        .output();

    // Best-effort cleanup of the temp dir.
    let _ = std::fs::remove_dir_all(&tmp);

    match outcome {
        Ok(output) if output.status.success() => Claim {
            claim: claim_text.into(),
            check,
            command,
            result: Outcome::Pass,
            detail: "silverc exited 0".into(),
            rederivable: true,
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Claim {
                claim: claim_text.into(),
                check,
                command,
                result: Outcome::Fail,
                detail: format!("silverc exited {}: {}", output.status, stderr.trim()),
                rederivable: true,
            }
        }
        Err(e) => Claim {
            claim: claim_text.into(),
            check,
            command,
            // Tool unavailable: cannot assert acceptance. Not a pass; not
            // re-derivable on this machine.
            result: Outcome::Error,
            detail: format!("could not invoke {}: {}", SILVERC_BIN, e),
            rederivable: false,
        },
    }
}

/// A claim whose check was not reached because an earlier stage failed. Recorded
/// as `Error` — honest about the fact it never ran.
fn not_reached(claim: &str, check: &str, command: &str, why: &str) -> Claim {
    Claim {
        claim: claim.into(),
        check: check.into(),
        command: command.into(),
        result: Outcome::Error,
        detail: why.into(),
        rederivable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write `src` to a temp `.portrait` file and return its path.
    fn temp_portrait(name: &str, src: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "portrait-verify-test-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.portrait", name));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(src.as_bytes()).unwrap();
        path
    }

    const COUNTER: &str = r#"pragma portrait ^0.1.0;
app Counter {
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

    #[test]
    fn passing_program_yields_all_pass_hallmark() {
        // A well-formed program: parse, sema, and emit must all pass. The
        // silverc-accepts claim passes when the pinned binary is present; if it
        // is absent we get Error (never a fabricated pass), so we assert the
        // three pure-pipeline claims here and check silverc separately below.
        let path = temp_portrait("counter_pass", COUNTER);
        let hm = hallmark(&path).expect("hallmark");
        assert_eq!(hm.maturity, MATURITY);

        let parse = &hm.claims[0];
        let sema = &hm.claims[1];
        let emit = &hm.claims[2];
        assert_eq!(parse.result, Outcome::Pass, "parse: {}", parse.detail);
        assert_eq!(sema.result, Outcome::Pass, "sema: {}", sema.detail);
        assert_eq!(emit.result, Outcome::Pass, "emit: {}", emit.detail);
        assert!(parse.rederivable && sema.rederivable && emit.rederivable);

        // The silverc claim must never be silently fabricated: it is Pass
        // (binary present, exit 0) or Error (binary absent) — but not Fail for a
        // program the engraver is known to compile.
        let silverc = &hm.claims[3];
        assert!(
            silverc.result == Outcome::Pass || silverc.result == Outcome::Error,
            "silverc claim should be pass or error, got {} ({})",
            silverc.result.as_str(),
            silverc.detail
        );
        // Re-derivable flag is honest: true iff it actually ran to a verdict.
        assert_eq!(silverc.rederivable, silverc.result == Outcome::Pass);
    }

    #[test]
    fn sema_rejected_program_yields_failed_claim_not_fabricated_pass() {
        // `live -> end via counter.bump` references a lifecycle target `end`
        // that originates no edge and is not a declared terminal — this trips
        // portrait-sema's structural checks. The hallmark must record a FAILED
        // sema claim, and must NOT fabricate a pass for the downstream stages.
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
        let path = temp_portrait("counter_bad_sema", bad);
        let hm = hallmark(&path).expect("hallmark");

        // Parse succeeds (syntactically valid), sema fails (ghost entrypoint).
        let sema = hm.claims.iter().find(|c| c.check == "sema").unwrap();
        assert_eq!(
            sema.result,
            Outcome::Fail,
            "sema should reject unknown entrypoint, detail: {}",
            sema.detail
        );
        assert!(
            !sema.detail.is_empty(),
            "fail must carry the real diagnostic"
        );

        // Downstream stages must NOT be fabricated as pass; they are Error
        // ("not reached"), since the real pipeline short-circuits on sema.
        let emit = hm.claims.iter().find(|c| c.check == "emit").unwrap();
        let silverc = hm
            .claims
            .iter()
            .find(|c| c.check.starts_with("silverc-accepts"))
            .unwrap();
        assert_ne!(emit.result, Outcome::Pass, "emit must not fabricate a pass");
        assert_ne!(
            silverc.result,
            Outcome::Pass,
            "silverc must not fabricate a pass"
        );
        assert!(!hm.all_pass());
    }

    #[test]
    fn json_is_well_formed_and_carries_maturity_and_rederive() {
        let path = temp_portrait("counter_json", COUNTER);
        let hm = hallmark(&path).expect("hallmark");
        let json = hm.to_json();
        assert!(json.starts_with('{') && json.trim_end().ends_with('}'));
        assert!(json.contains("\"maturity\""));
        assert!(json.contains(MATURITY));
        assert!(json.contains("\"rederive\""));
        assert!(json.contains("reproduce it"));
        // The three pipeline checks appear exactly once each.
        for check in ["parse", "sema", "emit"] {
            let needle = format!("\"check\": \"{}\"", check);
            assert_eq!(
                json.matches(&needle).count(),
                1,
                "expected exactly one {} claim",
                check
            );
        }
        // The Counter app has one role → one covenant → one silverc-accepts[Counter] claim.
        assert!(
            json.contains("\"check\": \"silverc-accepts[Counter]\""),
            "expected silverc-accepts[Counter] claim in: {}",
            json
        );
        // No claim asserts SMT/audit — those are never performed here.
        assert!(!json.to_lowercase().contains("smt"));
        assert!(!json.to_lowercase().contains("audited\""));
    }
}
