//! Composer **M2** front-end lift + per-role covenant skeleton emission.
//!
//! M1 (in `lib.rs`) gave the *type theory*: the [`Score`] grammar, [`project`],
//! and [`Score::check`]. But M1 could only be exercised via the hand-written
//! constructor API (`Global::interact(...)`) and the single worked
//! [`asset_escrow_example`]. The front-end carriers that already exist in the
//! parsed surface — `App.flow` (`Step::Move`/`Choose`/`Par`/`Repeat`) and the
//! cross-role `App.lifecycle` — were dormant: nothing turned a *parsed* program
//! into a Score (`docs/COMPOSER-M0-DESIGN.md §1`, "channels is never populated").
//!
//! M2 closes that gap with two pieces:
//!
//! 1. [`lift`] — `&App → Result<Score, LiftError>`, mapping the surface flow into
//!    the global-type grammar per the design doc §2.4:
//!    `Step::Move → Interact`, `Step::Choose → Branching`, `Step::Par → ∥`,
//!    `Step::Repeat(n,body) → μX. (body . X)`. When no `flow {}` block is present
//!    (the case for *every* current library program — `flow` is comment-only),
//!    the lift falls back to the **lifecycle** edges, which ARE the real parsed
//!    multi-role carrier (e.g. `DigitalReit`: `token.distribute → splitter.payout`).
//!
//! 2. [`emit_role_skeletons`] — from the projected per-role [`Local`] types, a
//!    structured per-role covenant **skeleton** (entrypoints + message/resource
//!    handoffs). These are clearly-labelled SKELETONS, **not** deployable `.sil`.
//!
//! # Faithfulness is the load-bearing claim
//!
//! The whole point of a Composer accept is that the Score it checked is a
//! FAITHFUL image of the program. A lift that quietly produced a *different*,
//! safe-looking Score for an unsafe program would be the worst possible failure
//! (`docs/COMPOSER-M0-DESIGN.md §4.4`). So the lift is conservative: every flow
//! construct maps to its designated global-type construct, and any construct that
//! cannot be faithfully represented returns a **named** [`LiftError`] — it never
//! emits a silently-wrong Score. A lifted Score is then handed to the *same*
//! `check()` as M1; this module adds no new "safety" reasoning of its own.

use crate::{Global, Local, Resource, Role, Score, Sort};
use portrait_syntax::{App, Edge, Flow, Step};
use std::collections::BTreeMap;

/// The continuation resource name every Move-handoff carries. A `Continuation`
/// (§2.3) — the right/obligation to take the next protocol step, realised
/// on-chain as the covenant-ID handoff (`parent_kov_id`). Threading it makes the
/// lifted Score's linearity check meaningful: the step token is produced once and
/// handed actor-to-actor down the flow.
pub const STEP_RESOURCE: &str = "step";

