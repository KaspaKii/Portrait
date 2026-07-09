//! M6 — STRUCTURAL translation validation across the model-vs-`.sil` gap.
//!
//! Lens proves properties of the Portrait **MODEL** (the AST-derived transition
//! relation); the chain enforces the **emitted `.sil`**. The honest #1 gap (see
//! `lib.rs` and `MODEL_NOT_SIL_CAVEAT`) is that Lens never checks the `.sil`
//! actually corresponds to the model it proved over. A faithful proof is worthless
//! if the engraver silently dropped a guard or introduced a value-bearing op the
//! model never accounted for.
//!
//! This module builds a STRUCTURAL correspondence check across that gap. For each
//! transition entrypoint it derives two fact sets:
//!
//! - **From the model (AST):** the multiset of guard predicates (one per
//!   `requires` line, i.e. each [`portrait_syntax::Stmt::Require`]) and the set of
//!   *value-bearing field writes* in the next-state return (a value-bearing field
//!   assigned anything other than a pure carry of its own prior value).
//! - **From the emitted `.sil` (text):** the `require(...)` clauses and the
//!   `return({ field: value, ... })` field writes of the matching `function`
//!   block.
//!
//! It then reports:
//!
//! - **CORRESPONDS** iff (1) every model guard maps to a `.sil` `require` (no model
//!   guard silently dropped in emission), AND (2) the model and the `.sil` agree on
//!   the value-bearing writes in **both directions**: the `.sil` introduces no
//!   value-bearing write the model did not account for (no unaccounted mint / extra
//!   output) AND every model value-bearing write appears in the `.sil` and is not
//!   dropped or replaced by a pure carry (no value lost / source leg never moved).
//! - **DIVERGES(reason…)** otherwise, naming each specific divergence — a dropped
//!   guard, an extra/changed value-bearing op, a dropped model value-bearing write,
//!   or a duplicate (shadowing) entrypoint block.
//!
//! ## Honest scope (paramount)
//!
//! This is a **STRUCTURAL** correspondence, **NOT** a semantic refinement proof.
//! It catches engraver drift of the specific, decidable shapes above (a dropped
//! `requires`; a value-bearing write that is added, altered, or dropped between
//! model and `.sil`; a duplicate entrypoint block). It does **NOT** prove
//! the emitted `.sil` *behaves* like the model: it does not give the `.sil` a
//! formal semantics, does not relate the two as a simulation/refinement, and does
//! not reason about silverscript builtins (`OpInputCovenantId`, `checkSig`, …)
//! beyond textual equality after a fixed normalization. A genuine behavioural
//! equivalence would require a `.sil` operational semantics plus an SMT refinement
//! obligation — the remaining deeper step, explicitly out of scope here.
//!
//! By design the `.sil` may carry **extra** `require` clauses the model did not
//! state (e.g. the engraver-injected `require(proof_cov_id == OpInputCovenantId(0))`
//! CSCI cross-layer binding): an *added* guard only tightens enforcement and is NOT
//! a divergence. Only a **dropped model guard**, a **value-bearing write added,
//! altered, or dropped** between model and `.sil`, or a **duplicate entrypoint
//! block** is.
//!
//! **Pre-production, unaudited, testnet-only.**

use crate::encode::is_value_bearing_split;
use portrait_syntax::{CovenantMode, Entry, Expr, Program, ReturnExpr, Role, Stmt};

/// The on-its-face honest footer for every translation-validation report. States
/// the structural-only boundary and the maturity stamp.
pub const TRANSLATION_STRUCTURAL_FOOTER: &str =
    "STRUCTURAL correspondence between the Portrait MODEL and the emitted .sil \
     (catches a dropped guard, or a value-bearing write added/altered/dropped \
     between model and .sil); NOT a semantic refinement proof — it does NOT \
     establish behavioural equivalence (that needs a .sil semantics + an SMT \
     refinement obligation). pre-production, unaudited, testnet-only.";

