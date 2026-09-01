//! portrait-compose — Composer **M1** (type-level only).
//!
//! This crate implements the *type theory* specified in
//! `kaspa-compliance-patterns/docs/COMPOSER-M0-DESIGN.md`: a **global protocol
//! type** (the *Score*, §2), its **projection** onto each role to a **local
//! type** (§3.1), and a **checker** for the three well-formedness conditions —
//! **projectability** (§3.2), **duality / no-orphan** (§3.3), and **linearity**
//! (§3.3). A valid protocol returns `Ok`; an invalid one returns a *named*
//! [`ComposeError`] pinpointing the violation.
//!
//! # What this proves — and what it does NOT
//!
//! A Composer *accept* is a statement about the **protocol abstraction** only:
//!
//! - **SAFETY (in scope).** If every role acts according to its projected local
//!   type, the composed protocol has **no stuck state** (every receive /
//!   external-choice has a matching send / internal-choice until all roles reach
//!   `end`) and every resource is produced once and consumed once
//!   (**linear handoff**). This is the synchronous-MPST "deadlock-free by
//!   construction" result, verified here by checking projectability + the
//!   syntactic side-conditions + linearity.
//!
//! - **NO liveness on a permissionless UTXO/DAG (out of scope).** This checker
//!   does **not** prove progress against an adversarially-silent counterparty.
//!   On a permissionless ledger a party can simply *never spend* the UTXO it was
//!   handed; session-type progress assumes eventual participation, which the
//!   ledger does not enforce. The only honest on-chain remedy is a relative
//!   timelock escape, whose guarantee is "not stranded **beyond `T`**", never
//!   "the happy path completes". That escape discipline is design-level (§4.3)
//!   and is **not** modelled or checked here.
//!
//! - **Proves the MODEL, not the deployed covenants.** Covenant *bodies*
//!   (value conservation) are out of scope (that is Lens's job). The fidelity of
//!   the emitted silverscript to the local type (model-vs-script) is a separate,
//!   unclaimed gap. This is **M1, type-level — not a runtime**: nothing here
//!   executes a protocol, talks to a chain, or emits an artifact.
//!
//! # No dependencies
//!
//! The Score is its own small AST; the checker is pure structural type theory.
//! This crate has **no** external Cargo dependencies and no path deps — it is a
//! standalone, dep-free workspace member.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};

/// A role identifier (e.g. `Buyer`). `R` in the design doc.
pub type Role = String;
/// A message / interaction label (e.g. `fund`). Maps to an `authorizing_entry`.
pub type Message = String;
/// A branch label (e.g. `release`). `L` in the design doc.
pub type Label = String;
/// A recursion variable name. `X` in the design doc.
pub type Var = String;

/// The **sort** of a linear resource — fixes how it would realise on-chain
/// (§2.3). The checker treats all sorts uniformly for linearity; the sort is
/// retained for diagnostics and downstream realisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sort {
    /// Fungible value units — a UTXO value moved by the settling tx.
    Value,
    /// An authorisation / guard token — a covenant guard predicate.
    Capability,
    /// The right/obligation to take the next protocol step — a covenant-ID
    /// handoff (the `parent_kov_id` mechanism generalised). The load-bearing
    /// sort: it is what turns "A's message to B" into "B can only act in a tx
    /// that spends A".
    Continuation,
}

/// A declared linear resource (`Score.resources`, §2.3). Each resource is
/// linear: introduced exactly once and consumed exactly once along every path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// The resource identifier (e.g. `payment`).
    pub name: String,
    /// Its on-chain realisation sort.
    pub sort: Sort,
}

impl Resource {
    /// Declare a resource of the given sort.
    pub fn new(name: impl Into<String>, sort: Sort) -> Self {
        Resource {
            name: name.into(),
            sort,
        }
    }
}

/// The **global type** `G` (§2.2): the protocol described once, from a
/// god's-eye view, before projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Global {
    /// `p →[m] q { r } . G` — `p` sends message `m`, carrying resources `r`, to
    /// `q`; then the protocol continues as the boxed `Global`.
    Interact {
        /// The single sending / authorising role (§2.2 single-sender).
        from: Role,
        /// The receiving role. Must differ from `from` (no self-interaction).
        to: Role,
        /// The message label (maps to an `authorizing_entry`).
        message: Message,
        /// The (possibly empty) multiset of resources transferred this step.
        resources: Vec<String>,
        /// The continuation.
        cont: Box<Global>,
    },
    /// `p → { … } informing q` — internal choice at `p` (the single decider),
    /// `q` is notified of the selected label; one continuation per label.
    Branching {
        /// The single deciding / selecting role.
        decider: Role,
        /// The notified role (told which branch was taken). Must differ from
        /// `decider`.
        informed: Role,
        /// One `(label, continuation)` per branch. Labels must be distinct and
        /// non-empty.
        branches: Vec<(Label, Global)>,
    },
    /// `G_1 ∥ G_2` — parallel composition over **disjoint** role *and* resource
    /// sets (the M0/M1 disjointness restriction, §2.2).
    Par(Box<Global>, Box<Global>),
    /// `μX. G` — guarded recursion (`X` must occur under a prefix inside `G`).
    Rec(Var, Box<Global>),
    /// `X` — a recursion variable; must be bound by an enclosing `Rec`.
    Var(Var),
    /// `end` — termination; all resources must be consumed by here.
    End,
}

// ----- small constructor API (ergonomic Score authoring) ---------------------

