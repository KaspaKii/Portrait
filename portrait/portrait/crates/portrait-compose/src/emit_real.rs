//! Composer **M5** real per-role covenant emission.
//!
//! M2 ([`crate::lift::emit_role_skeletons`]) emitted per-role covenant
//! *skeletons* — a structural summary (entrypoints + awaits) rendered to a
//! clearly-labelled, **non-parseable** block. M3 attached realization data. M5
//! closes the honesty gap M2 left open: it emits, from a checked projection, a
//! per-role covenant **TEXT** that genuinely round-trips —
//! [`portrait_syntax::parse`] returns `Ok` AND `portrait_sema::check` passes.
//!
//! # What "real" means here — stated precisely (and its limit)
//!
//! For each role, the role's **authorised entrypoints** (the `Send` / `Select`
//! labels in its projected [`Local`] — i.e. the messages it may originate) are
//! encoded as real `#[covenant(mode = transition)]` entrypoint declarations, each
//! with a matching `lifecycle` edge and a state-carrying `return`. That text is a
//! genuine Portrait covenant source: it parses and structurally checks.
//!
//! **The faithful subset, and the recorded gap.** The Portrait covenant grammar
//! is *single-app, role-local*: an entrypoint body cannot reference another
//! role's covenant, and the cross-role message *handoff* (who awaits this send,
//! the `Continuation` transfer) has no surface form in a single covenant. So the
//! emitted covenant faithfully encodes **one role's own entrypoints** as checkable
//! declarations; the cross-role wiring (awaited messages, peers, the handoff
//! chain) is recorded as covenant **comments**, not as fabricated declarations. We
//! emit the largest faithful, parse-and-check-clean subset and name what is not
//! expressible — we never emit something that fails to parse/check and call it
//! real. A role with **no** authorised entrypoints (a pure receiver) emits a valid
//! covenant with an empty role body and no lifecycle (the honest minimal subset).
//!
//! This is STILL not a deployed covenant: value-conservation bodies are Lens's job
//! and out of scope; the emitted `step` carry is a structural placeholder, not an
//! economic body. Emission proves the text is a well-formed, checkable covenant —
//! nothing on-chain.

use crate::lift::{emit_role_skeletons, SkelEntry};
use crate::{Local, Role};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// The state field every emitted covenant carries: an `int step` — the structural
/// image of the `Continuation` handoff token (§2.3), as a non-value-bearing field
/// (so it never trips the `value_conserved` C1/C2 checks; emission stays minimal
/// and does not claim an economic body).
const STEP_FIELD: &str = "step";

/// Safe placeholder identifier used for the `app`/`role` surface names when the
/// real role name is not a valid identifier ([`EmitGap::NonIdentRole`]). The real
/// name is preserved on [`RealCovenant::role`] and in the recorded gap; only the
/// emitted surface text uses this placeholder so the covenant still parses.
const PLACEHOLDER_ROLE: &str = "UnnamedRole";

/// A reserved Portrait surface keyword. An authorised message whose label collides
/// with one cannot be emitted as an entrypoint name (it would mis-parse), so it is
/// recorded as a [`EmitGap`] instead of producing broken text.
const RESERVED: &[&str] = &[
    "pragma",
    "use",
    "app",
    "role",
    "param",
    "state",
    "entrypoint",
    "function",
    "lifecycle",
    "via",
    "terminal",
    "invariant",
    "requires",
    "require",
    "return",
    "covenant",
    "mode",
    "transition",
    "verification",
    "true",
    "false",
    "int",
    "bool",
    "pubkey",
    "sig",
    "bytes32",
    "coin",
    "set",
    "map",
    "choose",
    "par",
    "repeat",
    "branch",
    "thread",
];

/// A construct in a role's local type that could **not** be faithfully emitted as
/// a checkable covenant declaration, recorded honestly rather than emitted as
/// broken text (the M5 honesty contract). The emitted covenant still parses and
/// checks; the gap names what was *omitted* from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmitGap {
    /// An authorised message label is a reserved surface keyword, so it cannot be
    /// an entrypoint name; the entrypoint was omitted from the emitted covenant.
    ReservedLabel {
        /// The role whose covenant omits it.
        role: Role,
        /// The offending label.
        label: String,
    },
    /// An authorised message label is not a valid identifier (empty, or starts
    /// with a non-alphabetic / contains a non-alphanumeric char), so it cannot be
    /// an entrypoint name; the entrypoint was omitted.
    NonIdentLabel {
        /// The role whose covenant omits it.
        role: Role,
        /// The offending label.
        label: String,
    },
    /// The role name itself is not a valid identifier, so it cannot be spliced
    /// into `app`/`role`/`return`/`lifecycle` surface forms. The covenant is
    /// emitted under a safe placeholder identifier with NO entrypoints or
    /// lifecycle (the honest minimal subset that still parses + checks), and the
    /// real role name is recorded here.
    NonIdentRole {
        /// The offending role name.
        role: Role,
    },
}

