//! Security primitives for `kcp-common`.
//!
//! **Pre-production, unaudited.** Do not use in production without independent
//! review.
//!
//! ## What is here
//!
//! - [`Pausable`] — lightweight pause-state value type. **Pure data; no
//!   enforcement.** Callers must check [`is_paused`](Pausable::is_paused) at
//!   every guarded call site. Solidity's `Pausable` pattern provides `whenNotPaused` modifiers
//!   that revert EVM transactions; those modifier semantics have no direct analog
//!   in the Kaspa UTXO model.
//! - [`TimelockKind`] — discriminant for DAA-score vs unix-seconds deadlines.
//! - [`TimelockController`] — deadline + kind pair with a structural validity
//!   check. **Not interchangeable with `kcp-vault`'s `SpendCondition` timelock
//!   variants** — see [`TimelockController`] for details.
//!
//! ## ReentrancyGuard — intentionally omitted
//!
//! Ethereum's `ReentrancyGuard` protects against cross-function re-entrancy
//! that arises from EVM call stacks. The Kaspa UTXO model is structurally
//! non-reentrant: each transaction is a discrete spend of committed outputs
//! with no mid-execution callbacks. There is nothing for a reentrancy guard to
//! protect against, so the primitive is not provided here.

use serde::{Deserialize, Serialize};

/// Pause-state primitive. EVM equivalent: `Pausable` — pre-production, unaudited.
///
/// **This is a pure value type.** It carries pause state but enforces nothing.
/// Callers must check [`is_paused`](Self::is_paused) at every guarded call site;
/// the library does not provide Solidity-style modifiers.
///
/// All methods are pure and return a new value; no interior mutation. To start
/// from the canonical unpaused state, use [`Default::default()`].
///
/// **Warning:** the `paused` field is `pub`. Direct field mutation bypasses the
/// named transition methods (`pause()` / `unpause()`). This is intentional for
/// restore-from-storage use cases, but callers should prefer the methods for
/// clarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pausable {
    /// Whether the guarded operation is currently paused. Prefer
    /// [`pause`](Self::pause) / [`unpause`](Self::unpause) over direct mutation.
    pub paused: bool,
}

impl Default for Pausable {
    /// Returns an unpaused [`Pausable`] — the standard initial state.
    fn default() -> Self {
        Self { paused: false }
    }
}

impl Pausable {
    /// Create a new [`Pausable`] with the given initial state.
    ///
    /// Use `Pausable::new(false)` (or [`Default::default()`]) to start unpaused.
    /// `Pausable::new(true)` is valid for restore-from-stored-state scenarios.
    pub fn new(paused: bool) -> Self {
        Self { paused }
    }

    /// Returns `true` if currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Returns a new [`Pausable`] in the paused state.
    pub fn pause(&self) -> Self {
        Self { paused: true }
    }

    /// Returns a new [`Pausable`] in the unpaused state.
    pub fn unpause(&self) -> Self {
        Self { paused: false }
    }
}

/// Discriminant for a [`TimelockController`] deadline.
///
/// Names are aligned with (but not identical to) the `TimelockHeight` /
/// `TimelockUnixSeconds` variants used in `kcp-vault`'s `SpendCondition`.
/// **`TimelockController` is not interchangeable with those variants** — see
/// [`TimelockController`] for the distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelockKind {
    /// Deadline is expressed as a Kaspa DAA blue score (sometimes called block
    /// height). This corresponds to `SpendCondition::TimelockHeight` in
    /// `kcp-vault`, but the two types serve different roles — see
    /// [`TimelockController`].
    ///
    /// Note: Kaspa has no classical block height. The DAA blue score advances
    /// roughly once per second at the target 1 BPS, but the actual rate is
    /// governed by the difficulty-adjustment algorithm.
    DaaScore,
    /// Deadline is expressed as a Unix timestamp in seconds.
    UnixSeconds,
}

