//! # portrait-lens — an SMT proof engine for the Portrait covenant *model*
//!
//! Lens slots **behind** a passing `portrait-sema` as an opt-in `prove` stage.
//! For each covenant transition entrypoint it builds a transition relation
//! `T(s, p, s') ≡ G(s, p) ∧ s' = ⟦return⟧` from the surface AST, generates the
//! **total value-conservation** verification condition (VC) over the
//! value-bearing state fields, emits SMT-LIB, and (when a `z3` binary is
//! available) discharges it by the negate-and-check protocol.
//!
//! ## What a `PROVED` means — and what it does NOT
//!
//! Lens proves a property of the **Portrait MODEL** — the AST-derived transition
//! relation — **NOT** of the emitted `.sil` silverscript the chain enforces, and
//! NOT of the deployed covenant. A sound `PROVED` is sound only under the
//! assumptions **A1–A4** below. Translation validation between the model and the
//! emitted script is explicitly future work.
//!
//! ## Soundness assumptions A1–A4 (the reviewer must accept these)
//!
//! These are the soundness contract for every `PROVED`. They are inlined here so
//! the contract is self-contained; the full encoding spec (with the worked
//! soundness-transfer argument) lives as `docs/LENS-M0-ENCODING-SPEC.md` §5.4 in
//! the separate **kaspa-compliance-patterns** repository.
//!
//! - **A1 — faithful concrete semantics.** The concrete semantics Lens encodes is
//!   the intended Portrait covenant model semantics. Any divergence voids the
//!   soundness transfer.
//! - **A2 — exact integer/boolean encoding.** The encoding table is faithful for
//!   the exact nodes (integers, booleans): SMT `+`/`<`/… mean what Portrait
//!   `+`/`<`/… mean over the same domain. (Immediate for mathematical-integer
//!   mode; bounded mode additionally requires the chosen width to match the
//!   engraver's.)
//! - **A3 — trusted solver.** A reported `unsat` is a true `unsat`. (Proof-
//!   certificate checking is future work that would discharge A3.)
//! - **A4 — passing sema.** Lens runs **behind** a passing `portrait-sema` (no
//!   `Raw` holes, all bodies typed, capability/threshold already checked
//!   syntactically). Lens depends on A4; it does not re-establish it.
//!
//! If any of A1–A4 fails, the `PROVED` soundness claim is voided.
//!
//! **Pre-production, unaudited, testnet-only.**
//!
//! ## Soundness discipline (paramount)
//!
//! Only the literal solver verdict `unsat` for the negated VC maps to
//! [`Outcome::Proved`]. **Every** other path — `unknown`, timeout, a `z3` binary
//! that is absent or errors, a non-verdict exit, a parse failure of the solver
//! output, an empty/vacuous value-bearing set, or a self-contradictory
//! transition relation — falls to [`Outcome::Unknown`], **never** to a false
//! `PROVED`. [`Outcome::Unknown`] is a first-class outcome, not a failure.
//!
//! ## Honest scope (M1 + M2 + M3 + M5)
//!
//! FIVE VC classes, each grounded in the **real** surface AST and each routed
//! through the *same* SAT(T) vacuity-safe [`discharge`] (only a satisfiable `T`
//! plus an unsat negated-VC yields `Proved`):
//!
//! - **Value conservation** (§4a) — the M1/M2 headline, superseding the
//!   structural C1 + D4 checks.
//! - **Spend / no-value-created** (M5, formerly Q3-deferred) — a clean value-out
//!   spend introduces a fresh `spent_out` var bound to the drop `Σf − Σf'` and
//!   proves `spent_out >= 0` (the model spend creates NO value; the state total
//!   does not increase). HONEST SCOPE: it proves the MODEL mints nothing on a
//!   spend; it does NOT bind `spent_out` to the actual on-chain output amount
//!   (the model does not read UTXO coin values) — that stays translation
//!   validation. Replaces the prior [`LensError::Unsupported`] spend stub.
//! - **Range / overflow** (§4c) — every value-bearing-field arithmetic result
//!   fits the on-chain `u64` sompi width `[0, 2^64)`. Turns the prior honest-scope
//!   bounded-int gap (`a*2` PROVED over ℤ but wraps on-chain) into a CHECKED
//!   obligation.
//! - **Refinement** (§4b) — a **declared** named refinement invariant
//!   (`non_negative_amount`, `spending_cap`) as a real implication `G ⟹ φ`.
//! - **Invariant preservation** (§4d) — a **declared** stateful invariant
//!   (`bounded_supply`: `supply <= total`; `monotonic_seq`: `seq' = seq + 1`)
//!   preserved across the transition, `I(s) ∧ G ⟹ I(s')`.
//!
//! **Not generated** (a VC that is never asserted is never reported `PROVED`): a
//! *user-written arbitrary arithmetic invariant* — the surface AST has **no
//! syntax** for one
//! ([`portrait_syntax::Invariant`] is a named tag, never an author-supplied
//! `Expr`), so §4(d) is grounded only in the named stateful invariants above; and
//! the `temporal_guard` invariant, whose meaning is a structural time/capability
//! gate rather than a closed arithmetic obligation.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod encode;
mod more_vc;
mod sexpr;
mod smt;
mod solve;
mod translation;
mod validate;
mod vc;