/// A named reason a parsed program could **not** be faithfully lifted to a Score.
/// The lift returns one of these rather than fabricating an interaction — a
/// wrong-but-checkable Score is the failure mode this guards against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiftError {
    /// The program has neither a `flow {}` block nor any lifecycle edges, so
    /// there is no protocol flow to lift.
    NoFlow,
    /// The program declares fewer than two roles. A single-role "protocol" has no
    /// inter-role interaction to model; fabricating a counterparty would be an
    /// unfaithful lift, so the lift refuses.
    SingleRole {
        /// The only role found (or `"<none>"`).
        role: String,
    },
    /// A flow step is performed by a role not declared in `App.roles`.
    UnknownRole {
        /// The undeclared acting role named by a `Step::Move`.
        role: String,
        /// The entry it tried to fire.
        entry: String,
    },
    /// A terminal `Step::Move` resolves to a receiver equal to its actor (a
    /// self-interaction `p → p`), which the Score grammar forbids (§2.2) and
    /// which cannot be faithfully represented.
    SelfInteractionStep {
        /// The offending actor role.
        role: String,
        /// The entry at which the self-interaction would occur.
        entry: String,
    },
    /// A `Step::Choose` branch is empty (no leading move), so the branch carries
    /// no distinguishing label — it cannot be faithfully lifted to a `Branching`
    /// label.
    EmptyChoiceBranch,
    /// A `Step::Choose` has fewer than two branches; a choice with one (or zero)
    /// alternatives is not a branching.
    DegenerateChoice {
        /// How many branches were present.
        count: usize,
    },
    /// A `Step::Repeat` body is empty, so there is nothing to recurse over.
    EmptyRepeatBody,
    /// A `Step::Repeat` body contains nested control flow (`choose`/`par`/nested
    /// `repeat`). M2 lifts only a Move-only repeat body; rather than silently
    /// mis-lifting the nested construct, the lift refuses with this named error.
    NestedControlInRepeat,
    /// One or more flow steps are sequenced *after* a construct that lifts to an
    /// **unbounded** recursion (`μX. body . X`). That recursion never reaches an
    /// `End` leaf, so there is nowhere to faithfully attach the following steps —
    /// the tail would be *unreachable* in the model. M2 models `repeat(n)` as an
    /// unbounded loop (an honest simplification), so rather than silently DROP the
    /// trailing steps (which would lift the program to a *different*, truncated
    /// Score — the worst-possible unfaithfulness, `docs/COMPOSER-M0-DESIGN.md
    /// §4.4`), the lift refuses with this named error. Re-order the program so the
    /// repeat is last, or model the post-loop work inside the loop body.
    TrailingStepsAfterUnboundedRepeat,
}

impl std::fmt::Display for LiftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiftError::NoFlow => write!(
                f,
                "no flow to lift: the program has neither a `flow {{}}` block nor lifecycle edges"
            ),
            LiftError::SingleRole { role } => write!(
                f,
                "single-role program (`{role}`): no inter-role interaction to model faithfully"
            ),
            LiftError::UnknownRole { role, entry } => write!(
                f,
                "flow step `{role}.{entry}` names a role not declared in the app"
            ),
            LiftError::SelfInteractionStep { role, entry } => write!(
                f,
                "flow step `{role}.{entry}` would lift to a self-interaction `{role} → {role}`"
            ),
            LiftError::EmptyChoiceBranch => {
                write!(f, "a `choose` branch is empty (no leading move to label)")
            }
            LiftError::DegenerateChoice { count } => {
                write!(
                    f,
                    "a `choose` has {count} branch(es); a branching needs >= 2"
                )
            }
            LiftError::EmptyRepeatBody => {
                write!(f, "a `repeat` body is empty (nothing to recurse)")
            }
            LiftError::NestedControlInRepeat => write!(
                f,
                "a `repeat` body contains nested control flow (choose/par/repeat); \
                 M2 lifts only a Move-only repeat body"
            ),
            LiftError::TrailingStepsAfterUnboundedRepeat => write!(
                f,
                "flow steps are sequenced after a `repeat`, which M2 models as an \
                 unbounded loop (`μX. body . X`) with no exit; the trailing steps \
                 would be unreachable, so the lift refuses rather than dropping them"
            ),
        }
    }
}

impl std::error::Error for LiftError {}

/// Lift a parsed Portrait [`App`] into a [`Score`] (M2, design doc §2.4).
///
/// Prefers an explicit `flow {}` block; when absent (every current library
/// program), synthesises a linear flow from the cross-role `lifecycle` edges in
/// declaration order — the real parsed multi-role carrier.
///
/// FAITHFULNESS: each flow construct maps to its designated global-type construct
/// and nothing else; an un-representable construct returns a named [`LiftError`].
/// The result is *not* pre-validated here — call [`Score::check`] on it (that is
/// the M1 safety check, unchanged).
pub fn lift(app: &App) -> Result<Score, LiftError> {
    let roles: Vec<Role> = app.roles.iter().map(|r| r.name.clone()).collect();
    if roles.len() < 2 {
        return Err(LiftError::SingleRole {
            role: roles
                .first()
                .cloned()
                .unwrap_or_else(|| "<none>".to_string()),
        });
    }

    // Source flow: explicit block, else lifecycle-derived.
    let steps: Vec<Step> = match &app.flow {
        Some(flow) => flow.steps.clone(),
        None => lifecycle_to_steps(&app.lifecycle),
    };
    if steps.is_empty() {
        return Err(LiftError::NoFlow);
    }

    let declared: std::collections::BTreeSet<&str> = roles.iter().map(|s| s.as_str()).collect();

    // A fresh recursion-variable counter so nested `repeat`s get distinct vars.
    let mut rec_counter = 0usize;
    let global = lift_steps(&steps, None, &declared, &roles, &mut rec_counter)?;

    Score::new(
        &roles.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec![Resource::new(STEP_RESOURCE, Sort::Continuation)],
        global,
    )
    .pipe_ok()
}

