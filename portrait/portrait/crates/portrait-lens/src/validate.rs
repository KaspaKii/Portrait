//! M4 — independent counter-model validation for the REFUTED side.
//!
//! SOUNDNESS POSTURE: this module ONLY touches the REFUTED side of [`crate::Outcome`].
//! It NEVER upgrades anything to [`crate::Outcome::Proved`] and NEVER changes the
//! PROVED/UNKNOWN logic in [`crate::solve`]. When z3 reports `sat` (a candidate
//! counter-example), we do NOT blindly trust the solver: we extract the model's
//! concrete integer/boolean values and INDEPENDENTLY replay them in Rust against
//! the asserted transition + negated VC, over interpreted (integer/bool)
//! arithmetic only.
//!
//! - If every asserted term evaluates to `true` under the concrete assignment
//!   using ONLY interpreted values ⇒ the witness is a genuine, self-contained
//!   counter-example ⇒ [`WitnessConfidence::Confirmed`].
//! - If validation must consult an UNINTERPRETED-function value (`checkSig`,
//!   `blake2b`, an opaque `acc_*` selector, …) whose truth does not correspond to
//!   any concrete on-chain execution ⇒ the REFUTED may be an over-approximation
//!   artifact ⇒ [`WitnessConfidence::Candidate`] (flagged, NOT silently trusted).
//!
//! The validator is fail-closed toward CANDIDATE: any parse gap, missing value, or
//! evaluation it cannot complete over pure integers/booleans degrades the witness
//! to CANDIDATE. A CONFIRMED is earned only by a complete, interpreted replay.

use std::collections::BTreeMap;

use crate::sexpr::{parse_all, Sexpr};

/// How much trust an independent Rust replay places in a z3 `sat` (REFUTED) verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessConfidence {
    /// The model's concrete integer/boolean assignment was independently replayed
    /// in Rust: every guard held, the next-state body matched, and the VC was
    /// concretely violated — using ONLY interpreted values. A validated witness.
    Confirmed,
    /// The REFUTED could NOT be independently confirmed over pure integers/booleans:
    /// the witness relies on an uninterpreted-function value (or the model could
    /// not be fully replayed). It may be an over-approximation artifact. The
    /// counter-example is reported, but flagged unvalidated.
    Candidate {
        /// Human-readable reason the witness could not be confirmed.
        reason: String,
    },
}

/// A concrete value pulled from a z3 model for one declared symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    /// An interpreted integer (z3 `Int`).
    Int(i128),
    /// An interpreted boolean (z3 `Bool`).
    Bool(bool),
    /// An opaque value (uninterpreted sort element, or an unparseable body):
    /// present in the model but NOT usable by the interpreted replay.
    Opaque,
}

/// Independently validate a z3 `sat` (REFUTED) verdict by replaying the model's
/// concrete integer/boolean assignment against the asserted transition + negated
/// VC, over interpreted arithmetic only.
///
/// `full_smtlib` is the exact document handed to z3 (its `(assert ...)` forms are
/// the guards, next-state equalities, domain axioms, and the negated VC).
/// `model` is z3's `get-model` body.
///
/// Returns [`WitnessConfidence::Confirmed`] iff every asserted term evaluates to
/// `true` under the model using ONLY interpreted values; otherwise
/// [`WitnessConfidence::Candidate`] with a reason (fail-closed).
pub fn validate_refutation(full_smtlib: &str, model: &str) -> WitnessConfidence {
    let assignment = match parse_model(model) {
        Ok(a) => a,
        Err(reason) => return WitnessConfidence::Candidate { reason },
    };
    // Symbols defined as uninterpreted FUNCTIONS in the model (e.g. checkSig with
    // a `((x!0 Sig)) Bool` arg list). An assert that *applies* one of these
    // depends on an uninterpreted value the replay cannot stand behind.
    let uf_names = uninterpreted_function_names(model);

    let asserts = match collect_asserts(full_smtlib) {
        Ok(a) => a,
        Err(reason) => return WitnessConfidence::Candidate { reason },
    };
    if asserts.is_empty() {
        return WitnessConfidence::Candidate {
            reason: "no assertions to replay against the model".to_string(),
        };
    }

    let ev = Evaluator {
        assignment: &assignment,
        uf_names: &uf_names,
    };
    for term in &asserts {
        match ev.eval_bool(term) {
            Ok(true) => {}
            Ok(false) => {
                // The model does not actually satisfy an asserted term under the
                // interpreted replay — z3's witness is not reproducible over pure
                // integers/booleans. Flag rather than confirm.
                return WitnessConfidence::Candidate {
                    reason: format!(
                        "an asserted term did not hold under the interpreted replay: {}",
                        term.render()
                    ),
                };
            }
            Err(reason) => {
                return WitnessConfidence::Candidate { reason };
            }
        }
    }
    WitnessConfidence::Confirmed
}