pub use translation::{validate_translation, Correspondence, TRANSLATION_STRUCTURAL_FOOTER};
pub use validate::WitnessConfidence;
pub use vc::{value_bearing_fields, VcKind, U64_EXCL_MAX};

/// The on-its-face caveat carried by every Lens report and CLI footer. States
/// the model-vs-`.sil` boundary and the maturity stamp.
pub const MODEL_NOT_SIL_CAVEAT: &str =
    "Proven over the Portrait MODEL under assumptions A1-A4; NOT a proof of the \
     emitted .sil script or the deployed covenant. pre-production, unaudited, \
     testnet-only.";

/// The skip-with-message reported when no usable `z3` binary is found. Reporting
/// `UNKNOWN` (never `PROVED`) when the solver is absent is a soundness invariant:
/// soundness must not depend on the solver being present.
pub const Z3_ABSENT_MESSAGE: &str =
    "z3 not found on PATH or $PORTRAIT_Z3; install z3 to discharge VCs — reporting \
     UNKNOWN, never PROVED";

/// A solver verdict for a single VC.
///
/// SOUNDNESS: only [`Outcome::Proved`] is reported, and only when the solver
/// returns the exact literal `unsat` for the negated VC. Anything else is
/// [`Outcome::Unknown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The negated VC is `unsat`: no model-level transition satisfies the guards
    /// yet violates the VC. PROVED **for the model**, under A1-A4.
    ///
    /// `unsat_core` is the M4 explainability annotation: the named assertions z3
    /// reported as the minimal set needed for the `unsat` (the guards / domain
    /// axioms / negated VC that carried the proof). EXPLAINABILITY ONLY — it does
    /// NOT change what PROVED means or its soundness; it is best-effort and empty
    /// when z3 cannot produce it.
    Proved {
        /// Names of the assertions z3 reported in `(get-unsat-core)` (empty if
        /// unavailable). Best-effort; never affects the soundness of PROVED.
        unsat_core: Vec<String>,
    },
    /// The negated VC is `sat`: a concrete counter-example fires the guards and
    /// breaks the VC. `model` is the solver's `get-model` text (the witness).
    ///
    /// `confidence` is the M4 independent-replay verdict (see [`WitnessConfidence`]):
    /// `Confirmed` when the model's integer/boolean assignment was replayed in Rust
    /// and the VC is genuinely violated over interpreted values; `Candidate` when
    /// the witness relies on an uninterpreted-function value (a possible
    /// over-approximation artifact), flagged rather than silently trusted.
    Refuted {
        /// The solver's counter-model text (non-empty for a real witness).
        model: String,
        /// Independent-replay trust level for this counter-example (M4).
        confidence: WitnessConfidence,
    },
    /// Outside Lens's decidable fragment, the solver could not decide, or the
    /// solver was unavailable. First-class — never silently upgraded to `Proved`.
    Unknown {
        /// Human-readable reason this VC fell to UNKNOWN.
        reason: String,
    },
}

