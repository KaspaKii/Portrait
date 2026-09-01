//! Composer **M5** local executor — an in-memory SIMULATION of the [`Score`]
//! model under cooperative scheduling.
//!
//! # HONESTY — what this is, and (loudly) what it is NOT
//!
//! This drives a [`Score`] through its interactions *in the model*: it delivers
//! messages in a valid order, moves the linear resources per the local-type
//! handoff rules (a `Continuation` transfers to the receiver; an escrowed
//! `Value`/`Capability` stays under the sending holder — §2.3), follows
//! `Branching` / `Par` / `Rec`, and produces a [`Trace`] plus a terminal
//! [`Status`].
//!
//! It is **NOT a chain runtime.** It executes **no transaction**, reads **no
//! chain**, builds **no UTXO**, and verifies **no covenant**. A successful run
//! (`Status::Completed`) is a statement about the *protocol model* only — that the
//! interactions can be scheduled to completion with a connected linear handoff. It
//! does **NOT** imply on-chain liveness: on a permissionless ledger a counterparty
//! can simply never spend the UTXO it was handed, and the only honest remedy is
//! the relative-timelock escape (the M3 NOT-STRANDED-BEYOND-`T` boundary), which
//! still governs the permissionless reality. This executor assumes cooperative
//! participation — exactly the assumption the ledger does not enforce.
//!
//! `Status::Stuck` is the executor's dynamic image of a broken handoff / deadlock:
//! an interaction authorised by a role that is **not** the current holder of a
//! live resource it carries cannot fire (two parties would claim the same linear
//! token), so the schedule wedges. A well-formed protocol never reaches `Stuck`.

use crate::{Global, Role, Score, Sort};
use std::collections::BTreeMap;

/// One executed interaction (a delivered message) in the simulation [`Trace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The authorising sender that fired this interaction.
    pub from: Role,
    /// The receiver the message was delivered to.
    pub to: Role,
    /// The message label.
    pub message: String,
    /// The resources moved by this step.
    pub resources: Vec<String>,
}

/// The terminal outcome of an [`execute`] run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The schedule reached `end` with every fired interaction authorised by the
    /// current holder of every live resource it carried — the model completes.
    Completed,
    /// The schedule wedged: an interaction could not fire because its authorising
    /// sender is **not** the current holder of a live resource it carries (a
    /// broken handoff / deadlock in the model). Names where it stuck.
    Stuck {
        /// Human-readable description of the wedged step.
        where_: String,
    },
    /// Execution unfolded a recursion up to the bound without reaching `end`; the
    /// loop is well-formed but does not terminate in the model (it is bounded here
    /// to keep the simulation finite). NOT a failure — a recursive protocol.
    LoopBounded,
}

/// The result of an [`execute`] run: the ordered [`Event`] trace and the terminal
/// [`Status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// The interactions, in the order the simulation delivered them.
    pub events: Vec<Event>,
    /// The terminal status.
    pub status: Status,
}

/// How many times a single recursion is unfolded before the simulation declares
/// [`Status::LoopBounded`]. Keeps the in-memory run finite (the model's recursion
/// is otherwise unbounded — it has no `end` on the loop path).
const REC_BOUND: usize = 3;

/// The current holder + last-delivered-receiver of one live linear resource (the
/// same state the static linearity checker tracks, here evolved dynamically).
struct Live {
    holder: Role,
    receiver: Option<Role>,
}

/// Execute a [`Score`] as an in-memory simulation of its protocol model.
///
/// Branching is resolved by **always taking the first branch** (a deterministic
/// cooperative schedule); [`execute_with_choices`] lets a caller author a specific
/// branch path. Resources move per the §2.3 handoff rules; a broken handoff wedges
/// the schedule into [`Status::Stuck`].
///
/// SIMULATION ONLY — not a chain runtime; `Completed` does not imply on-chain
/// liveness. See the module docs.
pub fn execute(score: &Score) -> Trace {
    execute_with_choices(score, &[])
}

