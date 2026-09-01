//! Composer **M3** realization layer (design doc §4 — "UTXO / covenant
//! realisation and cross-role binding"; §4.2 KIP-20 binding; §4.3 timelock
//! escapes; §3.4 liveness boundary).
//!
//! M1 (`lib.rs`) gave the *type theory*; M2 (`lift.rs`) lifted a parsed program
//! to a [`Score`] and emitted per-role covenant **skeletons**. M3 takes the M2
//! emission one step closer to the on-chain picture by attaching the two
//! realization disciplines the design doc specifies — **and nothing more**:
//!
//! 1. **KIP-20 cross-role binding (§4.2).** Every role covenant in one realized
//!    protocol carries a shared **instance binding id** (a [`InstanceId`], a
//!    `KovId`-like tag *derived from the Score*) plus a **binding clause** that
//!    mirrors the documented `require(proof_cov_id == OpInputCovenantId(0))`
//!    guard. Two distinct Scores derive distinct ids, so a role covenant emitted
//!    for instance A cannot be spliced into instance B. [`realize_binding`]
//!    confirms every emitted role carries the *same* instance id and each
//!    interaction references it.
//!
//! 2. **Timelock-escape discipline (§4.3).** For every role that **waits** on a
//!    counterparty (its [`Local`] view contains a `Recv` / `Offer`), the
//!    realization emits a **relative-timelock escape branch** (`after this.age >=
//!    T, reclaim`). [`has_escape_for_every_waiter`] returns `Ok` only if every
//!    waiting role has an escape, else a *named* [`RealizeError`] naming the
//!    strandable role.
//!
//! # HONESTY — the property this buys, stated precisely (and its hard limit)
//!
//! The escape discipline buys exactly one liveness property:
//! **NOT-STRANDED-BEYOND-`T`** — every waiting role can *recover* its escrowed
//! resource after the relative timelock `T` elapses. It does **NOT** buy
//! happy-path liveness / completion: a silent counterparty still blocks the
//! protocol from *progressing*; all the escape gives the waiter is the ability to
//! get its own resource back after `T`. See [`liveness_report`] and
//! [`NOT_STRANDED_BEYOND_T`].
//!
//! This is **STRUCTURAL EMISSION at the type / realization level — not a runtime
//! and not a deployed covenant.** The instance binding id is a tag mirroring the
//! on-chain `OpInputCovenantId` pattern; it is **not** an on-chain settlement and
//! proves **nothing on-chain**. Nothing here reads a chain, executes a protocol,
//! or emits deployable `.sil`. (`docs/COMPOSER-M0-DESIGN.md §0`, §4.4, §6.)

use crate::lift::RoleSkeleton;
use crate::{Local, Role, Score};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

/// A `KovId`-like **instance binding id** for one realized protocol (§4.2).
/// Derived deterministically from the [`Score`] (its roles, resources, and global
/// type), so two *different* Scores derive *different* ids — the property that
/// makes splicing a role covenant from instance A into instance B detectable.
///
/// This is a STRUCTURAL tag mirroring the on-chain covenant-id (`OpInputCovenantId`)
/// pattern. It is **not** an on-chain covenant id and proves nothing on-chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceId(pub u64);

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 16-hex-digit rendering, the shape a covenant-id tag would take.
        write!(f, "kov:{:016x}", self.0)
    }
}

/// Derive the instance binding id from a [`Score`]. Deterministic and
/// structure-sensitive: the roles, the resource ledger, and the full global type
/// all feed the hash, so any structural change to the protocol changes the id.
pub fn derive_instance_id(score: &Score) -> InstanceId {
    let mut h = DefaultHasher::new();
    // A domain-separation tag so this id space is distinct from any other hash use.
    "portrait-compose/m3/instance-id/v1".hash(&mut h);
    score.roles.hash(&mut h);
    for r in &score.resources {
        r.name.hash(&mut h);
        // Sort is `Copy` + has a stable discriminant; hash its debug-stable tag.
        (r.sort as u8).hash(&mut h);
    }
    hash_global(&score.global, &mut h);
    InstanceId(h.finish())
}