/// Map lifecycle edges to a linear `Move` flow: each `Edge { via_role, via_entry }`
/// is the actor `via_role` firing entry `via_entry`. Declaration order is the
/// flow order — exactly the cross-role sequence the lifecycle encodes.
fn lifecycle_to_steps(lifecycle: &[Edge]) -> Vec<Step> {
    lifecycle
        .iter()
        .map(|e| Step::Move {
            role: e.via_role.clone(),
            entry: e.via_entry.clone(),
        })
        .collect()
}

/// Lift a flat step sequence into a `Global`, threading `successor` — the role
/// that acts immediately *after* this sequence (the continuation receiver for the
/// last step). `None` means end-of-flow.
fn lift_steps(
    steps: &[Step],
    successor: Option<&Role>,
    declared: &std::collections::BTreeSet<&str>,
    roles: &[Role],
    rec_counter: &mut usize,
) -> Result<Global, LiftError> {
    let (head, rest) = match steps.split_first() {
        Some(pair) => pair,
        None => return Ok(Global::End),
    };

    // The role acting at the *start* of `rest` (the successor for `head`), if any.
    let next_actor = first_actor(rest, successor);

    match head {
        Step::Move { role, entry } => {
            if !declared.contains(role.as_str()) {
                return Err(LiftError::UnknownRole {
                    role: role.clone(),
                    entry: entry.clone(),
                });
            }
            // The continuation receiver is the next actor; for a terminal move
            // with no successor, pick a declared counterparty distinct from the
            // actor (the role the step hands off to before the protocol ends).
            let to = match &next_actor {
                Some(r) => r.clone(),
                None => {
                    counterparty(role, roles).ok_or_else(|| LiftError::SelfInteractionStep {
                        role: role.clone(),
                        entry: entry.clone(),
                    })?
                }
            };
            if to == *role {
                return Err(LiftError::SelfInteractionStep {
                    role: role.clone(),
                    entry: entry.clone(),
                });
            }
            let cont = lift_steps(rest, successor, declared, roles, rec_counter)?;
            Ok(Global::Interact {
                from: role.clone(),
                to,
                message: entry.clone(),
                resources: vec![STEP_RESOURCE.to_string()],
                cont: Box::new(cont),
            })
        }
        Step::Choose(branches) => {
            if branches.len() < 2 {
                return Err(LiftError::DegenerateChoice {
                    count: branches.len(),
                });
            }
            // The decider is the role that acted just before the choice. We don't
            // have it lexically here (it is the previous step's actor), so we use
            // the first actor *inside* the branches as the decider — i.e. the
            // choice is decided by whoever leads the branches. The informed role
            // is the successor (who continues after the choice). This is the
            // faithful reading of `Step::Choose` as an internal choice at the
            // branch-leading role.
            let decider = branch_decider(branches)?;
            let informed = next_actor
                .clone()
                .or_else(|| counterparty(&decider, roles))
                .ok_or(LiftError::EmptyChoiceBranch)?;
            let mut lifted = Vec::new();
            for branch in branches {
                let label = branch_label(branch)?;
                let g = lift_steps(&branch.steps, successor, declared, roles, rec_counter)?;
                lifted.push((label, g));
            }
            // Continuation after the whole choice (shared tail).
            // Note: in the surface grammar each branch is self-contained; the
            // post-choice steps `rest` continue after EACH branch. We append the
            // shared tail into each branch so the global type is well-formed.
            let tail = lift_steps(rest, successor, declared, roles, rec_counter)?;
            let mut branches_with_tail = Vec::with_capacity(lifted.len());
            for (l, g) in lifted {
                branches_with_tail.push((l, append_global(g, tail.clone())?));
            }
            Ok(Global::Branching {
                decider,
                informed,
                branches: branches_with_tail,
            })
        }
        Step::Par(flows) => {
            // Fold the parallel flows into nested `Par`. Each flow lifts
            // independently (disjoint roles/resources is checked by `check()`).
            let mut parts = Vec::new();
            for flow in flows {
                parts.push(lift_steps(&flow.steps, None, declared, roles, rec_counter)?);
            }
            let par = fold_par(parts);
            // Sequence the post-par tail after the parallel block.
            let tail = lift_steps(rest, successor, declared, roles, rec_counter)?;
            append_global(par, tail)
        }
        Step::Repeat(_n, body) => {
            if body.steps.is_empty() {
                return Err(LiftError::EmptyRepeatBody);
            }
            // μX. (body . X) — guarded bounded recursion. The `n` bound is the
            // finite-covenant guarantee already carried in the syntax; the
            // checker's guardedness condition is what we rely on here, so we lift
            // to a single guarded `Rec` whose body loops back to `X`.
            let var = format!("X{}", *rec_counter);
            *rec_counter += 1;
            // Inside the body, the successor of the last step is the loop variable
            // holder — model the loop-back by ending the body with `Var`.
            let body_global = lift_steps_looped(&body.steps, &var, declared, roles)?;
            let rec = Global::Rec(var, Box::new(body_global));
            let tail = lift_steps(rest, successor, declared, roles, rec_counter)?;
            append_global(rec, tail)
        }
    }
}