/// Parse the `(define-fun NAME () SORT BODY)` constant definitions from a z3
/// `get-model` body into a `name → Value` map. Function definitions
/// (`(define-fun NAME (args...) ...)`) and opaque-universe boilerplate are skipped
/// here (functions are handled separately by [`uninterpreted_function_names`]).
fn parse_model(model: &str) -> Result<BTreeMap<String, Value>, String> {
    let forms = parse_all(model).map_err(|e| format!("could not parse z3 model: {e}"))?;
    let mut map = BTreeMap::new();
    // z3 wraps the definitions in a single top-level list `( ... )`; accept both a
    // wrapping list and a bare sequence of define-funs.
    let items: Vec<&Sexpr> = match forms.as_slice() {
        [Sexpr::List(inner)] => inner.iter().collect(),
        many => many.iter().collect(),
    };
    for form in items {
        let Sexpr::List(parts) = form else { continue };
        // (define-fun NAME () SORT BODY)
        if parts.len() == 5 {
            if let (Some(Sexpr::Atom(kw)), Some(Sexpr::Atom(name)), Some(Sexpr::List(args))) =
                (parts.first(), parts.get(1), parts.get(2))
            {
                if kw == "define-fun" && args.is_empty() {
                    let sort = &parts[3];
                    let body = &parts[4];
                    map.insert(name.clone(), value_of(sort, body));
                }
            }
        }
        // (declare-fun NAME () SORT) for opaque-universe elements: record as Opaque
        // so a reference resolves (to a non-interpreted value) rather than erroring.
        if parts.len() == 4 {
            if let (Some(Sexpr::Atom(kw)), Some(Sexpr::Atom(name)), Some(Sexpr::List(args))) =
                (parts.first(), parts.get(1), parts.get(2))
            {
                if kw == "declare-fun" && args.is_empty() {
                    map.entry(name.clone()).or_insert(Value::Opaque);
                }
            }
        }
    }
    Ok(map)
}

/// Interpret a `(define-fun NAME () SORT BODY)` body as a [`Value`] given the
/// declared sort. `Int`/`Bool` bodies become interpreted values; anything else
/// (opaque-sort element, non-literal body) is [`Value::Opaque`].
fn value_of(sort: &Sexpr, body: &Sexpr) -> Value {
    match sort {
        Sexpr::Atom(s) if s == "Int" => int_literal(body).map(Value::Int).unwrap_or(Value::Opaque),
        Sexpr::Atom(s) if s == "Bool" => match body {
            Sexpr::Atom(b) if b == "true" => Value::Bool(true),
            Sexpr::Atom(b) if b == "false" => Value::Bool(false),
            _ => Value::Opaque,
        },
        _ => Value::Opaque,
    }
}

/// Parse an SMT integer literal: a bare numeral `12`, or the negation form
/// `(- 5)` z3 prints for negatives. Returns `None` for anything else (which the
/// caller maps to [`Value::Opaque`], fail-closed).
fn int_literal(body: &Sexpr) -> Option<i128> {
    match body {
        Sexpr::Atom(a) => a.parse::<i128>().ok(),
        Sexpr::List(parts) => match parts.as_slice() {
            [Sexpr::Atom(op), Sexpr::Atom(n)] if op == "-" => n.parse::<i128>().ok().map(|v| -v),
            _ => None,
        },
    }
}