impl Global {
    /// `p →[m] q { r } . cont`.
    pub fn interact(
        from: impl Into<Role>,
        message: impl Into<Message>,
        to: impl Into<Role>,
        resources: &[&str],
        cont: Global,
    ) -> Global {
        Global::Interact {
            from: from.into(),
            to: to.into(),
            message: message.into(),
            resources: resources.iter().map(|s| s.to_string()).collect(),
            cont: Box::new(cont),
        }
    }

    /// `decider → { ℓ_i : G_i } informing informed`.
    pub fn branching(
        decider: impl Into<Role>,
        informed: impl Into<Role>,
        branches: Vec<(&str, Global)>,
    ) -> Global {
        Global::Branching {
            decider: decider.into(),
            informed: informed.into(),
            branches: branches
                .into_iter()
                .map(|(l, g)| (l.to_string(), g))
                .collect(),
        }
    }

    /// `G_1 ∥ G_2`.
    pub fn par(a: Global, b: Global) -> Global {
        Global::Par(Box::new(a), Box::new(b))
    }

    /// `μX. body`.
    pub fn rec(x: impl Into<Var>, body: Global) -> Global {
        Global::Rec(x.into(), Box::new(body))
    }

    /// `X`.
    pub fn var(x: impl Into<Var>) -> Global {
        Global::Var(x.into())
    }
}

/// The **local type** `T` (§3.1): one role's private view of the protocol,
/// derived by projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Local {
    /// `q ![m] { r } . T` — send `m` to `q`.
    Send {
        /// Peer the message is sent to.
        to: Role,
        /// Message label.
        message: Message,
        /// Resources carried.
        resources: Vec<String>,
        /// Continuation.
        cont: Box<Local>,
    },
    /// `q ?[m] { r } . T` — receive `m` from `q`.
    Recv {
        /// Peer the message is received from.
        from: Role,
        /// Message label.
        message: Message,
        /// Resources carried.
        resources: Vec<String>,
        /// Continuation.
        cont: Box<Local>,
    },
    /// `q ⊕ { ℓ_i : T_i }` — internal choice: this role decides and informs `q`.
    Select {
        /// Role informed of the decision.
        to: Role,
        /// One continuation per label.
        branches: Vec<(Label, Local)>,
    },
    /// `q & { ℓ_i : T_i }` — external choice: this role awaits `q`'s selection.
    Offer {
        /// Role this choice is awaited from.
        from: Role,
        /// One continuation per label.
        branches: Vec<(Label, Local)>,
    },
    /// `T_1 ∥ T_2`.
    Par(Box<Local>, Box<Local>),
    /// `μX. T`.
    Rec(Var, Box<Local>),
    /// `X`.
    Var(Var),
    /// `end`.
    End,
}

/// A named well-formedness failure — the checker never silently passes; an
/// unprojectable / incompatible / non-linear protocol is a loud, *named*
/// rejection (§0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// An interaction or branching has `from == to` (`p ≠ q`, §2.2).
    SelfInteraction {
        /// The offending role.
        role: Role,
        /// The message or "<branching>".
        at: String,
    },
    /// Recursion `μX. G` is unguarded — `X` reachable without crossing a prefix
    /// (e.g. `μX. X`), which would not project to a finite covenant lifecycle.
    UnguardedRecursion {
        /// The recursion variable.
        var: Var,
    },
    /// A recursion variable is used outside any binding `μX` (§2.2).
    UnboundVariable {
        /// The free variable.
        var: Var,
    },
    /// A branching has duplicate or empty labels.
    BadBranchLabels {
        /// The deciding role.
        decider: Role,
    },
    /// `G_1 ∥ G_2` shares a role across the two branches (disjointness, §2.2).
    ParRoleOverlap {
        /// A role appearing in both parallel branches.
        role: Role,
    },
    /// `G_1 ∥ G_2` shares a resource across the two branches (§2.2).
    ParResourceOverlap {
        /// A resource appearing in both parallel branches.
        resource: String,
    },
    /// **Projectability** (§3.2): a non-deciding role's behaviour diverges
    /// across branches with no distinguishing notification — the merge is
    /// undefined. The role could wait on a message that, on the branch taken,
    /// is never sent.
    NotProjectable {
        /// The role whose merge is undefined.
        role: Role,
        /// Human-readable description of the two divergent behaviours.
        detail: String,
    },
    /// **Duality / no-orphan** (§3.3): a message appears as a send with no
    /// matching receive, or as a receive with no matching send.
    OrphanMessage {
        /// The orphaned message label.
        message: Message,
        /// `"send"` or `"receive"` — which half is unmatched.
        unmatched: &'static str,
    },
    /// **Linearity** (§3.3): a resource is consumed (sent) more than once on a
    /// single path — a double-spend of a `Continuation`.
    ResourceConsumedTwice {
        /// The over-consumed resource.
        resource: String,
    },
    /// **Linearity** (§3.3): a resource is introduced/produced but never
    /// consumed on some path to `end` — stranded value.
    ResourceStranded {
        /// The stranded resource.
        resource: String,
    },
    /// A resource carried by an interaction is not declared in the ledger.
    UndeclaredResource {
        /// The undeclared resource name.
        resource: String,
    },
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComposeError::SelfInteraction { role, at } => {
                write!(
                    f,
                    "self-interaction by role `{role}` at `{at}` (p ≠ q required)"
                )
            }
            ComposeError::UnguardedRecursion { var } => {
                write!(
                    f,
                    "unguarded recursion: variable `{var}` reachable without a prefix"
                )
            }
            ComposeError::UnboundVariable { var } => {
                write!(f, "unbound recursion variable `{var}`")
            }
            ComposeError::BadBranchLabels { decider } => {
                write!(f, "branching at `{decider}` has empty or duplicate labels")
            }
            ComposeError::ParRoleOverlap { role } => {
                write!(
                    f,
                    "parallel composition shares role `{role}` (must be disjoint)"
                )
            }
            ComposeError::ParResourceOverlap { resource } => {
                write!(
                    f,
                    "parallel composition shares resource `{resource}` (must be disjoint)"
                )
            }
            ComposeError::NotProjectable { role, detail } => {
                write!(f, "not projectable onto `{role}`: {detail}")
            }
            ComposeError::OrphanMessage { message, unmatched } => {
                write!(f, "orphan message `{message}`: {unmatched} has no match")
            }
            ComposeError::ResourceConsumedTwice { resource } => {
                write!(
                    f,
                    "linearity: resource `{resource}` consumed more than once on a path"
                )
            }
            ComposeError::ResourceStranded { resource } => {
                write!(
                    f,
                    "linearity: resource `{resource}` produced but never consumed (stranded)"
                )
            }
            ComposeError::UndeclaredResource { resource } => {
                write!(
                    f,
                    "resource `{resource}` carried by an interaction is not declared"
                )
            }
        }
    }
}