/// Like [`execute`], but `choices` authors the branch labels to take, in order, at
/// successive `Branching` nodes. A `Branching` with no remaining authored choice
/// falls back to its first branch. Lets a test drive a specific flow (e.g. the
/// escrow's `release` vs `refund` arm).
pub fn execute_with_choices(score: &Score, choices: &[&str]) -> Trace {
    let sorts: BTreeMap<String, Sort> = score
        .resources
        .iter()
        .map(|r| (r.name.clone(), r.sort))
        .collect();
    let mut live: BTreeMap<String, Live> = BTreeMap::new();
    let mut events = Vec::new();
    let mut choice_idx = 0usize;
    let status = run(
        &score.global,
        &sorts,
        &mut live,
        &mut events,
        choices,
        &mut choice_idx,
        &mut Vec::new(),
    );
    Trace { events, status }
}

/// Recursively drive one subtree. Returns the terminal status of THIS subtree;
/// `Completed` means it reached an `end` leaf cleanly. The first `Stuck` short-
/// circuits.
#[allow(clippy::too_many_arguments)]
fn run(
    g: &Global,
    sorts: &BTreeMap<String, Sort>,
    live: &mut BTreeMap<String, Live>,
    events: &mut Vec<Event>,
    choices: &[&str],
    choice_idx: &mut usize,
    rec_seen: &mut Vec<(String, usize)>,
) -> Status {
    match g {
        Global::End => Status::Completed,
        Global::Var(_) => {
            // A loop-back. The enclosing `Rec` accounts for unfolding; reaching a
            // `Var` here means the body finished one iteration. Treat as the
            // boundary the `Rec` arm re-drives (see below).
            Status::Completed
        }
        Global::Interact {
            from,
            to,
            message,
            resources,
            cont,
        } => {
            // Deliver this message: move each carried resource, enforcing the
            // holder chain dynamically (a non-holder carrying a live resource
            // wedges the schedule — the model's deadlock image).
            for r in resources {
                let sort = sorts.get(r).copied().unwrap_or(Sort::Continuation);
                match live.get(r) {
                    None => {
                        live.insert(
                            r.clone(),
                            Live {
                                holder: new_holder(sort, from, to),
                                receiver: delivered_receiver(sort, to),
                            },
                        );
                    }
                    Some(prev) => {
                        if prev.holder != *from {
                            return Status::Stuck {
                                where_: format!(
                                    "`{from}` fired `{message}` carrying `{r}`, but `{r}` is held \
                                     by `{}` (broken handoff)",
                                    prev.holder
                                ),
                            };
                        }
                        if let (Some(prev_to), Some(now_to)) =
                            (&prev.receiver, delivered_receiver(sort, to))
                        {
                            if *prev_to != now_to {
                                return Status::Stuck {
                                    where_: format!(
                                        "`{r}` (escrowed) was already delivered to `{prev_to}` but \
                                         `{message}` re-delivers it to `{now_to}` (double-spend)"
                                    ),
                                };
                            }
                        }
                        live.insert(
                            r.clone(),
                            Live {
                                holder: new_holder(sort, from, to),
                                receiver: delivered_receiver(sort, to),
                            },
                        );
                    }
                }
            }
            events.push(Event {
                from: from.clone(),
                to: to.clone(),
                message: message.clone(),
                resources: resources.clone(),
            });
            run(cont, sorts, live, events, choices, choice_idx, rec_seen)
        }
        Global::Branching { branches, .. } => {
            if branches.is_empty() {
                return Status::Stuck {
                    where_: "branching with no branches".to_string(),
                };
            }
            // Authored choice if available and it names a present branch; else the
            // first branch (deterministic cooperative schedule).
            let pick = choices.get(*choice_idx).and_then(|want| {
                branches
                    .iter()
                    .find(|(label, _)| label == want)
                    .map(|(_, sub)| sub)
            });
            *choice_idx += 1;
            let sub = pick.unwrap_or(&branches[0].1);
            run(sub, sorts, live, events, choices, choice_idx, rec_seen)
        }
        Global::Par(a, b) => {
            // Disjoint roles + resources (checked statically). A simple valid
            // cooperative interleaving is "all of a, then all of b".
            match run(a, sorts, live, events, choices, choice_idx, rec_seen) {
                Status::Completed => run(b, sorts, live, events, choices, choice_idx, rec_seen),
                other => other,
            }
        }
        Global::Rec(x, body) => {
            // Unfold up to REC_BOUND. Each unfolding re-drives the body; a body
            // ending in `Var(x)` loops back here.
            let count = rec_seen
                .iter()
                .find(|(v, _)| v == x)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            if count >= REC_BOUND {
                return Status::LoopBounded;
            }
            // push/update this rec's count
            if let Some(slot) = rec_seen.iter_mut().find(|(v, _)| v == x) {
                slot.1 += 1;
            } else {
                rec_seen.push((x.clone(), 1));
            }
            let status = run(body, sorts, live, events, choices, choice_idx, rec_seen);
            // If the body completed (reached `Var`/`End`), and the body loops (ends
            // in this var), re-drive; otherwise return. We detect a loop by whether
            // the body mentions `x` as a tail — approximated by re-running until the
            // bound, which yields `LoopBounded` for a true loop and `Completed` for
            // a body that actually reaches `End`.
            match status {
                Status::Completed if body_loops(body, x) => {
                    run(g, sorts, live, events, choices, choice_idx, rec_seen)
                }
                other => other,
            }
        }
    }
}