/// Lift a `repeat` body so its last step's continuation is the loop variable
/// `var` (modelling `μX. body . X`).
fn lift_steps_looped(
    steps: &[Step],
    var: &str,
    declared: &std::collections::BTreeSet<&str>,
    roles: &[Role],
) -> Result<Global, LiftError> {
    let (head, rest) = match steps.split_first() {
        Some(pair) => pair,
        None => return Ok(Global::Var(var.to_string())),
    };
    match head {
        Step::Move { role, entry } => {
            if !declared.contains(role.as_str()) {
                return Err(LiftError::UnknownRole {
                    role: role.clone(),
                    entry: entry.clone(),
                });
            }
            let next_actor = first_actor(rest, None);
            let to = match next_actor {
                Some(r) => r,
                None => {
                    counterparty(role, roles).ok_or_else(|| LiftError::SelfInteractionStep {
                        role: role.clone(),
                        entry: entry.clone(),
                    })?
                }
            };
            if to == *role {
                return Err(LiftError::SelfInteractionStep {
                    role: role.clone(),
                    entry: entry.clone(),
                });
            }
            let cont = lift_steps_looped(rest, var, declared, roles)?;
            Ok(Global::Interact {
                from: role.clone(),
                to,
                message: entry.clone(),
                resources: vec![STEP_RESOURCE.to_string()],
                cont: Box::new(cont),
            })
        }
        // Nested control flow inside a repeat body is not lifted in M2 (kept
        // conservative); a Move-only body is the supported shape. Refuse with a
        // named error rather than mis-lifting.
        _ => Err(LiftError::NestedControlInRepeat),
    }
}

/// The first acting role of a step sequence, falling back to `default` when the
/// sequence is empty or leads with a non-Move construct whose actor we resolve
/// recursively.
fn first_actor(steps: &[Step], default: Option<&Role>) -> Option<Role> {
    for step in steps {
        match step {
            Step::Move { role, .. } => return Some(role.clone()),
            Step::Choose(branches) => {
                for b in branches {
                    if let Some(r) = first_actor(&b.steps, None) {
                        return Some(r);
                    }
                }
            }
            Step::Par(flows) => {
                for fl in flows {
                    if let Some(r) = first_actor(&fl.steps, None) {
                        return Some(r);
                    }
                }
            }
            Step::Repeat(_, body) => {
                if let Some(r) = first_actor(&body.steps, None) {
                    return Some(r);
                }
            }
        }
    }
    default.cloned()
}