impl std::error::Error for ComposeError {}

/// A complete protocol: roles, the linear resource ledger, and the global type.
/// This is the object the checker consumes (the *Score*).
#[derive(Debug, Clone)]
pub struct Score {
    /// The declared roles (`R`).
    pub roles: Vec<Role>,
    /// The linear resource ledger (`Res`).
    pub resources: Vec<Resource>,
    /// The global protocol type (`G`).
    pub global: Global,
}

impl Score {
    /// Build a Score.
    pub fn new(roles: &[&str], resources: Vec<Resource>, global: Global) -> Self {
        Score {
            roles: roles.iter().map(|s| s.to_string()).collect(),
            resources,
            global,
        }
    }

    /// Run the full checker: well-formed-syntax → projectability → duality /
    /// no-orphan → linearity. On success, return the per-role projected local
    /// types (`BTreeMap` keyed by role, for stable iteration). On failure,
    /// return the first named violation.
    ///
    /// SCOPE: a successful check proves **safety of the protocol model with
    /// exactly-once handoff** under the assumption that each role acts per its
    /// local type. It does **not** prove liveness on a UTXO/DAG and does **not**
    /// reason about the deployed covenants. See the crate docs.
    pub fn check(&self) -> Result<BTreeMap<Role, Local>, ComposeError> {
        // (0) syntactic well-formedness.
        check_syntax(&self.global, &mut Vec::new())?;
        check_resources_declared(&self.global, &self.declared_resource_set())?;

        // (1) participating roles = roles touched by the global type, plus any
        // declared role (so a declared-but-silent role still projects to `end`).
        let mut roles: BTreeSet<Role> = self.roles.iter().cloned().collect();
        collect_roles(&self.global, &mut roles);

        // (2) projection (carries projectability check via merge).
        let mut locals: BTreeMap<Role, Local> = BTreeMap::new();
        for r in &roles {
            locals.insert(r.clone(), project(&self.global, r)?);
        }

        // (3) duality / no-orphan over the projected local types.
        check_duality(&locals)?;

        // (4) linearity over the global type (path-sensitive).
        check_linearity(&self.global, &self.sort_map())?;

        // (5) no declared resource is left unused — a declared-but-never-carried
        // resource is produced zero times and so can never be consumed: a
        // stranded ledger entry (§3.3 "none stranded", the degenerate case).
        let mut carried: BTreeSet<String> = BTreeSet::new();
        collect_resources(&self.global, &mut carried);
        for res in &self.resources {
            if !carried.contains(&res.name) {
                return Err(ComposeError::ResourceStranded {
                    resource: res.name.clone(),
                });
            }
        }

        Ok(locals)
    }

    fn declared_resource_set(&self) -> BTreeSet<String> {
        self.resources.iter().map(|r| r.name.clone()).collect()
    }

    fn sort_map(&self) -> BTreeMap<String, Sort> {
        self.resources
            .iter()
            .map(|r| (r.name.clone(), r.sort))
            .collect()
    }
}

// ===== (0) syntactic well-formedness =========================================

/// `bound` is the stack of in-scope recursion variables.
fn check_syntax(g: &Global, bound: &mut Vec<Var>) -> Result<(), ComposeError> {
    match g {
        Global::Interact {
            from,
            to,
            message,
            cont,
            ..
        } => {
            if from == to {
                return Err(ComposeError::SelfInteraction {
                    role: from.clone(),
                    at: message.clone(),
                });
            }
            check_syntax(cont, bound)
        }
        Global::Branching {
            decider,
            informed,
            branches,
        } => {
            if decider == informed {
                return Err(ComposeError::SelfInteraction {
                    role: decider.clone(),
                    at: "<branching>".to_string(),
                });
            }
            let mut seen = BTreeSet::new();
            for (label, _) in branches {
                if label.is_empty() || !seen.insert(label.clone()) {
                    return Err(ComposeError::BadBranchLabels {
                        decider: decider.clone(),
                    });
                }
            }
            for (_, sub) in branches {
                check_syntax(sub, bound)?;
            }
            Ok(())
        }
        Global::Par(a, b) => {
            // Disjoint roles.
            let (mut ra, mut rb) = (BTreeSet::new(), BTreeSet::new());
            collect_roles(a, &mut ra);
            collect_roles(b, &mut rb);
            if let Some(role) = ra.intersection(&rb).next() {
                return Err(ComposeError::ParRoleOverlap { role: role.clone() });
            }
            // Disjoint resources.
            let (mut sa, mut sb) = (BTreeSet::new(), BTreeSet::new());
            collect_resources(a, &mut sa);
            collect_resources(b, &mut sb);
            if let Some(res) = sa.intersection(&sb).next() {
                return Err(ComposeError::ParResourceOverlap {
                    resource: res.clone(),
                });
            }
            check_syntax(a, bound)?;
            check_syntax(b, bound)
        }
        Global::Rec(x, body) => {
            // Guardedness: `x` must not be reachable in `body` without crossing
            // a prefix (interaction/branching).
            if reachable_unguarded(body, x) {
                return Err(ComposeError::UnguardedRecursion { var: x.clone() });
            }
            bound.push(x.clone());
            let r = check_syntax(body, bound);
            bound.pop();
            r
        }
        Global::Var(x) => {
            if bound.contains(x) {
                Ok(())
            } else {
                Err(ComposeError::UnboundVariable { var: x.clone() })
            }
        }
        Global::End => Ok(()),
    }
}