impl std::fmt::Display for EmitGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitGap::ReservedLabel { role, label } => write!(
                f,
                "role `{role}`: authorised message `{label}` is a reserved keyword and \
                 cannot be emitted as an entrypoint; recorded as a gap"
            ),
            EmitGap::NonIdentLabel { role, label } => write!(
                f,
                "role `{role}`: authorised message `{label}` is not a valid identifier and \
                 cannot be emitted as an entrypoint; recorded as a gap"
            ),
            EmitGap::NonIdentRole { role } => write!(
                f,
                "role `{role}`: the role name is not a valid identifier and cannot be \
                 spliced into surface forms; emitted under a placeholder with no \
                 entrypoints/lifecycle; recorded as a gap"
            ),
        }
    }
}

/// A real, parse-and-check-clean per-role covenant emitted by M5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealCovenant {
    /// The role this covenant is for.
    pub role: Role,
    /// The covenant source TEXT. Guaranteed (by the M5 tests) to satisfy
    /// `portrait_syntax::parse(&source).is_ok()` AND
    /// `portrait_sema::check(&parsed).is_ok()`.
    pub source: String,
    /// Constructs that could not be faithfully emitted as checkable declarations
    /// and were recorded as comments instead of fabricated (the honesty contract).
    /// Empty for a covenant that emitted every authorised entrypoint.
    pub gaps: Vec<EmitGap>,
}

/// Emit one real, parse-and-check-clean covenant per role from a checked
/// projection (the `BTreeMap` returned by [`crate::Score::check`]).
///
/// Each role's authorised entrypoints (its `Send`/`Select` labels) become real
/// `transition` entrypoint declarations with lifecycle edges; awaited messages and
/// peers are recorded as comments (the faithful single-role subset). Any label
/// that cannot be a valid entrypoint identifier is recorded as an [`EmitGap`]
/// rather than emitted as broken text.
pub fn emit_real_covenants(locals: &BTreeMap<Role, Local>) -> Vec<RealCovenant> {
    let skeletons = emit_role_skeletons(locals);
    skeletons
        .into_iter()
        .map(|sk| emit_one(&sk.role, &sk.entrypoints, &sk.awaits))
        .collect()
}

/// Emit one covenant for `role` from its authorised `entrypoints` and awaited
/// messages. De-duplicates labels (a label authorised on several branches is one
/// entrypoint), and records reserved/invalid labels as gaps.
fn emit_one(role: &Role, entrypoints: &[SkelEntry], awaits: &[SkelEntry]) -> RealCovenant {
    let mut gaps = Vec::new();

    // The role name is spliced verbatim into `app {role}` / `role {role}` /
    // `return {role}` / `lifecycle ... via {role}.{msg}`. If it is not a valid
    // identifier we CANNOT emit any of those and have them parse, so we refuse
    // to claim the entrypoints/lifecycle round-trip: emit under a safe
    // placeholder identifier with no entrypoints and no lifecycle (still a
    // parse+check-clean covenant), and record the real role name as a gap.
    if !is_ident(role) {
        gaps.push(EmitGap::NonIdentRole { role: role.clone() });
        let source = render_covenant(PLACEHOLDER_ROLE, &[], awaits, &gaps);
        return RealCovenant {
            role: role.clone(),
            source,
            gaps,
        };
    }

    let mut emitted: Vec<&SkelEntry> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for e in entrypoints {
        if !seen.insert(e.message.as_str()) {
            continue; // same label across branches → one entrypoint
        }
        if !is_ident(&e.message) {
            gaps.push(EmitGap::NonIdentLabel {
                role: role.clone(),
                label: e.message.clone(),
            });
            continue;
        }
        if RESERVED.contains(&e.message.as_str()) {
            gaps.push(EmitGap::ReservedLabel {
                role: role.clone(),
                label: e.message.clone(),
            });
            continue;
        }
        emitted.push(e);
    }

    let source = render_covenant(role, &emitted, awaits, &gaps);
    RealCovenant {
        role: role.clone(),
        source,
        gaps,
    }
}