/// The deciding role of a `Step::Choose`: the role leading the first branch.
fn branch_decider(branches: &[Flow]) -> Result<Role, LiftError> {
    for b in branches {
        if let Some(r) = first_actor(&b.steps, None) {
            return Ok(r);
        }
    }
    Err(LiftError::EmptyChoiceBranch)
}

/// The branch label of a `choose` branch: the entry name of its leading move.
fn branch_label(flow: &Flow) -> Result<String, LiftError> {
    match flow.steps.first() {
        Some(Step::Move { entry, .. }) => Ok(entry.clone()),
        _ => Err(LiftError::EmptyChoiceBranch),
    }
}

/// A declared role distinct from `role` (a counterparty for a terminal handoff).
fn counterparty(role: &str, roles: &[Role]) -> Option<Role> {
    roles.iter().find(|r| r.as_str() != role).cloned()
}

/// Replace every `End` leaf of `g` with `tail` (sequential composition). Used to
/// thread the post-construct continuation after a `Branching`/`Par`/`Rec`.
///
/// FAITHFULNESS: a `Rec`/`Var` leaf is an **unbounded** recursion with no `End`
/// to replace — so a non-trivial `tail` cannot be attached there. Rather than
/// silently DROP the tail (which would yield a Score for a *different*, truncated
/// program — `docs/COMPOSER-M0-DESIGN.md §4.4`), this returns
/// [`LiftError::TrailingStepsAfterUnboundedRepeat`]. An `End` tail (nothing to
/// sequence) is always fine.
fn append_global(g: Global, tail: Global) -> Result<Global, LiftError> {
    match g {
        Global::End => Ok(tail),
        Global::Interact {
            from,
            to,
            message,
            resources,
            cont,
        } => Ok(Global::Interact {
            from,
            to,
            message,
            resources,
            cont: Box::new(append_global(*cont, tail)?),
        }),
        Global::Branching {
            decider,
            informed,
            branches,
        } => {
            let mut lifted = Vec::with_capacity(branches.len());
            for (l, b) in branches {
                lifted.push((l, append_global(b, tail.clone())?));
            }
            Ok(Global::Branching {
                decider,
                informed,
                branches: lifted,
            })
        }
        Global::Par(a, b) => {
            // Append to the second side (the first finishes into the second).
            Ok(Global::Par(a, Box::new(append_global(*b, tail)?)))
        }
        // A recursion boundary or variable has no `End` leaf to replace. Appending
        // an empty tail is a no-op (fine); appending real steps would silently
        // truncate the program, so refuse with a named error.
        g @ (Global::Rec(..) | Global::Var(_)) => {
            if matches!(tail, Global::End) {
                Ok(g)
            } else {
                Err(LiftError::TrailingStepsAfterUnboundedRepeat)
            }
        }
    }
}

/// Fold a list of globals into right-nested `Par`. Empty → `End`; one → itself.
fn fold_par(mut parts: Vec<Global>) -> Global {
    match parts.len() {
        0 => Global::End,
        1 => parts.pop().unwrap(),
        _ => {
            let first = parts.remove(0);
            Global::Par(Box::new(first), Box::new(fold_par(parts)))
        }
    }
}

// Small helper so `lift` reads as a pipeline; `Score` has no error path of its
// own, so this is infallible but keeps the `?`-free tail tidy.
trait PipeOk: Sized {
    fn pipe_ok(self) -> Result<Self, LiftError> {
        Ok(self)
    }
}
impl PipeOk for Score {}

// ===== per-role covenant skeleton emission ===================================