/// True if `var` is reachable in `g` without crossing an interaction/branching
/// prefix — i.e. the recursion is unguarded.
fn reachable_unguarded(g: &Global, var: &str) -> bool {
    match g {
        Global::Var(x) => x == var,
        // Prefixes guard everything beneath them.
        Global::Interact { .. } | Global::Branching { .. } => false,
        Global::Par(a, b) => reachable_unguarded(a, var) || reachable_unguarded(b, var),
        // A nested `μvar` shadows; a different `μy` is transparent for `var`.
        Global::Rec(y, body) => y != var && reachable_unguarded(body, var),
        Global::End => false,
    }
}

fn check_resources_declared(g: &Global, declared: &BTreeSet<String>) -> Result<(), ComposeError> {
    match g {
        Global::Interact {
            resources, cont, ..
        } => {
            for r in resources {
                if !declared.contains(r) {
                    return Err(ComposeError::UndeclaredResource {
                        resource: r.clone(),
                    });
                }
            }
            check_resources_declared(cont, declared)
        }
        Global::Branching { branches, .. } => {
            for (_, sub) in branches {
                check_resources_declared(sub, declared)?;
            }
            Ok(())
        }
        Global::Par(a, b) => {
            check_resources_declared(a, declared)?;
            check_resources_declared(b, declared)
        }
        Global::Rec(_, body) => check_resources_declared(body, declared),
        Global::Var(_) | Global::End => Ok(()),
    }
}

fn collect_roles(g: &Global, out: &mut BTreeSet<Role>) {
    match g {
        Global::Interact { from, to, cont, .. } => {
            out.insert(from.clone());
            out.insert(to.clone());
            collect_roles(cont, out);
        }
        Global::Branching {
            decider,
            informed,
            branches,
        } => {
            out.insert(decider.clone());
            out.insert(informed.clone());
            for (_, sub) in branches {
                collect_roles(sub, out);
            }
        }
        Global::Par(a, b) => {
            collect_roles(a, out);
            collect_roles(b, out);
        }
        Global::Rec(_, body) => collect_roles(body, out),
        Global::Var(_) | Global::End => {}
    }
}

fn collect_resources(g: &Global, out: &mut BTreeSet<String>) {
    match g {
        Global::Interact {
            resources, cont, ..
        } => {
            for r in resources {
                out.insert(r.clone());
            }
            collect_resources(cont, out);
        }
        Global::Branching { branches, .. } => {
            for (_, sub) in branches {
                collect_resources(sub, out);
            }
        }
        Global::Par(a, b) => {
            collect_resources(a, out);
            collect_resources(b, out);
        }
        Global::Rec(_, body) => collect_resources(body, out),
        Global::Var(_) | Global::End => {}
    }
}

// ===== (2) projection (§3.1) + projectability (§3.2) =========================

/// Project the global type `g` onto role `s`, deriving `s`'s local type.
/// Returns [`ComposeError::NotProjectable`] when a non-decider's branch
/// behaviour cannot be merged (the projectability side-condition, §3.2).
pub fn project(g: &Global, s: &Role) -> Result<Local, ComposeError> {
    match g {
        Global::Interact {
            from,
            to,
            message,
            resources,
            cont,
        } => {
            let tail = project(cont, s)?;
            if s == from {
                Ok(Local::Send {
                    to: to.clone(),
                    message: message.clone(),
                    resources: resources.clone(),
                    cont: Box::new(tail),
                })
            } else if s == to {
                Ok(Local::Recv {
                    from: from.clone(),
                    message: message.clone(),
                    resources: resources.clone(),
                    cont: Box::new(tail),
                })
            } else {
                // non-participant skips this step.
                Ok(tail)
            }
        }
        Global::Branching {
            decider,
            informed,
            branches,
        } => {
            if s == decider {
                let mut bs = Vec::new();
                for (label, sub) in branches {
                    bs.push((label.clone(), project(sub, s)?));
                }
                Ok(Local::Select {
                    to: informed.clone(),
                    branches: bs,
                })
            } else if s == informed {
                let mut bs = Vec::new();
                for (label, sub) in branches {
                    bs.push((label.clone(), project(sub, s)?));
                }
                Ok(Local::Offer {
                    from: decider.clone(),
                    branches: bs,
                })
            } else {
                // Non-decider, non-informed role: its behaviour must MERGE across
                // all branches. Defined only if identical in every branch (it has
                // no distinguishing notification of the chosen label) — §3.2.
                let mut projected = Vec::new();
                for (_, sub) in branches {
                    projected.push(project(sub, s)?);
                }
                merge_all(&projected, s)
            }
        }
        Global::Par(a, b) => {
            let pa = project(a, s)?;
            let pb = project(b, s)?;
            // Disjoint roles (checked in syntax): `s` participates in at most one
            // side; the other projects to `end`. Collapse the trivial side.
            match (pa, pb) {
                (Local::End, x) | (x, Local::End) => Ok(x),
                (x, y) => Ok(Local::Par(Box::new(x), Box::new(y))),
            }
        }
        Global::Rec(x, body) => {
            let inner = project(body, s)?;
            // Drop a vacuous recursion (`s` does not act inside it).
            if mentions_var(&inner, x) {
                Ok(Local::Rec(x.clone(), Box::new(inner)))
            } else {
                Ok(strip_var(inner, x))
            }
        }
        Global::Var(x) => Ok(Local::Var(x.clone())),
        Global::End => Ok(Local::End),
    }
}