/// Feed a [`crate::Global`] into the hasher structurally (order-sensitive).
fn hash_global(g: &crate::Global, h: &mut DefaultHasher) {
    use crate::Global::*;
    match g {
        Interact {
            from,
            to,
            message,
            resources,
            cont,
        } => {
            0u8.hash(h);
            from.hash(h);
            to.hash(h);
            message.hash(h);
            resources.hash(h);
            hash_global(cont, h);
        }
        Branching {
            decider,
            informed,
            branches,
        } => {
            1u8.hash(h);
            decider.hash(h);
            informed.hash(h);
            for (label, sub) in branches {
                label.hash(h);
                hash_global(sub, h);
            }
        }
        Par(a, b) => {
            2u8.hash(h);
            hash_global(a, h);
            hash_global(b, h);
        }
        Rec(x, body) => {
            3u8.hash(h);
            x.hash(h);
            hash_global(body, h);
        }
        Var(x) => {
            4u8.hash(h);
            x.hash(h);
        }
        End => 5u8.hash(h),
    }
}

/// A **relative-timelock escape branch** (§4.3) attached to a waiting role: after
/// `this.age >= delay`, the waiter may `reclaim` the named escrowed resources
/// rather than block forever on a silent counterparty.
///
/// STRUCTURAL: this records the escape's existence and shape for the realization
/// report; it is not executable and not deployed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelockEscape {
    /// The counterparty whose silence this escape protects against (the role the
    /// waiter is waiting on).
    pub waiting_on: Role,
    /// The relative-timelock delay (`this.age >= delay`) after which reclaim is
    /// enabled. A symbolic `T`; the deployer fixes the concrete block-age.
    pub delay: RelativeTimelock,
    /// The resources the waiter may reclaim once the timelock elapses. These are
    /// the waiter's **own previously-escrowed** resources (resources it sent into
    /// the protocol before this wait) — NOT the incoming/awaited resource, which
    /// the silent counterparty holds and the waiter never received.
    pub reclaim: Vec<String>,
    /// The waiter's own escrowed resources that are **at risk** at this wait
    /// (everything it has sent into the protocol up to this point). The escape is
    /// only non-vacuous if `reclaim` actually recovers these — checked by
    /// [`has_escape_for_every_waiter`]. This is what makes the
    /// NOT-STRANDED-BEYOND-T verdict non-vacuous (red-team fix).
    pub escrowed_at_risk: Vec<String>,
}

/// A symbolic relative timelock (`this.age >= T`), the Kaspa relative-timelock
/// primitive the design doc names (§4.3). `T` is left symbolic at this layer; the
/// deployer fixes the concrete block-age.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RelativeTimelock {
    /// The symbolic deadline parameter index (always `0` for the single-`T`
    /// model). Retained so a multi-deadline relaxation has somewhere to grow.
    pub deadline: u32,
}

/// A wait point at which a role has its **own escrowed resources at risk** — the
/// ground truth the escape discipline is checked against (red-team fix). Derived
/// by [`realize`] from the *ordered* projected [`Local`]: a wait carries risk iff
/// the role has sent (escrowed) resources into the protocol before reaching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtRiskWait {
    /// The counterparty whose silence would strand the role at this wait.
    pub waiting_on: Role,
    /// The role's OWN escrowed resources at risk at this wait (non-empty by
    /// construction — an empty-risk wait is not recorded here).
    pub escrowed: Vec<String>,
}