/// The verdict of a structural translation-validation check for one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Correspondence {
    /// Every model guard maps to a `.sil` `require`, and the model and `.sil` agree
    /// on every value-bearing write (none added, altered, or dropped). STRUCTURAL
    /// only.
    Corresponds,
    /// At least one structural divergence was found. `reasons` names each one (a
    /// dropped guard, a value-bearing write added/altered/dropped between model and
    /// `.sil`, a duplicate block, or a missing block).
    Diverges {
        /// One human-readable line per specific divergence found.
        reasons: Vec<String>,
    },
}

impl Correspondence {
    /// `true` iff this is [`Correspondence::Corresponds`].
    pub fn corresponds(&self) -> bool {
        matches!(self, Correspondence::Corresponds)
    }
}

/// Run the STRUCTURAL translation-validation check for `program` against its
/// emitted silverscript `sil_text`.
///
/// For every covenant **transition** entrypoint of every role, this builds the
/// model fact set from the AST and the corresponding fact set from the matching
/// `function NAME(...)` block in `sil_text`, then checks the two correspondence
/// rules (no dropped model guard; no unaccounted value-bearing write).
///
/// Returns [`Correspondence::Corresponds`] iff every checked entrypoint
/// corresponds; otherwise [`Correspondence::Diverges`] naming each divergence.
/// Non-transition entrypoints (verification / non-covenant) are skipped — they
/// are handled by other layers and carry no value-conservation surface here.
///
/// HONEST SCOPE: STRUCTURAL only — see the module docs and
/// [`TRANSLATION_STRUCTURAL_FOOTER`]. A CORRESPONDS is NOT a behavioural-equivalence
/// claim.
pub fn validate_translation(program: &Program, sil_text: &str) -> Correspondence {
    // Strip comments BEFORE any structural scan. The downstream extractors are
    // ad-hoc substring/brace matchers with no lexer, so a commented-out
    // `// require(...)` would otherwise be harvested as a live guard, and a
    // `/* ... */`-hidden block would distort brace balancing. Stripping comments up
    // front means the whole check reasons only over code that the chain enforces.
    let sil_text = &strip_sil_comments(sil_text);
    let mut reasons = Vec::new();
    for role in &program.app.roles {
        for entry in &role.entrypoints {
            if !matches!(entry.mode, CovenantMode::Transition) {
                continue;
            }
            check_entrypoint(role, entry, sil_text, &mut reasons);
        }
    }
    if reasons.is_empty() {
        Correspondence::Corresponds
    } else {
        Correspondence::Diverges { reasons }
    }
}