/// The set of symbols the model defines as uninterpreted FUNCTIONS (a non-empty
/// argument list in a `define-fun`). An asserted term that applies one of these
/// is depending on an uninterpreted value ⇒ CANDIDATE.
fn uninterpreted_function_names(model: &str) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    let Ok(forms) = parse_all(model) else {
        return names;
    };
    let items: Vec<&Sexpr> = match forms.as_slice() {
        [Sexpr::List(inner)] => inner.iter().collect(),
        many => many.iter().collect(),
    };
    for form in items {
        if let Sexpr::List(parts) = form {
            if parts.len() == 5 {
                if let (Some(Sexpr::Atom(kw)), Some(Sexpr::Atom(name)), Some(Sexpr::List(args))) =
                    (parts.first(), parts.get(1), parts.get(2))
                {
                    if kw == "define-fun" && !args.is_empty() {
                        names.insert(name.clone());
                    }
                }
            }
        }
    }
    names
}

/// Pull the body terms of every `(assert TERM)` from an SMT-LIB document.
fn collect_asserts(smtlib: &str) -> Result<Vec<Sexpr>, String> {
    let forms = parse_all(smtlib).map_err(|e| format!("could not parse SMT-LIB document: {e}"))?;
    let mut out = Vec::new();
    for form in forms {
        if let Sexpr::List(parts) = &form {
            if let Some(Sexpr::Atom(head)) = parts.first() {
                if head == "assert" && parts.len() == 2 {
                    out.push(parts[1].clone());
                }
            }
        }
    }
    Ok(out)
}

/// Evaluates ground SMT-LIB terms against a concrete model assignment, over
/// interpreted integer/boolean arithmetic ONLY. Any application of an
/// uninterpreted function (or any reference it cannot resolve to an interpreted
/// value) is an error — surfaced to the caller as CANDIDATE (fail-closed).
struct Evaluator<'a> {
    assignment: &'a BTreeMap<String, Value>,
    uf_names: &'a std::collections::BTreeSet<String>,
}

/// An interpreted evaluation result.
enum Eval {
    Int(i128),
    Bool(bool),
}