/// A **realized** per-role covenant skeleton: the M2 [`RoleSkeleton`] plus the M3
/// realization data — the shared instance binding id (§4.2) and the timelock
/// escapes for every wait this role performs (§4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealizedRole {
    /// The underlying structural skeleton (entrypoints + awaits).
    pub skeleton: RoleSkeleton,
    /// The shared instance binding id this role covenant carries (§4.2). Every
    /// role in one realized protocol carries the SAME id.
    pub instance: InstanceId,
    /// Relative-timelock escapes — one per wait at which this role has escrowed
    /// resources at risk (§4.3). Empty for a role that is never at risk (it only
    /// receives, only sends, or always receives before it escrows anything).
    pub escapes: Vec<TimelockEscape>,
    /// The ground-truth wait points at which this role has its OWN escrowed
    /// resources at risk (red-team fix). The escape discipline is checked against
    /// THIS — not the raw skeleton awaits — so the verdict is non-vacuous: every
    /// at-risk wait must be covered by an escape that actually reclaims those
    /// escrowed resources. A role with no entries here is not strandable.
    pub at_risk_waits: Vec<AtRiskWait>,
}

impl RealizedRole {
    /// Whether this role can be **stranded** — i.e. it has at least one wait at
    /// which its own escrowed resources are at risk. (NOT merely "has an await":
    /// a role that always receives before it escrows anything has nothing of its
    /// own to lose and is not strandable — red-team fix.)
    pub fn is_waiter(&self) -> bool {
        !self.at_risk_waits.is_empty()
    }
}

/// A named realization failure. Like the M1/M2 errors, M3 never silently passes —
/// a missing escape or a binding mismatch is a loud, *named* rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealizeError {
    /// **Timelock-escape discipline (§4.3).** A role that waits on a counterparty
    /// has no relative-timelock escape branch, so on a silent counterparty it
    /// could be **stranded forever** (beyond `T` with no recovery). Names the
    /// strandable role and the counterparty it waits on.
    MissingEscape {
        /// The role that can be stranded.
        role: Role,
        /// A counterparty whose silence would strand it.
        waiting_on: Role,
    },
    /// **KIP-20 binding (§4.2).** Two role covenants in the same realized protocol
    /// carry *different* instance binding ids — a splice: a role from one instance
    /// has been mixed with a role from another. Names the two divergent ids.
    InstanceIdMismatch {
        /// The role whose id diverges from the protocol's.
        role: Role,
        /// The id the protocol expects (the shared instance id).
        expected: InstanceId,
        /// The id this role actually carries.
        found: InstanceId,
    },
    /// **KIP-20 binding (§4.2).** A realized protocol has no roles at all, so there
    /// is no instance to bind — a degenerate realization.
    NoRoles,
}

impl std::fmt::Display for RealizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RealizeError::MissingEscape { role, waiting_on } => write!(
                f,
                "role `{role}` waits on `{waiting_on}` but has no relative-timelock \
                 escape: it could be stranded beyond T with no recovery (§4.3)"
            ),
            RealizeError::InstanceIdMismatch {
                role,
                expected,
                found,
            } => write!(
                f,
                "role `{role}` carries instance id `{found}` but the protocol is \
                 bound to `{expected}` — a cross-instance splice (§4.2)"
            ),
            RealizeError::NoRoles => {
                write!(f, "realization has no roles — nothing to bind (§4.2)")
            }
        }
    }
}

impl std::error::Error for RealizeError {}

/// **Realize** a checked projection into per-role covenant data with the M3
/// disciplines attached: the shared instance binding id (§4.2) and a
/// relative-timelock escape for every wait (§4.3).
///
/// `score` provides the structure the instance id is derived from; `locals` is the
/// projection returned by [`Score::check`]. The returned roles are in role order.
///
/// SCOPE: this attaches STRUCTURAL realization data mirroring the documented
/// on-chain patterns. It is not a deployment and proves nothing on-chain.
pub fn realize(score: &Score, locals: &BTreeMap<Role, Local>) -> Vec<RealizedRole> {
    let instance = derive_instance_id(score);
    let skeletons = crate::lift::emit_role_skeletons(locals);
    skeletons
        .into_iter()
        .map(|skeleton| {
            // At-risk waits are derived from the ordered projection (not the
            // flattened skeleton), because what a waiter can be stranded on is
            // exactly the resources it has ESCROWED (sent) BEFORE the wait, in
            // protocol order. The escape for each at-risk wait reclaims THOSE OWN
            // escrowed resources — never the incoming/awaited resource.
            let local = locals
                .get(&skeleton.role)
                .expect("skeleton role must have a projected local");
            let at_risk_waits = at_risk_waits_for(local);
            let escapes = at_risk_waits
                .iter()
                .map(|w| TimelockEscape {
                    waiting_on: w.waiting_on.clone(),
                    delay: RelativeTimelock::default(),
                    reclaim: w.escrowed.clone(),
                    escrowed_at_risk: w.escrowed.clone(),
                })
                .collect();
            RealizedRole {
                skeleton,
                instance,
                escapes,
                at_risk_waits,
            }
        })
        .collect()
}