/// Whether `body` loops back to recursion variable `x` on some path (its tail is
/// `Var(x)`), distinguishing a genuine loop from a body that reaches `End`.
fn body_loops(body: &Global, x: &str) -> bool {
    match body {
        Global::Var(v) => v == x,
        Global::Interact { cont, .. } => body_loops(cont, x),
        Global::Branching { branches, .. } => branches.iter().any(|(_, s)| body_loops(s, x)),
        Global::Par(a, b) => body_loops(a, x) || body_loops(b, x),
        Global::Rec(y, inner) => y != x && body_loops(inner, x),
        Global::End => false,
    }
}

/// The holder of a resource after a step carries it (mirrors the static checker's
/// `new_holder`): a `Continuation` transfers to the receiver; an escrowed
/// `Value`/`Capability` stays under the authorising sender.
fn new_holder(sort: Sort, from: &Role, to: &Role) -> Role {
    match sort {
        Sort::Continuation => to.clone(),
        Sort::Value | Sort::Capability => from.clone(),
    }
}

/// The receiver a step delivers an escrowed `Value`/`Capability` to (for double-
/// delivery detection); `None` for a `Continuation`.
fn delivered_receiver(sort: Sort, to: &Role) -> Option<Role> {
    match sort {
        Sort::Value | Sort::Capability => Some(to.clone()),
        Sort::Continuation => None,
    }
}

/// Render a [`Trace`] to a clearly-labelled block with the honest footer.
pub fn render_trace(trace: &Trace) -> String {
    let mut out = String::new();
    out.push_str("--- M5 local executor trace (in-memory SIMULATION of the Score model) ---\n");
    for (i, e) in trace.events.iter().enumerate() {
        out.push_str(&format!(
            "  {:>2}. {} --[{}]--> {} {}\n",
            i + 1,
            e.from,
            e.message,
            e.to,
            render_res(&e.resources)
        ));
    }
    out.push_str(&format!("  status: {}\n", render_status(&trace.status)));
    out.push('\n');
    out.push_str(EXECUTOR_FOOTER);
    out.push('\n');
    out
}

fn render_status(s: &Status) -> String {
    match s {
        Status::Completed => "Completed".to_string(),
        Status::Stuck { where_ } => format!("Stuck {{ {where_} }}"),
        Status::LoopBounded => format!("LoopBounded (unfolded {REC_BOUND}x)"),
    }
}

fn render_res(resources: &[String]) -> String {
    if resources.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", resources.join(", "))
    }
}

/// The honest footer for any executor output. Makes the SIMULATION / not-a-chain-
/// runtime status, and the "Completed does not imply on-chain liveness" boundary,
/// unmissable.
pub const EXECUTOR_FOOTER: &str = "\
This is an IN-MEMORY SIMULATION of the Score protocol model under COOPERATIVE\n\
scheduling. It executes NO transaction, reads NO chain, builds NO UTXO, and\n\
verifies NO covenant. A `Completed` status is a statement about the MODEL only —\n\
it does NOT imply on-chain liveness: on a permissionless ledger a counterparty\n\
can simply never spend the UTXO it was handed, and only the relative-timelock\n\
escape (M3 NOT-STRANDED-BEYOND-T) bounds that reality. Pre-production, unaudited.";

#[cfg(test)]
mod tests;