/// A per-role covenant **skeleton** — a STRUCTURAL summary of one role's view,
/// derived from its projected [`Local`] type. NOT deployable silverscript: it
/// records entrypoints (sends/selects this role authorises) and awaited messages
/// (receives/offers), with the resource handoffs, so the cross-role wiring is
/// legible. Emission of real `.sil` bodies (value conservation) is Lens's job and
/// is explicitly out of scope (`docs/COMPOSER-M0-DESIGN.md §0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSkeleton {
    /// The role this skeleton is for.
    pub role: Role,
    /// Entrypoints this role authorises (sends / internal-choice selections),
    /// each with the peer it targets and the resources handed off.
    pub entrypoints: Vec<SkelEntry>,
    /// Messages this role awaits (receives / external-choice offers), each with
    /// the peer it awaits from and the resources received.
    pub awaits: Vec<SkelEntry>,
}

/// One entrypoint or awaited message in a [`RoleSkeleton`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkelEntry {
    /// The message / branch label.
    pub message: String,
    /// The peer role (target of a send, source of a receive).
    pub peer: Role,
    /// Resources carried by this handoff.
    pub resources: Vec<String>,
}

/// Emit per-role covenant skeletons from a checked projection (the `BTreeMap`
/// returned by [`Score::check`]). One [`RoleSkeleton`] per role, in role order.
pub fn emit_role_skeletons(locals: &BTreeMap<Role, Local>) -> Vec<RoleSkeleton> {
    locals
        .iter()
        .map(|(role, local)| {
            let mut entrypoints = Vec::new();
            let mut awaits = Vec::new();
            walk_local(local, &mut entrypoints, &mut awaits);
            RoleSkeleton {
                role: role.clone(),
                entrypoints,
                awaits,
            }
        })
        .collect()
}

fn walk_local(t: &Local, entrypoints: &mut Vec<SkelEntry>, awaits: &mut Vec<SkelEntry>) {
    match t {
        Local::Send {
            to,
            message,
            resources,
            cont,
        } => {
            entrypoints.push(SkelEntry {
                message: message.clone(),
                peer: to.clone(),
                resources: resources.clone(),
            });
            walk_local(cont, entrypoints, awaits);
        }
        Local::Recv {
            from,
            message,
            resources,
            cont,
        } => {
            awaits.push(SkelEntry {
                message: message.clone(),
                peer: from.clone(),
                resources: resources.clone(),
            });
            walk_local(cont, entrypoints, awaits);
        }
        Local::Select { to, branches } => {
            for (label, sub) in branches {
                entrypoints.push(SkelEntry {
                    message: label.clone(),
                    peer: to.clone(),
                    resources: Vec::new(),
                });
                walk_local(sub, entrypoints, awaits);
            }
        }
        Local::Offer { from, branches } => {
            for (label, sub) in branches {
                awaits.push(SkelEntry {
                    message: label.clone(),
                    peer: from.clone(),
                    resources: Vec::new(),
                });
                walk_local(sub, entrypoints, awaits);
            }
        }
        Local::Par(a, b) => {
            walk_local(a, entrypoints, awaits);
            walk_local(b, entrypoints, awaits);
        }
        Local::Rec(_, body) => walk_local(body, entrypoints, awaits),
        Local::Var(_) | Local::End => {}
    }
}

/// Render the per-role skeletons to a clearly-labelled, human-readable block.
/// The leading banner makes the SKELETON / not-deployable status unmissable.
pub fn render_skeletons(skeletons: &[RoleSkeleton]) -> String {
    let mut out = String::new();
    out.push_str("--- per-role covenant SKELETONS (structural, NOT deployable .sil) ---\n");
    for s in skeletons {
        out.push_str(&format!("\ncovenant_skeleton {} {{\n", s.role));
        for e in &s.entrypoints {
            out.push_str(&format!(
                "  entrypoint {}  ->{}  {}\n",
                e.message,
                e.peer,
                render_res(&e.resources)
            ));
        }
        for a in &s.awaits {
            out.push_str(&format!(
                "  awaits     {}  <-{}  {}\n",
                a.message,
                a.peer,
                render_res(&a.resources)
            ));
        }
        out.push_str("}\n");
    }
    out
}

fn render_res(resources: &[String]) -> String {
    if resources.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", resources.join(", "))
    }
}

#[cfg(test)]
mod tests;