/// Compute the wait points at which a role has its OWN escrowed resources at risk
/// (§4.3), from its **ordered** projected [`Local`]. The honest model: at each
/// wait (`Recv` / `Offer`) the role is at risk on exactly the resources it has
/// **escrowed** (sent into the protocol) up to that point. A wait at which the
/// role has escrowed nothing carries nothing of its own at risk and is NOT
/// recorded (so it is not strandable).
fn at_risk_waits_for(local: &Local) -> Vec<AtRiskWait> {
    let mut out = Vec::new();
    let mut escrowed: Vec<String> = Vec::new();
    walk_at_risk(local, &mut escrowed, &mut out);
    out
}

/// Walk the local type in protocol order, accumulating own-escrowed (sent)
/// resources and recording an at-risk wait at each `Recv` / `Offer` that has
/// escrowed resources at risk. Branches are walked with a snapshot of the
/// escrowed-so-far set so a branch's sends do not leak across sibling branches.
fn walk_at_risk(t: &Local, escrowed: &mut Vec<String>, out: &mut Vec<AtRiskWait>) {
    match t {
        Local::Send {
            resources, cont, ..
        } => {
            for r in resources {
                if !escrowed.contains(r) {
                    escrowed.push(r.clone());
                }
            }
            walk_at_risk(cont, escrowed, out);
        }
        Local::Recv { from, cont, .. } => {
            // A wait. The role is at risk on whatever it has escrowed so far.
            if !escrowed.is_empty() {
                out.push(AtRiskWait {
                    waiting_on: from.clone(),
                    escrowed: escrowed.clone(),
                });
            }
            walk_at_risk(cont, escrowed, out);
        }
        Local::Offer { from, branches } => {
            // An external choice is a wait too: at risk on what is escrowed so
            // far, regardless of which branch the decider picks.
            if !escrowed.is_empty() {
                out.push(AtRiskWait {
                    waiting_on: from.clone(),
                    escrowed: escrowed.clone(),
                });
            }
            for (_, sub) in branches {
                let mut branch_escrowed = escrowed.clone();
                walk_at_risk(sub, &mut branch_escrowed, out);
            }
        }
        Local::Select { branches, .. } => {
            for (_, sub) in branches {
                let mut branch_escrowed = escrowed.clone();
                walk_at_risk(sub, &mut branch_escrowed, out);
            }
        }
        Local::Par(a, b) => {
            walk_at_risk(a, escrowed, out);
            walk_at_risk(b, escrowed, out);
        }
        Local::Rec(_, body) => walk_at_risk(body, escrowed, out),
        Local::Var(_) | Local::End => {}
    }
}

// ===== (1) KIP-20 cross-role binding check (§4.2) ============================

/// Confirm the realized protocol is **bound to a single instance** (§4.2): every
/// role carries the SAME instance binding id, and that id is the one derived from
/// `score`. Rejects a cross-instance splice with a named
/// [`RealizeError::InstanceIdMismatch`].
///
/// This is the structural statement of "a role covenant from instance A cannot be
/// spliced into instance B": each role's `instance` must equal the protocol's
/// derived id, so a role realized against a *different* Score (a different id)
/// is rejected.
pub fn realize_binding(score: &Score, roles: &[RealizedRole]) -> Result<InstanceId, RealizeError> {
    if roles.is_empty() {
        return Err(RealizeError::NoRoles);
    }
    let expected = derive_instance_id(score);
    for r in roles {
        if r.instance != expected {
            return Err(RealizeError::InstanceIdMismatch {
                role: r.skeleton.role.clone(),
                expected,
                found: r.instance,
            });
        }
    }
    Ok(expected)
}