/// Check one transition entrypoint, pushing a divergence line per finding.
fn check_entrypoint(role: &Role, entry: &Entry, sil_text: &str, reasons: &mut Vec<String>) {
    let qual = format!("{}.{}", role.name, entry.name);

    // ── Locate the matching `.sil` function block. ───────────────────────────
    // Reject a duplicate same-name definition: a shadowing block could carry a
    // different (malicious) guard/value surface that inspecting only the first
    // block would miss, so a non-unique definition is itself a divergence.
    if count_function_blocks(sil_text, &entry.name) > 1 {
        reasons.push(format!(
            "{qual}: multiple `function {}` blocks in the emitted .sil (duplicate / \
             shadowing definition — cannot structurally validate a non-unique entrypoint)",
            entry.name
        ));
        return;
    }
    let block = match extract_function_block(sil_text, &entry.name) {
        Some(b) => b,
        None => {
            reasons.push(format!(
                "{qual}: no `function {}` block found in the emitted .sil (the model \
                 entrypoint was not emitted)",
                entry.name
            ));
            return;
        }
    };

    // ── Rule 1: no model guard silently dropped in emission. ─────────────────
    // Model guards: one normalized predicate per `requires` (Stmt::Require).
    let sil_requires = extract_sil_requires(&block);
    for stmt in &entry.body {
        if let Stmt::Require(expr) = stmt {
            let model_guard = normalize_predicate(&expr.to_silverscript());
            if !sil_requires.iter().any(|r| r == &model_guard) {
                reasons.push(format!(
                    "{qual}: model guard `requires {}` has NO corresponding require(...) in the \
                     emitted .sil (guard dropped in emission)",
                    expr.to_silverscript()
                ));
            }
        }
    }

    // ── Rule 2: model and .sil agree on every value-bearing write — BOTH
    //    directions. (a) the .sil introduces no value-bearing write the model did
    //    not account for (no unaccounted mint / extra output); AND (b) every model
    //    value-bearing write appears in the .sil and is not silently dropped or
    //    substituted by a pure carry (no value lost / source leg never moved). ──
    let model_writes = model_value_writes(role, entry);
    let sil_writes = sil_value_writes(role, &block);
    // (a) sil -> model: every .sil value write is accounted for by the model.
    for (field, sil_val) in &sil_writes {
        match model_writes.iter().find(|(f, _)| f == field) {
            None => reasons.push(format!(
                "{qual}: emitted .sil writes value-bearing field `{field}` (= `{sil_val}`) that \
                 the model does not write (unaccounted value-bearing op / extra output)"
            )),
            Some((_, model_val)) if model_val != sil_val => reasons.push(format!(
                "{qual}: emitted .sil writes value-bearing field `{field}` as `{sil_val}` but the \
                 model writes `{model_val}` (value-bearing op altered in emission)"
            )),
            Some(_) => {}
        }
    }
    // (b) model -> sil: every model value write must appear in the .sil (and not as
    // a pure carry, which sil_value_writes already skips). A model write absent from
    // sil_writes is either omitted entirely or substituted by a pure carry — both
    // are value-bearing ops dropped in emission (value lost / source leg never
    // moved). The mismatch (altered) case is already reported by (a).
    for (field, model_val) in &model_writes {
        if !sil_writes.iter().any(|(f, _)| f == field) {
            reasons.push(format!(
                "{qual}: model writes value-bearing field `{field}` (= `{model_val}`) but the \
                 emitted .sil does not (write dropped or replaced by a pure carry in emission — \
                 value lost / leg never moved)"
            ));
        }
    }
}

/// The model's value-bearing field writes: each next-state object-return field
/// that is value-bearing and is NOT a pure carry (`f: f`). Returned as normalized
/// `(field, value)` pairs so they compare directly with [`sil_value_writes`].
fn model_value_writes(role: &Role, entry: &Entry) -> Vec<(String, String)> {
    let return_fields = entry.body.iter().find_map(|s| match s {
        Stmt::Return(ReturnExpr::Object { fields, .. }) => Some(fields),
        _ => None,
    });
    let Some(fields) = return_fields else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (field, value) in fields {
        if !role
            .state
            .iter()
            .any(|f| &f.name == field && is_value_bearing_split(&f.name, &f.ty))
        {
            continue;
        }
        // A pure carry `f: f` is not a value-bearing *op* — no value moves.
        if matches!(value, Expr::Var(n) if n == field) {
            continue;
        }
        out.push((field.clone(), normalize_predicate(&value.to_silverscript())));
    }
    out
}

/// The emitted `.sil`'s value-bearing field writes, parsed from the
/// `return({ field: value, ... })` clause of `block`. Returned as normalized
/// `(field, value)` pairs, skipping pure carries (`f: prev_states[0].f`).
fn sil_value_writes(role: &Role, block: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(inner) = extract_return_object(block) else {
        return out;
    };
    for (field, value) in split_object_fields(&inner) {
        if !role
            .state
            .iter()
            .any(|f| f.name == field && is_value_bearing_split(&f.name, &f.ty))
        {
            continue;
        }
        let norm = normalize_predicate(&value);
        // Pure carry after normalization is `field` itself (prev_states[0]. stripped).
        if norm == field {
            continue;
        }
        out.push((field, norm));
    }
    out
}