impl Evaluator<'_> {
    /// Evaluate a term expected to be Bool. Errors if it is not interpreted-Bool.
    fn eval_bool(&self, term: &Sexpr) -> Result<bool, String> {
        match self.eval(term)? {
            Eval::Bool(b) => Ok(b),
            Eval::Int(_) => Err(format!(
                "expected a boolean assertion but got an integer term: {}",
                term.render()
            )),
        }
    }

    fn eval_int(&self, term: &Sexpr) -> Result<i128, String> {
        match self.eval(term)? {
            Eval::Int(n) => Ok(n),
            Eval::Bool(_) => Err(format!(
                "expected an integer operand but got a boolean term: {}",
                term.render()
            )),
        }
    }

    fn eval(&self, term: &Sexpr) -> Result<Eval, String> {
        match term {
            Sexpr::Atom(a) => self.eval_atom(a),
            Sexpr::List(parts) => self.eval_list(parts, term),
        }
    }

    fn eval_atom(&self, a: &str) -> Result<Eval, String> {
        if a == "true" {
            return Ok(Eval::Bool(true));
        }
        if a == "false" {
            return Ok(Eval::Bool(false));
        }
        if let Ok(n) = a.parse::<i128>() {
            return Ok(Eval::Int(n));
        }
        match self.assignment.get(a) {
            Some(Value::Int(n)) => Ok(Eval::Int(*n)),
            Some(Value::Bool(b)) => Ok(Eval::Bool(*b)),
            Some(Value::Opaque) => Err(format!(
                "symbol `{a}` resolves to an uninterpreted (opaque) value; \
                 witness depends on a non-integer/non-boolean value"
            )),
            None => Err(format!(
                "symbol `{a}` has no concrete value in the model; cannot replay over integers"
            )),
        }
    }

    fn eval_list(&self, parts: &[Sexpr], whole: &Sexpr) -> Result<Eval, String> {
        let Some(Sexpr::Atom(op)) = parts.first() else {
            return Err(format!(
                "non-atom operator head in term: {}",
                whole.render()
            ));
        };
        let args = &parts[1..];
        // An application of a symbol the model defines as an uninterpreted function
        // means the witness leans on an uninterpreted value: CANDIDATE.
        if self.uf_names.contains(op) {
            return Err(format!(
                "term applies uninterpreted function `{op}`; witness depends on an \
                 uninterpreted value (possible over-approximation artifact)"
            ));
        }
        match op.as_str() {
            // ── Boolean connectives ──────────────────────────────────────────
            "not" => {
                let b = self.eval_bool(&args[0])?;
                Ok(Eval::Bool(!b))
            }
            "and" => {
                for a in args {
                    if !self.eval_bool(a)? {
                        return Ok(Eval::Bool(false));
                    }
                }
                Ok(Eval::Bool(true))
            }
            "or" => {
                for a in args {
                    if self.eval_bool(a)? {
                        return Ok(Eval::Bool(true));
                    }
                }
                Ok(Eval::Bool(false))
            }
            "=>" => {
                let p = self.eval_bool(&args[0])?;
                let q = self.eval_bool(&args[1])?;
                Ok(Eval::Bool(!p || q))
            }
            // ── Equality / distinct (over integers or booleans) ──────────────
            "=" => Ok(Eval::Bool(self.all_equal(args)?)),
            "distinct" => Ok(Eval::Bool(!self.all_equal(args)?)),
            // ── Integer comparisons ──────────────────────────────────────────
            "<" | "<=" | ">" | ">=" => {
                let l = self.eval_int(&args[0])?;
                let r = self.eval_int(&args[1])?;
                let v = match op.as_str() {
                    "<" => l < r,
                    "<=" => l <= r,
                    ">" => l > r,
                    ">=" => l >= r,
                    _ => unreachable!(),
                };
                Ok(Eval::Bool(v))
            }
            // ── Integer arithmetic ───────────────────────────────────────────
            "+" => {
                let mut acc: i128 = 0;
                for a in args {
                    acc = acc
                        .checked_add(self.eval_int(a)?)
                        .ok_or("integer overflow during interpreted replay (+)")?;
                }
                Ok(Eval::Int(acc))
            }
            "*" => {
                let mut acc: i128 = 1;
                for a in args {
                    acc = acc
                        .checked_mul(self.eval_int(a)?)
                        .ok_or("integer overflow during interpreted replay (*)")?;
                }
                Ok(Eval::Int(acc))
            }
            "-" => {
                if args.len() == 1 {
                    let n = self.eval_int(&args[0])?;
                    return Ok(Eval::Int(
                        n.checked_neg()
                            .ok_or("integer overflow during interpreted replay (neg)")?,
                    ));
                }
                let mut acc = self.eval_int(&args[0])?;
                for a in &args[1..] {
                    acc = acc
                        .checked_sub(self.eval_int(a)?)
                        .ok_or("integer overflow during interpreted replay (-)")?;
                }
                Ok(Eval::Int(acc))
            }
            // Any other head is treated as an (undeclared) uninterpreted symbol:
            // the replay cannot stand behind it ⇒ CANDIDATE.
            other => Err(format!(
                "term uses operator `{other}` outside the interpreted (integer/bool) \
                 replay fragment; cannot confirm witness"
            )),
        }
    }

    /// `true` iff all operands evaluate equal (works for ints and bools, but the
    /// operands must be of one kind).
    fn all_equal(&self, args: &[Sexpr]) -> Result<bool, String> {
        if args.len() < 2 {
            return Ok(true);
        }
        // Try integer comparison first; fall back to boolean.
        let first = self.eval(&args[0])?;
        match first {
            Eval::Int(n0) => {
                for a in &args[1..] {
                    if self.eval_int(a)? != n0 {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Eval::Bool(b0) => {
                for a in &args[1..] {
                    if self.eval_bool(a)? != b0 {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_integer_witness_is_confirmed() {
        // A value-creating transition: balance' = balance - x, other = other + x + 1.
        // The negated conservation VC fires. The model is pure-integer, so the
        // independent replay must CONFIRM.
        let full = "(set-logic QF_LIA)\n\
            (declare-const balance Int)\n\
            (declare-const other Int)\n\
            (declare-const x Int)\n\
            (declare-const balance_p Int)\n\
            (declare-const other_p Int)\n\
            (assert (>= x 0))\n\
            (assert (= balance_p (- balance x)))\n\
            (assert (= other_p (+ other x 1)))\n\
            (assert (not (= (+ balance_p other_p) (+ balance other))))\n\
            (check-sat)\n(get-model)\n";
        let model = "(\n  (define-fun balance () Int 10)\n  (define-fun other () Int 0)\n\
            (define-fun x () Int 3)\n  (define-fun balance_p () Int 7)\n\
            (define-fun other_p () Int 4)\n)";
        assert_eq!(
            validate_refutation(full, model),
            WitnessConfidence::Confirmed
        );
    }

    #[test]
    fn witness_relying_on_uninterpreted_function_is_candidate() {
        // The only thing making the negated VC `sat` is an uninterpreted
        // predicate `(opaque x)` asserted true. The replay cannot stand behind it.
        let full = "(set-logic QF_AUFLIA)\n\
            (declare-const x Int)\n\
            (declare-fun opaque (Int) Bool)\n\
            (assert (opaque x))\n\
            (assert (not (>= x 0)))\n\
            (check-sat)\n(get-model)\n";
        let model = "(\n  (define-fun x () Int (- 1))\n\
            (define-fun opaque ((x!0 Int)) Bool true)\n)";
        match validate_refutation(full, model) {
            WitnessConfidence::Candidate { reason } => {
                assert!(
                    reason.contains("opaque"),
                    "reason should name the UF: {reason}"
                );
            }
            other => panic!("expected Candidate, got {other:?}"),
        }
    }

    #[test]
    fn negative_integer_literal_is_parsed() {
        assert_eq!(int_literal(&parse_all("(- 5)").unwrap()[0]), Some(-5));
        assert_eq!(int_literal(&parse_all("12").unwrap()[0]), Some(12));
    }

    #[test]
    fn missing_model_value_degrades_to_candidate_fail_closed() {
        // `y` is asserted but absent from the model: the replay cannot complete
        // over integers, so it must fail closed to CANDIDATE (never CONFIRMED).
        let full = "(assert (>= y 0))\n(check-sat)\n";
        let model = "(\n  (define-fun x () Int 1)\n)";
        assert!(matches!(
            validate_refutation(full, model),
            WitnessConfidence::Candidate { .. }
        ));
    }

    #[test]
    fn opaque_sort_value_makes_application_candidate() {
        // checkSig over opaque-sort args, asserted true; the negated VC only fires
        // because of it. Replay must flag CANDIDATE.
        let full = "(set-logic QF_AUFLIA)\n\
            (declare-sort Sig 0)\n(declare-sort PubKey 0)\n\
            (declare-const auth Sig)\n(declare-const owner PubKey)\n\
            (declare-fun checkSig (Sig PubKey) Bool)\n\
            (assert (checkSig auth owner))\n\
            (assert (not false))\n\
            (check-sat)\n(get-model)\n";
        let model = "(\n  (declare-fun Sig!val!0 () Sig)\n\
            (declare-fun PubKey!val!0 () PubKey)\n\
            (define-fun auth () Sig Sig!val!0)\n\
            (define-fun owner () PubKey PubKey!val!0)\n\
            (define-fun checkSig ((x!0 Sig) (x!1 PubKey)) Bool true)\n)";
        match validate_refutation(full, model) {
            WitnessConfidence::Candidate { reason } => {
                assert!(reason.contains("checkSig"), "reason: {reason}");
            }
            other => panic!("expected Candidate, got {other:?}"),
        }
    }
}