// ===== (2) timelock-escape discipline check (§4.3) ==========================

/// Confirm **every at-risk wait has a NON-VACUOUS relative-timelock escape**
/// (§4.3). Returns `Ok(())` only when, for each role and each wait at which the
/// role has its OWN escrowed resources at risk ([`RealizedRole::at_risk_waits`]),
/// there is an escape on that counterparty whose `reclaim` **actually recovers
/// those escrowed resources** (non-empty and a superset of the at-risk set).
/// Otherwise a named [`RealizeError::MissingEscape`] naming the strandable role
/// and the counterparty whose silence would strand it.
///
/// This is the red-team fix: the original check matched escapes by peer ONLY and
/// never inspected `reclaim`, so an empty-reclaim escape, or one reclaiming the
/// incoming (counterparty-held) resource instead of the waiter's own, passed
/// while recovering nothing. The verdict is now non-vacuous — an escape that does
/// not reclaim the at-risk escrowed resources does not satisfy the check.
///
/// A role with no at-risk waits (only receives, only sends, or always receives
/// before it escrows anything) has nothing of its own to lose and needs no
/// escape — it is not flagged.
pub fn has_escape_for_every_waiter(roles: &[RealizedRole]) -> Result<(), RealizeError> {
    for r in roles {
        for wait in &r.at_risk_waits {
            let covered = r.escapes.iter().any(|e| {
                e.waiting_on == wait.waiting_on
                    && !e.reclaim.is_empty()
                    && reclaim_covers(&e.reclaim, &wait.escrowed)
            });
            if !covered {
                return Err(RealizeError::MissingEscape {
                    role: r.skeleton.role.clone(),
                    waiting_on: wait.waiting_on.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Whether an escape's `reclaim` actually recovers every escrowed resource at
/// risk (i.e. `reclaim ⊇ escrowed`). An escape that reclaims only the incoming
/// resource, or a strict subset of the escrowed resources, does NOT cover the
/// wait — the red-team's vacuous-escape vector.
fn reclaim_covers(reclaim: &[String], escrowed: &[String]) -> bool {
    escrowed.iter().all(|res| reclaim.contains(res))
}

// ===== (3) liveness property: NOT-STRANDED-BEYOND-T (§3.4, §4.3) =============

/// The precise liveness property the escape discipline buys — stated so it cannot
/// be upgraded to happy-path completion (§3.4 boundary, §4.3 limit).
pub const NOT_STRANDED_BEYOND_T: &str =
    "NOT-STRANDED-BEYOND-T: every waiting role can RECOVER its escrowed resource \
     after the relative timelock T elapses (via its escape branch). This is the \
     ONLY liveness property the escape discipline buys. It is NOT happy-path \
     liveness / completion: a silent counterparty still blocks the protocol from \
     progressing; the escape only lets the waiter reclaim its own resource after T.";

/// The outcome of the liveness analysis for a realized protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessReport {
    /// True iff every at-risk wait is covered by a NON-VACUOUS escape (one that
    /// actually reclaims the escrowed resources), so NOT-STRANDED-BEYOND-`T`
    /// holds. When false, [`LivenessReport::strandable`] names the gaps.
    pub not_stranded_beyond_t: bool,
    /// The roles that have escrowed resources at risk at some wait (the roles for
    /// which the property is load-bearing). A role that only receives / only sends
    /// / always receives before escrowing anything is NOT listed — it is not
    /// strandable (red-team fix).
    pub waiters: Vec<Role>,
    /// At-risk roles whose escape is missing or vacuous (empty when the property
    /// holds).
    pub strandable: Vec<Role>,
}

impl LivenessReport {
    /// Whether happy-path completion is guaranteed. **ALWAYS `false`** — this layer
    /// never claims happy-path liveness. A silent counterparty blocks progress
    /// regardless of escapes; the escape only bounds stranding. Provided as an
    /// explicit, honest method so callers cannot mistake the property for
    /// completion.
    pub fn happy_path_completion_guaranteed(&self) -> bool {
        false
    }
}

/// Compute the [`LivenessReport`] for a realized protocol: assert
/// NOT-STRANDED-BEYOND-`T` (every waiting role has an escape), and clearly
/// distinguish it from happy-path completion (which is NOT guaranteed —
/// [`LivenessReport::happy_path_completion_guaranteed`] is always `false`).
pub fn liveness_report(roles: &[RealizedRole]) -> LivenessReport {
    let mut waiters = Vec::new();
    let mut strandable = Vec::new();
    for r in roles {
        if r.is_waiter() {
            waiters.push(r.skeleton.role.clone());
        }
        // Non-vacuous: a wait is covered only by an escape that actually reclaims
        // its at-risk escrowed resources — not by a peer-only match.
        for wait in &r.at_risk_waits {
            let covered = r.escapes.iter().any(|e| {
                e.waiting_on == wait.waiting_on
                    && !e.reclaim.is_empty()
                    && reclaim_covers(&e.reclaim, &wait.escrowed)
            });
            if !covered && !strandable.contains(&r.skeleton.role) {
                strandable.push(r.skeleton.role.clone());
            }
        }
    }
    LivenessReport {
        not_stranded_beyond_t: strandable.is_empty(),
        waiters,
        strandable,
    }
}

// ===== (4) rendering: binding id + escapes + honest liveness footer ==========

/// Render the realized roles to a clearly-labelled block: the shared instance
/// binding id, each role's binding clause + escapes, and the honest liveness
/// footer. The banner makes the STRUCTURAL / not-on-chain status unmissable.
pub fn render_realization(score: &Score, roles: &[RealizedRole]) -> String {
    let instance = derive_instance_id(score);
    let report = liveness_report(roles);
    let mut out = String::new();
    out.push_str(
        "--- realized per-role covenants (M3 STRUCTURAL realization, NOT on-chain, \
         NOT deployable .sil) ---\n",
    );
    out.push_str(&format!("instance binding id (KIP-20, §4.2): {instance}\n"));
    for r in roles {
        out.push_str(&format!("\ncovenant {} {{\n", r.skeleton.role));
        // The KIP-20 binding clause — mirrors require(proof_cov_id == OpInputCovenantId(0)).
        out.push_str(&format!(
            "  // KIP-20 cross-role binding (§4.2) — structural, mirrors on-chain guard\n  \
             require(instance_id == {instance});  // == OpInputCovenantId(0)\n"
        ));
        for e in &r.skeleton.entrypoints {
            out.push_str(&format!(
                "  entrypoint {}  ->{}  {}\n",
                e.message,
                e.peer,
                render_res(&e.resources)
            ));
        }
        for a in &r.skeleton.awaits {
            out.push_str(&format!(
                "  awaits     {}  <-{}  {}\n",
                a.message,
                a.peer,
                render_res(&a.resources)
            ));
        }
        for esc in &r.escapes {
            out.push_str(&format!(
                "  escape     after this.age >= T(deadline {})  reclaim {}  // silence of {} (§4.3)\n",
                esc.delay.deadline,
                render_res(&esc.reclaim),
                esc.waiting_on,
            ));
        }
        out.push_str("}\n");
    }
    out.push_str("\n-- liveness (honest) --\n");
    out.push_str(&format!(
        "NOT-STRANDED-BEYOND-T holds: {}\n",
        report.not_stranded_beyond_t
    ));
    if !report.waiters.is_empty() {
        out.push_str(&format!("waiting roles: {}\n", report.waiters.join(", ")));
    }
    if !report.strandable.is_empty() {
        out.push_str(&format!(
            "STRANDABLE (missing escape): {}\n",
            report.strandable.join(", ")
        ));
    }
    out.push_str(&format!(
        "happy-path completion guaranteed: {}\n",
        report.happy_path_completion_guaranteed()
    ));
    out.push('\n');
    out.push_str(NOT_STRANDED_BEYOND_T);
    out.push('\n');
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