/// Merge a non-decider role's per-branch projections (§3.2). Standard MPST
/// *full merge*, specialised to this subset:
///
/// - if all branches are **structurally identical**, the merge is that common
///   type (the role behaves the same regardless of the branch — it needs no
///   notification);
/// - otherwise the branches must be **distinguished by a leading receive** from
///   the *same* sender with *distinct* message labels (a "distinguishing receive
///   at the head", §3.2). They merge into an external choice (`Offer`) over those
///   labels — the role is notified which branch was taken and acts accordingly;
/// - any other divergence (different leading actions with no notification, or
///   the same label leading to different continuations) is **undefined** and
///   rejected as [`ComposeError::NotProjectable`] — the role could wait on a
///   message that, on the branch taken, is never sent.
fn merge_all(projected: &[Local], role: &Role) -> Result<Local, ComposeError> {
    let first = match projected.first() {
        Some(f) => f,
        None => return Ok(Local::End),
    };

    // Case 1: all identical.
    if projected.iter().all(|t| t == first) {
        return Ok(first.clone());
    }

    // Case 2: all are leading receives from the SAME sender — merge by label.
    let mut from_sender: Option<Role> = None;
    let mut merged: Vec<(Label, Local)> = Vec::new();
    let mut all_recv = true;
    for t in projected {
        match t {
            Local::Recv {
                from,
                message,
                resources,
                cont,
            } => {
                match &from_sender {
                    None => from_sender = Some(from.clone()),
                    Some(s) if s == from => {}
                    Some(_) => {
                        all_recv = false;
                        break;
                    }
                }
                // Use the message label as the distinguishing branch label.
                let branch = Local::Recv {
                    from: from.clone(),
                    message: message.clone(),
                    resources: resources.clone(),
                    cont: cont.clone(),
                };
                if merged.iter().any(|(l, t2)| *l == *message && *t2 != branch) {
                    // Same label, different continuation → not mergeable.
                    all_recv = false;
                    break;
                }
                if !merged.iter().any(|(l, _)| *l == *message) {
                    merged.push((message.clone(), branch));
                }
            }
            _ => {
                all_recv = false;
                break;
            }
        }
    }

    if all_recv && merged.len() >= 2 {
        if let Some(from) = from_sender {
            return Ok(Local::Offer {
                from,
                branches: merged,
            });
        }
    }

    Err(ComposeError::NotProjectable {
        role: role.clone(),
        detail: format!(
            "behaviour diverges across branches with no distinguishing \
             notification: `{}` vs `{}`",
            describe(&projected[0]),
            describe(projected.iter().find(|t| *t != first).unwrap_or(first))
        ),
    })
}

/// A short human-readable tag for a local type head (for diagnostics).
fn describe(t: &Local) -> String {
    match t {
        Local::Send { to, message, .. } => format!("send {message}→{to}"),
        Local::Recv { from, message, .. } => format!("recv {message}←{from}"),
        Local::Select { to, .. } => format!("select →{to}"),
        Local::Offer { from, .. } => format!("offer ←{from}"),
        Local::Par(..) => "parallel".to_string(),
        Local::Rec(x, _) => format!("μ{x}"),
        Local::Var(x) => x.clone(),
        Local::End => "end".to_string(),
    }
}

fn mentions_var(t: &Local, x: &str) -> bool {
    match t {
        Local::Var(y) => y == x,
        Local::Send { cont, .. } | Local::Recv { cont, .. } => mentions_var(cont, x),
        Local::Select { branches, .. } | Local::Offer { branches, .. } => {
            branches.iter().any(|(_, sub)| mentions_var(sub, x))
        }
        Local::Par(a, b) => mentions_var(a, x) || mentions_var(b, x),
        Local::Rec(y, body) => y != x && mentions_var(body, x),
        Local::End => false,
    }
}

/// Replace a now-unused recursion: since `x` does not occur, the body IS the
/// result; just return it (the `Rec` wrapper was vacuous).
fn strip_var(t: Local, _x: &str) -> Local {
    t
}

// ===== (3) duality / no-orphan (§3.3) ========================================