/// Find the byte offset of the next `function <ws> NAME <ws> (` definition in
/// `sil` at or after `from`, tolerating ANY inter-token whitespace (so a shadow
/// block spelled `function  NAME(` with extra spaces — or a newline — cannot evade
/// the match). Returns the offset of the `function` keyword.
fn find_function_block(sil: &str, name: &str, from: usize) -> Option<usize> {
    let mut search = from;
    while let Some(rel) = sil[search..].find("function") {
        let kw = search + rel;
        let after_kw = kw + "function".len();
        let rest = &sil[after_kw..];
        // Require at least one whitespace char after the keyword, then the name,
        // then optional whitespace, then `(`.
        let trimmed = rest.trim_start();
        let consumed_ws = rest.len() - trimmed.len();
        if consumed_ws > 0 {
            if let Some(tail) = trimmed.strip_prefix(name) {
                let tail_trimmed = tail.trim_start();
                // Guard against a longer identifier sharing this prefix (e.g.
                // `function rebalanceX(`): the char after the name must not continue
                // an identifier, and the next non-ws token must be `(`.
                let name_boundary_ok = tail
                    .chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                if name_boundary_ok && tail_trimmed.starts_with('(') {
                    return Some(kw);
                }
            }
        }
        search = after_kw;
    }
    None
}

/// Count the number of `function NAME(` definitions in `sil` (whitespace-tolerant).
/// Used to reject a duplicate / shadowing same-name block, which the single-block
/// extractor would otherwise silently ignore.
fn count_function_blocks(sil: &str, name: &str) -> usize {
    let mut count = 0;
    let mut from = 0;
    while let Some(kw) = find_function_block(sil, name, from) {
        count += 1;
        from = kw + "function".len();
    }
    count
}