/// A hard refusal to emit any encoding. Distinct from [`Outcome::Unknown`]: a
/// refusal means *no SMT was produced*, never a verdict. Lens fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensError {
    /// `portrait-sema` did not pass; Lens refuses to prove an unchecked covenant
    /// (assumption A4).
    SemaNotPassed,
    /// A `Stmt::Raw` (untyped hole) reached a covenant entrypoint body. There is
    /// no sound encoding of an untyped hole (spec §3.2); Lens refuses to run.
    RawStmtInCovenant,
    /// The transition is outside the supported VC scope (e.g. a non-foldable
    /// nonlinear term in the conservation sum, or a lone mint-like increase).
    /// Carries a reason. NOT a verdict.
    Unsupported(String),
}

impl std::fmt::Display for LensError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LensError::SemaNotPassed => {
                write!(
                    f,
                    "portrait-sema did not pass; Lens runs only behind a passing sema (A4)"
                )
            }
            LensError::RawStmtInCovenant => write!(
                f,
                "a raw (untyped) statement reached a covenant entrypoint; no sound encoding exists"
            ),
            LensError::Unsupported(why) => write!(f, "unsupported by Lens M1/M2: {why}"),
        }
    }
}

impl std::error::Error for LensError {}

/// A single verification-condition report for one entrypoint.
#[derive(Debug, Clone)]
pub struct VcReport {
    /// The qualified entrypoint name, `role.entry`.
    pub entrypoint: String,
    /// Which VC class this report covers (M1/M2: only value conservation).
    pub vc_kind: VcKind,
    /// The SMT-LIB document emitted for this VC (filled by M1): the transition
    /// relation `T` **and** the negated VC `(assert (not VC))`.
    pub smtlib: String,
    /// The SMT-LIB document for the transition relation `T` **alone** — the same
    /// declarations + guards + next-state binding + domain axioms, but **without**
    /// the negated VC. Discharged as a vacuity probe: if `T` alone is `unsat` the
    /// entrypoint is unreachable (self-contradictory guards) and the conservation
    /// query is vacuously `unsat`, so the result must fall to [`Outcome::Unknown`],
    /// **never** [`Outcome::Proved`] (soundness — see [`discharge`]).
    pub transition_smtlib: String,
    /// The solver verdict (filled by M2 [`discharge`]; `None` until then).
    pub outcome: Option<Outcome>,
}

/// Build the total-value-conservation VC for a single transition entrypoint.
///
/// M1: fills [`VcReport::smtlib`] with the complete SMT-LIB document
/// (declarations, transition relation `T`, the negated VC `(assert (not VC))`,
/// `check-sat`, `get-model`); leaves [`VcReport::outcome`] as `None`.
///
/// For a clean value-out spend this returns a [`VcKind::Spend`] report (M5), not
/// a refusal. Returns [`LensError`] (a hard refusal, not a verdict) when there is
/// no sound encoding to emit: a `Raw` statement in the body, an empty
/// value-bearing set (the conservation VC would collapse to the vacuous `0 = 0`),
/// a mint/burn entrypoint (exempt — no VC), a clean lone-increase mint-like move,
/// or a non-foldable nonlinear term.
pub fn build_conservation_vc(
    role: &portrait_syntax::Role,
    entry: &portrait_syntax::Entry,
) -> Result<VcReport, LensError> {
    vc::build_conservation_vc(role, entry)
}