/// Every send label must have a matching receive in exactly one other role's
/// local type, and vice versa (the static per-label precondition, §3.2 "no
/// orphan messages"). Ordering/duality then follows by construction from the
/// projection of one well-formed global type (§3.3) — we do not re-derive it by
/// exploring the product automaton.
fn check_duality(locals: &BTreeMap<Role, Local>) -> Result<(), ComposeError> {
    let mut sends: BTreeMap<Message, usize> = BTreeMap::new();
    let mut recvs: BTreeMap<Message, usize> = BTreeMap::new();
    for t in locals.values() {
        collect_message_polarity(t, &mut sends, &mut recvs);
    }
    // Every send needs a matching receive.
    for m in sends.keys() {
        if !recvs.contains_key(m) {
            return Err(ComposeError::OrphanMessage {
                message: m.clone(),
                unmatched: "send",
            });
        }
    }
    // Every receive needs a matching send.
    for m in recvs.keys() {
        if !sends.contains_key(m) {
            return Err(ComposeError::OrphanMessage {
                message: m.clone(),
                unmatched: "receive",
            });
        }
    }
    Ok(())
}

fn collect_message_polarity(
    t: &Local,
    sends: &mut BTreeMap<Message, usize>,
    recvs: &mut BTreeMap<Message, usize>,
) {
    match t {
        Local::Send { message, cont, .. } => {
            *sends.entry(message.clone()).or_insert(0) += 1;
            collect_message_polarity(cont, sends, recvs);
        }
        Local::Recv { message, cont, .. } => {
            *recvs.entry(message.clone()).or_insert(0) += 1;
            collect_message_polarity(cont, sends, recvs);
        }
        Local::Select { branches, .. } | Local::Offer { branches, .. } => {
            for (_, sub) in branches {
                collect_message_polarity(sub, sends, recvs);
            }
        }
        Local::Par(a, b) => {
            collect_message_polarity(a, sends, recvs);
            collect_message_polarity(b, sends, recvs);
        }
        Local::Rec(_, body) => collect_message_polarity(body, sends, recvs),
        Local::Var(_) | Local::End => {}
    }
}

// ===== (4) linearity (§3.3) ==================================================

/// Linearity over the global type, path-sensitive, modelled as a **handoff
/// chain** with explicit holder-tracking (§3.3):
///
/// - **Introduced once.** The first interaction to carry a resource *produces*
///   it; its receiver becomes the resource's current **holder**.
/// - **Handed off in a connected chain.** A later interaction carrying a live
///   resource must be authorised by its **current holder** (`from == holder`);
///   it consumes the resource from that holder and rebinds it. Who the new
///   holder is depends on the resource **sort** (§2.3): a `Continuation`
///   transfers to the **receiver** (it *is* the right to act next), while an
///   escrowed `Value` / `Capability` stays under the **sender** — the
///   authorising party retains spend control of the escrowed coin until it is
///   handed off. This is exactly the escrow's legal `fund … settle` chain:
///   `payment` (a `Value`) is escrowed under Buyer at `fund` and released by
///   Buyer at `settle` (single-active-role); the `step` `Continuation` transfers
///   to Seller at `fund`.
/// - **Double-spend = a broken chain.** If a live resource is carried by a role
///   that is *not* its current holder, two parties claim the same linear token —
///   a double-spend of a `Continuation`. Rejected as
///   [`ComposeError::ResourceConsumedTwice`].
/// - **Exclusive branches don't double-count.** Each maximal path is checked
///   independently, so a resource on branch `ℓ_1` and on `ℓ_2` is fine (they
///   never both happen).
/// - **Double-receive of an escrowed token = a broken chain.** A `Value` or
///   `Capability` re-carried by its (unchanged) holder to a *different* receiver
///   is the same linear token delivered to two parties — rejected as
///   [`ComposeError::ResourceConsumedTwice`]. A re-carry to the *same* receiver
///   (the escrow's `fund … settle`) is a legal re-carry and accepted.
/// - **Reaching `end` discharges the live set** to its terminal holder, so a
///   resource legitimately held at `end` (e.g. `payment` with Seller on
///   `release`) is *not* stranded. Stranding ([`ComposeError::ResourceStranded`])
///   is detected for an escrowed `Value`/`Capability` still live at a
///   **non-terminating recursion cut** (a `Var`, where the path folds back
///   without reaching `end`): such a token is produced and then abandoned. A
///   `Continuation` live at a cut is the right to act in the *next* iteration, so
///   it threads through the loop-back and is **not** flagged here (avoiding false
///   positives on legitimate recursive handoff protocols). The degenerate case —
///   a declared resource never carried by any interaction — is also stranded
///   (caught in [`Score::check`]).
///
/// SCOPE NOTE: this is the off-chain, statically-checked image of the on-chain
/// single-spend guarantee — checked *before* deployment, over the protocol
/// model. It is not a proof about the emitted covenant body.
fn check_linearity(g: &Global, sorts: &BTreeMap<String, Sort>) -> Result<(), ComposeError> {
    for path in enumerate_paths(g) {
        check_path_linearity(&path, sorts)?;
    }
    Ok(())
}

/// The role that holds a resource after an interaction carries it: a
/// `Continuation` transfers to the receiver; an escrowed `Value`/`Capability`
/// stays under the authorising sender (§2.3 realisation).
fn new_holder(sort: Sort, step: &Step) -> Role {
    match sort {
        Sort::Continuation => step.to.clone(),
        Sort::Value | Sort::Capability => step.from.clone(),
    }
}

/// One step on a path: the authorising sender, the receiver, and the resources
/// the interaction carries.
#[derive(Debug, Clone)]
struct Step {
    from: Role,
    to: Role,
    resources: Vec<String>,
}

/// One root-to-leaf path through the global type.
#[derive(Debug, Clone)]
struct PathTrace {
    steps: Vec<Step>,
    /// Whether this path reaches `end` (vs. being cut at a recursion `Var`).
    terminates: bool,
}