/// Extract the body text of the first `function NAME(...) ... { BODY }` block in
/// `sil`. Matches on the `function NAME(` token (whitespace-tolerant) and returns
/// the brace-balanced body between the signature's opening `{` and its matching `}`.
fn extract_function_block(sil: &str, name: &str) -> Option<String> {
    let start = find_function_block(sil, name, 0)?;
    // Find the first `{` after the signature, then balance braces.
    let after = &sil[start..];
    let open_rel = after.find('{')?;
    let bytes = after.as_bytes();
    let mut depth = 0usize;
    let mut i = open_rel;
    let body_start = open_rel + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(after[body_start..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Extract every normalized `require(...)` predicate from a `.sil` function body.
fn extract_sil_requires(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(pos) = rest.find("require(") {
        let after = &rest[pos + "require(".len()..];
        if let Some(arg) = balanced_paren_arg(after) {
            out.push(normalize_predicate(&arg));
            rest = &after[arg.len()..];
        } else {
            break;
        }
    }
    out
}

/// Extract the inner text of the `return({ ... })` object literal in a `.sil`
/// function body, i.e. the `field: value, ...` between the `{` and `}`.
fn extract_return_object(block: &str) -> Option<String> {
    let pos = block.find("return(")?;
    let after = &block[pos + "return(".len()..];
    let brace = after.find('{')?;
    let bytes = after.as_bytes();
    let mut depth = 0usize;
    let mut i = brace;
    let inner_start = brace + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(after[inner_start..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Read the brace/paren-balanced argument inside a `require(...)` — the text up to
/// the matching close paren of the already-consumed open paren. Returns the inner
/// argument text (without the trailing `)`).
fn balanced_paren_arg(after_open: &str) -> Option<String> {
    let bytes = after_open.as_bytes();
    let mut depth = 1usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(after_open[..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a `field: value, field: value, ...` object-body string into `(field,
/// value)` pairs, respecting nested `(`/`{` so a comma inside `f(a, b)` does not
/// split a value.
fn split_object_fields(inner: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let bytes = inner.as_bytes();
    let mut depth = 0i32;
    let mut seg_start = 0usize;
    let mut i = 0usize;
    let mut segments: Vec<&str> = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b',' if depth == 0 => {
                segments.push(&inner[seg_start..i]);
                seg_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if seg_start < inner.len() {
        segments.push(&inner[seg_start..]);
    }
    for seg in segments {
        // Split on the FIRST top-level `:` (a field key never contains `:`).
        if let Some(colon) = seg.find(':') {
            let key = seg[..colon].trim().to_string();
            let val = seg[colon + 1..].trim().to_string();
            if !key.is_empty() {
                pairs.push((key, val));
            }
        }
    }
    pairs
}

/// Strip `//` line comments and `/* ... */` block comments from `.sil` text so the
/// downstream substring/brace scanners only ever see code the chain enforces. A
/// commented-out `require(...)` or `function ...` block is dead text on-chain;
/// counting it as live would let the structural check report CORRESPONDS for a
/// `.sil` that actually dropped a guard or shadowed a block. String/char literals
/// in Portrait-emitted `.sil` do not contain comment delimiters, so a naive scan is
/// sufficient here; comment bytes are replaced with spaces to preserve byte offsets
/// and token boundaries.
fn strip_sil_comments(sil: &str) -> String {
    let bytes = sil.as_bytes();
    let mut out = String::with_capacity(sil.len());
    let mut i = 0usize;
    while i < bytes.len() {
        // Line comment: `// ... \n`
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        // Block comment: `/* ... */`
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            out.push(' ');
            out.push(' ');
            i += 2;
            while i < bytes.len() {
                if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    break;
                }
                // Preserve newlines so line structure is unchanged; blank the rest.
                out.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        // Default: copy the byte through (UTF-8 safe — we only special-case ASCII).
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&sil[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Byte length of the UTF-8 char beginning at `lead`.
fn utf8_char_len(lead: u8) -> usize {
    match lead {
        b if b < 0x80 => 1,
        b if b >> 5 == 0b110 => 2,
        b if b >> 4 == 0b1110 => 3,
        _ => 4,
    }
}

/// Normalize a predicate / value expression for structural comparison across the
/// model-vs-`.sil` boundary:
/// - strip the `prev_states[0].` state-access prefix the engraver injects (so a
///   `.sil` `prev_states[0].owner` compares equal to a model `owner`);
/// - re-parse the result and render it through the SAME canonical
///   [`Expr::to_silverscript`] the model side uses, so the comparison is robust to
///   incidental grouping parens (`a - (x + y)` vs `a - x + y` as the model's own
///   AST renders it) and whitespace. If re-parsing fails (a silverscript construct
///   outside the Portrait expression grammar), fall back to whitespace
///   normalization — a strictly textual, conservative comparison.
///
/// This is the FIXED, intentionally SYNTACTIC normalization the structural check
/// is defined modulo. It canonicalizes spelling, NOT meaning: it does not apply
/// arithmetic identities (`x * 2` stays distinct from `x + x`), so it can only ever
/// over-report (flag a benign re-spelling), never silently accept a real
/// value-bearing change.
fn normalize_predicate(s: &str) -> String {
    let stripped = s.replace("prev_states[0].", "");
    match portrait_syntax::parse_expr(stripped.trim()) {
        Ok(expr) => expr.to_silverscript(),
        Err(_) => stripped.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// InternalSplit: a genuine 3-leg internal flow. The emitted .sil below is the
    /// faithful engraver output — every model guard is present and the
    /// value-bearing writes match. Must CORRESPOND.
    const INTERNAL_SPLIT: &str = r#"pragma portrait ^0.1.0;
app InternalSplit {
  role pool {
    param int    pool_a_balance;
    param int    pool_b_balance;
    param int    pool_c_balance;
    param pubkey owner;
    state {
      int    pool_a_balance;
      int    pool_b_balance;
      int    pool_c_balance;
      pubkey owner;
    }
    #[covenant(mode = transition)]
    entrypoint function rebalance(sig auth, int x, int y)
      : (int pool_a_balance, int pool_b_balance, int pool_c_balance, pubkey owner) {
      requires checkSig(auth, owner);
      requires x >= 0;
      requires y >= 0;
      requires x + y <= pool_a_balance;
      return InternalSplit {
        pool_a_balance: pool_a_balance - (x + y),
        pool_b_balance: pool_b_balance + x,
        pool_c_balance: pool_c_balance + y,
        owner:          owner
      };
    }
  }
  lifecycle { live -> live via pool.rebalance; }
  invariant conservation_split;
}"#;

    const INTERNAL_SPLIT_SIL_FAITHFUL: &str = r#"pragma silverscript ^0.1.0;
contract InternalSplit(int max_ins, int max_outs, int pool_a_balance, int pool_b_balance, int pool_c_balance, pubkey owner) {
    int pool_a_balance = pool_a_balance;
    int pool_b_balance = pool_b_balance;
    int pool_c_balance = pool_c_balance;
    pubkey owner = owner;
    #[covenant(binding = cov, from = max_ins, to = 1, mode = transition)]
    function rebalance(State[] prev_states, sig auth, int x, int y) : (State) {
        require(checkSig(auth, prev_states[0].owner));
        require(x >= 0);
        require(y >= 0);
        require(x + y <= prev_states[0].pool_a_balance);
        return({ pool_a_balance: prev_states[0].pool_a_balance - (x + y), pool_b_balance: prev_states[0].pool_b_balance + x, pool_c_balance: prev_states[0].pool_c_balance + y, owner: prev_states[0].owner });
    }
}"#;

    fn parse(src: &str) -> Program {
        portrait_syntax::parse(src).expect("fixture parses")
    }

    #[test]
    fn faithful_emission_corresponds() {
        let prog = parse(INTERNAL_SPLIT);
        let verdict = validate_translation(&prog, INTERNAL_SPLIT_SIL_FAITHFUL);
        assert_eq!(verdict, Correspondence::Corresponds, "{verdict:?}");
    }

    #[test]
    fn dropped_guard_diverges_naming_it() {
        // Mutate the faithful .sil to DROP the `x + y <= pool_a_balance` overdraw
        // guard. The check must DIVERGE and name that specific dropped guard.
        let mutated = INTERNAL_SPLIT_SIL_FAITHFUL.replace(
            "        require(x + y <= prev_states[0].pool_a_balance);\n",
            "",
        );
        assert!(
            !mutated.contains("x + y <="),
            "mutation must actually remove the guard"
        );
        let prog = parse(INTERNAL_SPLIT);
        match validate_translation(&prog, &mutated) {
            Correspondence::Diverges { reasons } => {
                assert_eq!(reasons.len(), 1, "exactly one divergence: {reasons:?}");
                assert!(
                    reasons[0].contains("x + y <= pool_a_balance")
                        && reasons[0].contains("dropped"),
                    "must name the dropped guard: {}",
                    reasons[0]
                );
            }
            Correspondence::Corresponds => {
                panic!("a dropped guard MUST diverge (the check has teeth)")
            }
        }
    }

    #[test]
    fn extra_value_op_diverges_naming_it() {
        // Mutate the faithful .sil so leg b is minted MORE than the model says
        // (`+ x + 1` instead of `+ x`): an unaccounted value-bearing op. Must
        // DIVERGE naming the altered value-bearing write.
        let mutated = INTERNAL_SPLIT_SIL_FAITHFUL.replace(
            "pool_b_balance: prev_states[0].pool_b_balance + x,",
            "pool_b_balance: prev_states[0].pool_b_balance + x + 1,",
        );
        assert!(mutated.contains("+ x + 1"), "mutation applied");
        let prog = parse(INTERNAL_SPLIT);
        match validate_translation(&prog, &mutated) {
            Correspondence::Diverges { reasons } => {
                assert!(
                    reasons
                        .iter()
                        .any(|r| r.contains("pool_b_balance") && r.contains("altered")),
                    "must name the altered value-bearing op: {reasons:?}"
                );
            }
            Correspondence::Corresponds => {
                panic!("an extra value-bearing op MUST diverge (the check has teeth)")
            }
        }
    }

    #[test]
    fn injected_vprog_require_does_not_diverge() {
        // The engraver injects `require(proof_cov_id == OpInputCovenantId(0))` for
        // CSCI cross-layer binding. An ADDED guard only tightens enforcement; it is
        // NOT a dropped model guard and NOT a value-bearing op, so it must NOT
        // trigger a divergence.
        let with_injected = INTERNAL_SPLIT_SIL_FAITHFUL.replace(
            "        require(checkSig(auth, prev_states[0].owner));\n",
            "        require(proof_cov_id == OpInputCovenantId(0));\n        require(checkSig(auth, prev_states[0].owner));\n",
        );
        let prog = parse(INTERNAL_SPLIT);
        assert_eq!(
            validate_translation(&prog, &with_injected),
            Correspondence::Corresponds,
            "an injected extra require must not be flagged as a divergence"
        );
    }

    #[test]
    fn dropped_model_write_diverges() {
        // ATK1: the .sil OMITS the model's `pool_c_balance: + y` credit field
        // entirely. The source leg is debited but the destination leg never gets
        // its value — a dropped model value-bearing write (value lost / not
        // conserved). The check MUST DIVERGE naming the missing field.
        let mutated = INTERNAL_SPLIT_SIL_FAITHFUL
            .replace("pool_c_balance: prev_states[0].pool_c_balance + y, ", "");
        assert!(
            !mutated.contains("pool_c_balance: prev_states[0].pool_c_balance + y"),
            "mutation must remove the credit field"
        );
        let prog = parse(INTERNAL_SPLIT);
        match validate_translation(&prog, &mutated) {
            Correspondence::Diverges { reasons } => {
                assert!(
                    reasons
                        .iter()
                        .any(|r| r.contains("pool_c_balance") && r.contains("model")),
                    "must name the dropped model value-bearing write: {reasons:?}"
                );
            }
            Correspondence::Corresponds => {
                panic!("a DROPPED model value-bearing write MUST diverge")
            }
        }
    }

    #[test]
    fn model_write_replaced_by_pure_carry_diverges() {
        // ATK7: the .sil replaces the model's debit `pool_a_balance - (x + y)`
        // with a PURE CARRY `pool_a_balance: prev_states[0].pool_a_balance`. The
        // source leg is never debited (money printing). A pure carry on the .sil
        // side is skipped by sil_value_writes, so the only way to catch this is the
        // symmetric model->sil direction. MUST DIVERGE.
        let mutated = INTERNAL_SPLIT_SIL_FAITHFUL.replace(
            "pool_a_balance: prev_states[0].pool_a_balance - (x + y),",
            "pool_a_balance: prev_states[0].pool_a_balance,",
        );
        assert!(
            mutated.contains("pool_a_balance: prev_states[0].pool_a_balance,"),
            "mutation applied"
        );
        let prog = parse(INTERNAL_SPLIT);
        match validate_translation(&prog, &mutated) {
            Correspondence::Diverges { reasons } => {
                assert!(
                    reasons.iter().any(|r| r.contains("pool_a_balance")),
                    "must name pool_a_balance debit replaced by a carry: {reasons:?}"
                );
            }
            Correspondence::Corresponds => {
                panic!("a model debit replaced by a pure carry MUST diverge (money printing)")
            }
        }
    }

    #[test]
    fn duplicate_function_block_diverges() {
        // ATK4: a second same-name `rebalance` block that drops the overdraw guard
        // and mints +999 is appended after the faithful one. The extractor must not
        // silently inspect only the first block and miss the malicious shadow — a
        // duplicate definition is itself a structural anomaly the check must report.
        let shadow = r#"
    function rebalance(State[] prev_states, sig auth, int x, int y) : (State) {
        require(checkSig(auth, prev_states[0].owner));
        return({ pool_a_balance: prev_states[0].pool_a_balance + 999, pool_b_balance: prev_states[0].pool_b_balance, pool_c_balance: prev_states[0].pool_c_balance, owner: prev_states[0].owner });
    }
"#;
        // Insert the shadow just before the contract's closing brace.
        let mutated =
            INTERNAL_SPLIT_SIL_FAITHFUL.replacen("    }\n}", &format!("    }}\n{shadow}}}"), 1);
        assert_eq!(
            mutated.matches("function rebalance(").count(),
            2,
            "two rebalance blocks present"
        );
        let prog = parse(INTERNAL_SPLIT);
        match validate_translation(&prog, &mutated) {
            Correspondence::Diverges { reasons } => {
                assert!(
                    reasons.iter().any(|r| r.contains("rebalance")
                        && (r.contains("multiple") || r.contains("duplicate"))),
                    "must report the duplicate definition: {reasons:?}"
                );
            }
            Correspondence::Corresponds => {
                panic!("a duplicate same-name function block MUST diverge")
            }
        }
    }

    #[test]
    fn whitespace_shadow_function_block_diverges() {
        // DEFECT 1 (sweep): a second same-name `rebalance` block spelled with TWO
        // spaces (`function  rebalance(`) that drops the overdraw guard and mints
        // +999 must NOT evade the duplicate detector / extractor. A non-unique
        // entrypoint (regardless of inter-token whitespace) is a structural anomaly.
        let shadow = "
    function  rebalance(State[] prev_states, sig auth, int x, int y) : (State) {
        require(checkSig(auth, prev_states[0].owner));
        return({ pool_a_balance: prev_states[0].pool_a_balance + 999, pool_b_balance: prev_states[0].pool_b_balance, pool_c_balance: prev_states[0].pool_c_balance, owner: prev_states[0].owner });
    }
";
        let mutated =
            INTERNAL_SPLIT_SIL_FAITHFUL.replacen("    }\n}", &format!("    }}\n{shadow}}}"), 1);
        let prog = parse(INTERNAL_SPLIT);
        match validate_translation(&prog, &mutated) {
            Correspondence::Diverges { reasons } => {
                assert!(
                    reasons.iter().any(|r| r.contains("rebalance")
                        && (r.contains("multiple") || r.contains("duplicate"))),
                    "must report the whitespace-spelled duplicate definition: {reasons:?}"
                );
            }
            Correspondence::Corresponds => {
                panic!("a whitespace-spelled duplicate same-name block MUST diverge")
            }
        }
    }

    #[test]
    fn commented_out_require_does_not_count_as_a_live_guard() {
        // DEFECT 2 (sweep): dropping the LIVE overdraw guard but leaving a
        // commented-out `// require(x + y <= pool_a_balance);` must NOT let the
        // extractor harvest the commented text as a present guard. The model guard is
        // not enforced -> MUST DIVERGE naming the dropped guard.
        let mutated = INTERNAL_SPLIT_SIL_FAITHFUL.replace(
            "        require(x + y <= prev_states[0].pool_a_balance);\n",
            "        // require(x + y <= prev_states[0].pool_a_balance);\n",
        );
        assert!(
            mutated.contains("// require(x + y <="),
            "mutation must leave a commented-out guard"
        );
        let prog = parse(INTERNAL_SPLIT);
        match validate_translation(&prog, &mutated) {
            Correspondence::Diverges { reasons } => {
                assert!(
                    reasons
                        .iter()
                        .any(|r| r.contains("x + y <= pool_a_balance") && r.contains("dropped")),
                    "must name the dropped guard despite the comment: {reasons:?}"
                );
            }
            Correspondence::Corresponds => {
                panic!("a commented-out require MUST NOT count as a live guard (guard dropped)")
            }
        }
    }

    #[test]
    fn missing_function_block_diverges() {
        let prog = parse(INTERNAL_SPLIT);
        let empty = "pragma silverscript ^0.1.0;\ncontract InternalSplit() {}\n";
        match validate_translation(&prog, empty) {
            Correspondence::Diverges { reasons } => {
                assert!(
                    reasons[0].contains("no `function rebalance`"),
                    "{reasons:?}"
                );
            }
            Correspondence::Corresponds => panic!("a missing entrypoint block must diverge"),
        }
    }
}