/// Build **every** VC class for **every** transition entrypoint of every role:
/// value conservation (§4a), range/overflow (§4c), declared-refinement (§4b), and
/// declared-invariant-preservation (§4d). Each report is a distinct per-class
/// verdict line, and all four route through the *same* SAT(T) vacuity-safe
/// [`discharge`].
///
/// Refuses (returns [`LensError::SemaNotPassed`]) unless `portrait-sema::check`
/// passes for `program` (assumption A4). Entrypoints that legitimately have no VC
/// of a given class (mint/burn exemption, no value-bearing fields, an undeclared
/// refinement, no arithmetic to range-check) are silently skipped for that class —
/// they are not failures. A clean value-out spend now yields a [`VcKind::Spend`]
/// VC (M5) rather than being skipped.
pub fn prove_program(program: &portrait_syntax::Program) -> Result<Vec<VcReport>, LensError> {
    if portrait_sema::check(program).is_err() {
        return Err(LensError::SemaNotPassed);
    }
    let declared = more_vc::DeclaredInvariants::from_app(&program.app.invariants);
    let mut reports = Vec::new();
    for role in &program.app.roles {
        for entry in &role.entrypoints {
            if !matches!(entry.mode, portrait_syntax::CovenantMode::Transition) {
                continue;
            }
            // (a) Value conservation.
            match vc::build_conservation_vc(role, entry) {
                Ok(report) => reports.push(report),
                // A Raw hole in a covenant transition is a hard, program-wide
                // refusal (fail closed): there is no sound encoding.
                Err(LensError::RawStmtInCovenant) => return Err(LensError::RawStmtInCovenant),
                Err(LensError::SemaNotPassed) => return Err(LensError::SemaNotPassed),
                // Unsupported / exempt / vacuous: skip this class, keep going.
                Err(LensError::Unsupported(_)) => {}
            }
            // (c) Range / overflow.
            match more_vc::build_range_vc(role, entry) {
                Ok(report) => reports.push(report),
                Err(LensError::RawStmtInCovenant) => return Err(LensError::RawStmtInCovenant),
                Err(LensError::SemaNotPassed) => return Err(LensError::SemaNotPassed),
                Err(LensError::Unsupported(_)) => {}
            }
            // (b) Declared-refinement implications G ⟹ φ.
            reports.extend(more_vc::build_refinement_vcs(role, entry, &declared));
            // (d) Declared-invariant preservation I(s) ∧ G ⟹ I(s').
            reports.extend(more_vc::build_preservation_vcs(role, entry, &declared));
        }
    }
    Ok(reports)
}

/// Discharge a VC report by shelling out to the `z3` binary (M2).
///
/// Fills [`VcReport::outcome`]. SOUNDNESS: only the literal `unsat` for the
/// negated VC maps to [`Outcome::Proved`], **and only after** a second z3 call
/// confirms the transition relation `T` alone is `sat` (the entrypoint is
/// reachable). If `T` alone is `unsat` the guards are self-contradictory and
/// `(T ∧ ¬VC)` is vacuously `unsat`; that maps to [`Outcome::Unknown`]
/// ("vacuous transition"), **never** [`Outcome::Proved`]. `sat` for the negated
/// VC maps to [`Outcome::Refuted`] with the captured counter-model; **every**
/// other path (z3 absent/errors, `unknown`, timeout, non-verdict, parse failure,
/// or an indecisive vacuity probe) maps to [`Outcome::Unknown`].
///
/// `timeout_ms` bounds the solver per-VC. The binary is discovered via the
/// `PORTRAIT_Z3` env var first, then `z3` on `PATH`.
pub fn discharge(report: &mut VcReport, timeout_ms: u64) {
    report.outcome = Some(solve::discharge(
        &report.transition_smtlib,
        &report.smtlib,
        timeout_ms,
    ));
}

/// Probe once whether a usable `z3` binary is reachable (`PORTRAIT_Z3` then
/// `PATH`). Used by the CLI and by solver-gated tests to self-skip when z3 is
/// absent, keeping the default `cargo test` green on a machine without z3.
pub fn z3_available() -> bool {
    solve::z3_available()
}