/// Whether `s` is a valid Portrait identifier (ASCII-alpha or `_` lead, then
/// ASCII-alphanumeric or `_`), matching the tokenizer in `portrait-syntax`.
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Render the covenant source for `role`. App name is the role; the single role
/// carries an `int step` state field and one transition entrypoint per authorised
/// message. Awaited messages, peers and gaps are recorded as comments.
fn render_covenant(
    role: &str,
    entrypoints: &[&SkelEntry],
    awaits: &[SkelEntry],
    gaps: &[EmitGap],
) -> String {
    let mut s = String::new();
    s.push_str("// M5 real per-role covenant — parses (portrait_syntax::parse) and\n");
    s.push_str("// sema-checks (portrait_sema::check). The faithful single-role subset:\n");
    s.push_str("// this role's authorised entrypoints are REAL transition declarations;\n");
    s.push_str("// cross-role awaits/peers/handoff are recorded as comments (not\n");
    s.push_str("// fabricated). NOT a deployed covenant; `step` is a structural carry,\n");
    s.push_str("// not an economic body (value conservation is out of scope — Lens's job).\n");
    s.push_str("pragma portrait ^0.1.0;\n\n");
    s.push_str(&format!("app {role} {{\n"));
    s.push_str(&format!("  role {role} {{\n"));
    s.push_str(&format!("    state {{ int {STEP_FIELD}; }}\n"));

    for e in entrypoints {
        s.push_str(&format!(
            "\n    // authorises `{}` -> {} {}\n",
            e.message,
            e.peer,
            render_res(&e.resources)
        ));
        s.push_str("    #[covenant(mode = transition)]\n");
        s.push_str(&format!(
            "    entrypoint function {}(int next) : (int {STEP_FIELD}) {{\n",
            e.message
        ));
        s.push_str(&format!("      return {role} {{ {STEP_FIELD}: next }};\n"));
        s.push_str("    }\n");
    }
    s.push_str("  }\n");

    // Cross-role wiring recorded as comments (no surface form in a single covenant).
    if !awaits.is_empty() {
        s.push_str("\n  // awaited messages (cross-role handoff — recorded, not emitted):\n");
        for a in awaits {
            s.push_str(&format!(
                "  //   awaits `{}` <- {} {}\n",
                a.message,
                a.peer,
                render_res(&a.resources)
            ));
        }
    }
    if !gaps.is_empty() {
        s.push_str("\n  // recorded gaps (not faithfully emittable as declarations):\n");
        for g in gaps {
            s.push_str(&format!("  //   {g}\n"));
        }
    }

    // lifecycle: one self-edge per emitted entrypoint (each transition is reached
    // by a non-terminal edge, satisfying the transition/return sema rule).
    if entrypoints.is_empty() {
        s.push_str("\n  // no authorised entrypoints (pure receiver): no lifecycle edges.\n");
    } else {
        s.push_str("\n  lifecycle {\n");
        for e in entrypoints {
            s.push_str(&format!("    live -> live via {role}.{};\n", e.message));
        }
        s.push_str("  }\n");
    }

    // `no_undeclared_state` only: declaring `value_conserved` would pull in the
    // C1/C2 conservation+authorization checks, which the minimal `step` carry is
    // not trying to satisfy (and would force fabricated checkSig bodies). The
    // emitted covenant claims structural well-formedness, nothing economic.
    s.push_str("\n  invariant no_undeclared_state;\n");
    s.push_str("}\n");
    s
}

fn render_res(resources: &[String]) -> String {
    if resources.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", resources.join(", "))
    }
}

/// Render all emitted real covenants to a single clearly-labelled block, with the
/// honest banner. For the harness/report.
pub fn render_real_covenants(covenants: &[RealCovenant]) -> String {
    let mut out = String::new();
    out.push_str("=== M5 real per-role covenants (parse + sema-check clean, NOT deployed) ===\n");
    for c in covenants {
        out.push_str(&format!("\n----- covenant for role `{}` -----\n", c.role));
        out.push_str(&c.source);
        if !c.gaps.is_empty() {
            out.push_str(&format!(
                "  [{} recorded gap(s) — see covenant comments]\n",
                c.gaps.len()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests;