/// Enumerate all root-to-leaf paths through the global type, unfolding each
/// recursion at most once (guarded ⇒ finite). Branching forks the path set;
/// `Par` (disjoint resources ⇒ per-branch-independent linearity) concatenates
/// each side's path traces.
fn enumerate_paths(g: &Global) -> Vec<PathTrace> {
    fn go(g: &Global, seen_rec: &mut BTreeSet<Var>) -> Vec<PathTrace> {
        match g {
            Global::End => vec![PathTrace {
                steps: Vec::new(),
                terminates: true,
            }],
            Global::Var(_) => vec![PathTrace {
                steps: Vec::new(),
                terminates: false,
            }],
            Global::Interact {
                from,
                to,
                resources,
                cont,
                ..
            } => {
                let head = Step {
                    from: from.clone(),
                    to: to.clone(),
                    resources: resources.clone(),
                };
                go(cont, seen_rec)
                    .into_iter()
                    .map(|mut t| {
                        let mut steps = vec![head.clone()];
                        steps.append(&mut t.steps);
                        PathTrace {
                            steps,
                            terminates: t.terminates,
                        }
                    })
                    .collect()
            }
            Global::Branching { branches, .. } => {
                let mut out = Vec::new();
                for (_, sub) in branches {
                    out.extend(go(sub, seen_rec));
                }
                out
            }
            Global::Par(a, b) => {
                let pa = go(a, seen_rec);
                let pb = go(b, seen_rec);
                let mut out = Vec::new();
                for ta in &pa {
                    for tb in &pb {
                        let mut steps = ta.steps.clone();
                        steps.extend(tb.steps.clone());
                        out.push(PathTrace {
                            steps,
                            terminates: ta.terminates && tb.terminates,
                        });
                    }
                }
                out
            }
            Global::Rec(x, body) => {
                if seen_rec.contains(x) {
                    vec![PathTrace {
                        steps: Vec::new(),
                        terminates: false,
                    }]
                } else {
                    seen_rec.insert(x.clone());
                    let r = go(body, seen_rec);
                    seen_rec.remove(x);
                    r
                }
            }
        }
    }
    go(g, &mut BTreeSet::new())
}

/// The live-tracking state of one linear resource along a path.
struct Live {
    /// Its sort (fixes the handoff rule and whether it strands at a cut).
    sort: Sort,
    /// The role currently authorised to hand it off (the *holder*).
    holder: Role,
    /// For an escrowed `Value`/`Capability`, the role it was last DELIVERED to.
    /// A `Value`/`Capability` re-carried to a DIFFERENT receiver is a
    /// double-spend (the same coin delivered to two parties), even though the
    /// authorising holder is unchanged. `None` for a `Continuation` (whose
    /// holder *is* the receiver, so the holder check already covers it).
    receiver: Option<Role>,
}

/// Holder-chain check for one path (see [`check_linearity`]).
fn check_path_linearity(
    path: &PathTrace,
    sorts: &BTreeMap<String, Sort>,
) -> Result<(), ComposeError> {
    // resource -> its live-tracking state.
    let mut live: BTreeMap<String, Live> = BTreeMap::new();
    for step in &path.steps {
        for r in &step.resources {
            // Default sort for an (already-declared) resource; declaration is
            // enforced earlier, so this lookup always succeeds in `check()`.
            let sort = sorts.get(r).copied().unwrap_or(Sort::Continuation);
            match live.get(r) {
                None => {
                    // First carry: produce it.
                    live.insert(
                        r.clone(),
                        Live {
                            sort,
                            holder: new_holder(sort, step),
                            receiver: delivered_receiver(sort, step),
                        },
                    );
                }
                Some(prev) => {
                    if prev.holder != step.from {
                        // A non-holder is carrying a live resource → double-spend.
                        return Err(ComposeError::ResourceConsumedTwice {
                            resource: r.clone(),
                        });
                    }
                    // For an escrowed Value/Capability the holder (the sender)
                    // is unchanged across a re-carry, so the holder check above
                    // cannot see a delivery to a SECOND, distinct receiver. Track
                    // the receiver side explicitly: a re-carry to a different
                    // party is the same linear token delivered twice → reject.
                    if let (Some(prev_to), Some(now_to)) =
                        (&prev.receiver, delivered_receiver(sort, step))
                    {
                        if *prev_to != now_to {
                            return Err(ComposeError::ResourceConsumedTwice {
                                resource: r.clone(),
                            });
                        }
                    }
                    // Legal handoff / re-carry: rebind per sort.
                    live.insert(
                        r.clone(),
                        Live {
                            sort,
                            holder: new_holder(sort, step),
                            receiver: delivered_receiver(sort, step),
                        },
                    );
                }
            }
        }
    }

    // A path cut at a recursion `Var` (non-terminating) discharges nothing to a
    // terminal holder. Any escrowed `Value`/`Capability` still live at the cut is
    // produced and then ABANDONED — stranded (§3.3). A `Continuation` is the
    // right to act in the next iteration, so it threads through the loop-back and
    // is NOT treated as stranded here.
    if !path.terminates {
        for (name, l) in &live {
            if matches!(l.sort, Sort::Value | Sort::Capability) {
                return Err(ComposeError::ResourceStranded {
                    resource: name.clone(),
                });
            }
        }
    }
    Ok(())
}