/// A (kind, deadline) pair for expressing timelock constraints, independent
/// of any spending-condition or script context.
///
/// ## Distinction from `kcp-vault::SpendCondition` timelock variants
///
/// `SpendCondition::TimelockHeight` and `SpendCondition::TimelockUnixSeconds`
/// bundle a signing key with the deadline and are directly consumed by the
/// `kcp-vault` script compiler and P2SH spend path. `TimelockController` is
/// a **pure data helper** with no key, no script-compilation path, and no
/// acceptance by any `kcp-vault` function. Constructing a valid
/// `TimelockController` is NOT sufficient precondition checking before
/// populating a `SpendCondition` — you must separately supply the controller
/// key.
///
/// ## `validate()` semantics
///
/// [`validate`](Self::validate) checks **structural** validity only: it
/// rejects `deadline == 0`. It does NOT check that the deadline is in the
/// future. A deadline of `1` (Unix epoch second one, 1970-01-01) passes
/// validation; a caller building a real timelock must additionally compare
/// `deadline` against the current environment value
/// (`deadline > current_unix_timestamp` for `UnixSeconds`, or
/// `deadline > current_daa_score` for `DaaScore`).
///
/// ## `pub` fields
///
/// Both fields are `pub` to support restore-from-storage patterns. Direct
/// mutation (e.g. `tc.deadline = 0`) bypasses the `validate()` invariant.
/// Re-validate after any field write.
///
/// **Deadline semantics:** `deadline` must be greater than zero. Note that
/// `kcp-vault`'s `SpendCondition` accepts `deadline = 0` in some variants;
/// this type is stricter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelockController {
    /// Whether the deadline is a DAA blue score or a Unix timestamp.
    pub kind: TimelockKind,
    /// The deadline value. Must be > 0. See type-level docs for temporal-validity
    /// semantics — `validate()` does not check that the deadline is in the future.
    pub deadline: u64,
}

impl TimelockController {
    /// Create a new [`TimelockController`].
    ///
    /// Call [`validate`](Self::validate) before trusting the value.
    pub fn new(kind: TimelockKind, deadline: u64) -> Self {
        Self { kind, deadline }
    }

    /// Returns `Ok(())` if the controller is structurally valid.
    ///
    /// **Structural check only** — this rejects `deadline == 0` but does NOT
    /// verify that the deadline is in the future. Callers must separately
    /// compare `deadline` against the current environment value (DAA score or
    /// Unix timestamp) before treating the lock as still active.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::ConditionInvalid`] if `deadline == 0`.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.deadline == 0 {
            return Err(crate::error::Error::ConditionInvalid(format!(
                "{:?} deadline must be greater than zero",
                self.kind
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Pausable ---

    #[test]
    fn pausable_new_starts_unpaused() {
        assert!(!Pausable::new(false).is_paused());
    }

    #[test]
    fn pausable_new_starts_paused() {
        assert!(Pausable::new(true).is_paused());
    }

    #[test]
    fn pausable_pause_returns_paused() {
        assert!(Pausable::new(false).pause().is_paused());
    }

    #[test]
    fn pausable_unpause_returns_unpaused() {
        assert!(!Pausable::new(true).unpause().is_paused());
    }

    #[test]
    fn pausable_pause_idempotent() {
        assert!(Pausable::new(true).pause().is_paused());
    }

    #[test]
    fn pausable_unpause_idempotent() {
        assert!(!Pausable::new(false).unpause().is_paused());
    }

    #[test]
    fn pausable_serde_round_trip() {
        let p = Pausable::new(true);
        let json = serde_json::to_string(&p).unwrap();
        let p2: Pausable = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn pausable_default_is_unpaused() {
        assert!(!Pausable::default().is_paused());
    }

    // --- TimelockController ---

    #[test]
    fn timelock_daa_score_valid() {
        assert!(TimelockController::new(TimelockKind::DaaScore, 1000)
            .validate()
            .is_ok());
    }

    #[test]
    fn timelock_unix_seconds_valid() {
        assert!(
            TimelockController::new(TimelockKind::UnixSeconds, 1_700_000_000)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn timelock_deadline_zero_rejected() {
        let err = TimelockController::new(TimelockKind::DaaScore, 0)
            .validate()
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("deadline must be greater than zero"),
            "unexpected error: {err}"
        );
    }

    /// deadline=1 is structurally valid (validate() is not a temporal check).
    /// For UnixSeconds, 1 is January 1970 — always elapsed in real use.
    #[test]
    fn timelock_deadline_one_valid() {
        assert!(TimelockController::new(TimelockKind::UnixSeconds, 1)
            .validate()
            .is_ok());
    }

    #[test]
    fn timelock_deadline_max_valid() {
        assert!(TimelockController::new(TimelockKind::DaaScore, u64::MAX)
            .validate()
            .is_ok());
    }

    #[test]
    fn timelock_serde_round_trip_daa_score() {
        let tc = TimelockController::new(TimelockKind::DaaScore, 42_000);
        let json = serde_json::to_string(&tc).unwrap();
        let tc2: TimelockController = serde_json::from_str(&json).unwrap();
        assert_eq!(tc, tc2);
    }

    #[test]
    fn timelock_serde_round_trip_unix_seconds() {
        let tc = TimelockController::new(TimelockKind::UnixSeconds, 1_800_000_000);
        let json = serde_json::to_string(&tc).unwrap();
        let tc2: TimelockController = serde_json::from_str(&json).unwrap();
        assert_eq!(tc, tc2);
    }
}