/// The role a step DELIVERS an escrowed `Value`/`Capability` to (its receiver),
/// for double-receive detection. `None` for a `Continuation` (its holder is the
/// receiver, so the holder check already covers it).
fn delivered_receiver(sort: Sort, step: &Step) -> Option<Role> {
    match sort {
        Sort::Value | Sort::Capability => Some(step.to.clone()),
        Sort::Continuation => None,
    }
}

// ===== pretty-printing (test/CLI harness aid) ================================

/// Render a local type to a one-line, human-readable string (for the optional
/// test harness / `portrait compose` print). Purely a diagnostic aid.
pub fn render_local(t: &Local) -> String {
    match t {
        Local::Send {
            to,
            message,
            resources,
            cont,
        } => {
            format!(
                "{to} ![{message}]{} . {}",
                render_res(resources),
                render_local(cont)
            )
        }
        Local::Recv {
            from,
            message,
            resources,
            cont,
        } => {
            format!(
                "{from} ?[{message}]{} . {}",
                render_res(resources),
                render_local(cont)
            )
        }
        Local::Select { to, branches } => {
            let bs: Vec<String> = branches
                .iter()
                .map(|(l, t)| format!("{l}: {}", render_local(t)))
                .collect();
            format!("{to} ⊕ {{ {} }}", bs.join(", "))
        }
        Local::Offer { from, branches } => {
            let bs: Vec<String> = branches
                .iter()
                .map(|(l, t)| format!("{l}: {}", render_local(t)))
                .collect();
            format!("{from} & {{ {} }}", bs.join(", "))
        }
        Local::Par(a, b) => format!("({} ∥ {})", render_local(a), render_local(b)),
        Local::Rec(x, body) => format!("μ{x}. {}", render_local(body)),
        Local::Var(x) => x.clone(),
        Local::End => "end".to_string(),
    }
}

fn render_res(resources: &[String]) -> String {
    if resources.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", resources.join(", "))
    }
}

/// The honest boundary footer — what a Composer *accept* means, and (loudly)
/// what it does not. Intended for any harness/CLI that prints a verdict.
pub const HONEST_BOUNDARY_FOOTER: &str = "\
Composer M1 (type-level). An ACCEPT proves SAFETY of the protocol MODEL only:\n\
no stuck state and exactly-once linear handoff, IF each role acts per its local\n\
type. It does NOT prove liveness on a permissionless UTXO/DAG (a counterparty\n\
can simply never act; only a timelock escape bounds stranding, and that is not\n\
modelled here). It proves the MODEL, not the deployed covenants (covenant bodies\n\
are Lens's job; model-vs-emitted-script fidelity is a separate, unclaimed gap).\n\
Pre-production, unaudited, testnet-only.";

/// The canonical 3-party escrow worked example from the M0 doc (§5): Buyer funds
/// escrow handing a `step` continuation to Seller; Seller delivers `asset` to
/// Buyer; Arbiter decides `release` (payment → Seller) or `refund` (payment
/// discharged to Buyer), notifying both Buyer and Seller. Projects to three
/// well-formed role local types and passes all three checks.
pub fn asset_escrow_example() -> Score {
    use Sort::*;
    // Arbiter's verdict must notify BOTH non-deciders. The grammar's `Branching`
    // notifies one role; we notify Seller at the decision, then Buyer relays the
    // verdict to Seller-independent... In the single-informed grammar we model
    // the two notifications as nested branchings sharing the same label set so
    // that BOTH Buyer and Seller learn the branch (projectability §5.3).
    let release = Global::interact("Buyer", "settle", "Seller", &["payment"], Global::End);
    let refund = Global::End; // payment discharged locally to Buyer (no self-send, §5.1).

    // Arbiter informs Buyer; Buyer's branch then informs Seller of the same
    // verdict via a notify message, so Seller is notified (no orphan wait).
    let release_with_notify = Global::interact("Buyer", "verdict_release", "Seller", &[], release);
    let refund_with_notify = Global::interact("Buyer", "verdict_refund", "Seller", &[], refund);

    let global = Global::interact(
        "Buyer",
        "fund",
        "Seller",
        &["payment", "step"],
        Global::interact(
            "Seller",
            "deliver",
            "Buyer",
            &["asset"],
            Global::branching(
                "Arbiter",
                "Buyer",
                vec![
                    ("release", release_with_notify),
                    ("refund", refund_with_notify),
                ],
            ),
        ),
    );

    Score::new(
        &["Buyer", "Seller", "Arbiter"],
        vec![
            Resource::new("payment", Value),
            Resource::new("asset", Capability),
            Resource::new("step", Continuation),
        ],
        global,
    )
}

/// M2 front-end lift (`App` flow → `Score`) + per-role covenant skeleton emission.
pub mod lift;

/// M3 realization layer: KIP-20 cross-role binding (§4.2), timelock-escape
/// discipline (§4.3), and the NOT-STRANDED-BEYOND-`T` liveness property (§3.4).
pub mod realize;

/// M5 **real** per-role covenant emission: from a checked projection, emit
/// per-role `.portrait` covenant TEXT that genuinely round-trips
/// (`portrait_syntax::parse` Ok + `portrait_sema::check` passes) — the faithful
/// single-role subset, with non-emittable constructs recorded as named gaps.
pub mod emit_real;

/// M5 **local executor**: an in-memory SIMULATION of the Score model under
/// cooperative scheduling — NOT a chain runtime. Drives the protocol to a `Trace`
/// + terminal `Status` (`Completed` / `Stuck` / `LoopBounded`). A `Completed` run
/// does NOT imply on-chain liveness.
pub mod execute;

#[cfg(test)]
mod tests;
