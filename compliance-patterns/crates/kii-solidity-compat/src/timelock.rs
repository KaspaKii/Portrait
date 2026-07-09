//! `TimelockController`-shaped facade over `kcp-governance::TimelockAction`.
//!
//! EVM equivalent: `TimelockController` (Solidity pattern-library v5 shape).
//!
//! In Ethereum, `TimelockController` queues operations with a mandatory delay
//! and role-based access. In Kaspa, time is measured in **DAA heights** (one
//! unit ≈ one second at 1 BPS). This type wraps `kcp-governance::TimelockAction`
//! and adds a proposer/executor key pair for role-based gating.
//!
//! | Solidity `TimelockController` | This type |
//! |---|---|
//! | `constructor(minDelay, proposers[], executors[])` | `TimelockController::new(min_delay_daa, proposer, executor)` |
//! | `schedule(target, …, delay)` | `controller.schedule(current_daa)` |
//! | `isOperationReady(id)` | `controller.is_ready(current_daa)` |
//! | `execute(target, …)` | `controller.execute(current_daa)` |
//! | `cancel(id)` | `controller.cancel()` |
//!
//! **Kaspa difference:** the EVM pattern uses `bytes32 id` to identify queued operations;
//! here the entire `TimelockController` struct IS one queued operation. Create
//! one per logical action and persist it (e.g., anchor to a `kcp-sealed-lineage`
//! covenant for on-chain state continuity).
//!
//! **Pre-production, unaudited, testnet-only.**

use kcp_governance::action::TimelockAction;

use crate::error::{Error, Result};

/// A single queued operation with delay and role gating.
///
/// Holds the proposer and executor keys, wraps a `TimelockAction` for
/// delay logic, and tracks whether the operation has been cancelled.
#[derive(Debug, Clone)]
pub struct TimelockController {
    /// 32-byte x-only public key of the proposer (who may schedule).
    pub proposer_key: [u8; 32],
    /// 32-byte x-only public key of the executor (who may execute).
    pub executor_key: [u8; 32],
    /// Underlying timelock action.
    action: TimelockAction,
    /// Whether this operation has been cancelled.
    cancelled: bool,
}

impl TimelockController {
    /// Create a new, unscheduled controller.
    ///
    /// - `min_delay_daa`: minimum delay in DAA heights (≥ 1). At 1 BPS this
    ///   is approximately `min_delay_daa` seconds.
    /// - `proposer_key`: 32-byte x-only Schnorr key authorised to schedule.
    /// - `executor_key`: 32-byte x-only Schnorr key authorised to execute.
    ///
    /// Equivalent to `TimelockController(minDelay, [proposer], [executor])`.
    pub fn new(min_delay_daa: u64, proposer_key: [u8; 32], executor_key: [u8; 32]) -> Result<Self> {
        let action = TimelockAction::new(min_delay_daa).map_err(Error::Governance)?;
        Ok(Self {
            proposer_key,
            executor_key,
            action,
            cancelled: false,
        })
    }

    /// Schedule the operation at the current DAA height.
    ///
    /// The caller must provide the `proposer_key` to authenticate.
    /// Returns `Err(NotProposer)` if `by` is not the registered proposer, or
    /// `Err(AlreadyCancelled)` if the operation was previously cancelled.
    pub fn schedule(&mut self, by: [u8; 32], current_daa: u64) -> Result<()> {
        if self.cancelled {
            return Err(Error::AlreadyCancelled);
        }
        if by != self.proposer_key {
            return Err(Error::NotProposer);
        }
        self.action.schedule(current_daa).map_err(Error::Governance)
    }

    /// Return the earliest DAA height at which `execute` will succeed.
    /// Returns `None` if not yet scheduled.
    pub fn earliest_execution_height(&self) -> Option<u64> {
        self.action.earliest_execution_height()
    }

    /// Return `true` if the delay has elapsed and the operation is ready.
    pub fn is_ready(&self, current_daa: u64) -> bool {
        self.action.can_execute(current_daa).is_ok()
    }

    /// Execute the operation.
    ///
    /// Returns `Err(NotExecutor)` if `by` is not the registered executor,
    /// `Err(AlreadyCancelled)` if cancelled, or `Err(Governance)` if the
    /// delay has not elapsed.
    pub fn execute(&mut self, by: [u8; 32], current_daa: u64) -> Result<()> {
        if self.cancelled {
            return Err(Error::AlreadyCancelled);
        }
        if by != self.executor_key {
            return Err(Error::NotExecutor);
        }
        self.action
            .can_execute(current_daa)
            .map_err(Error::Governance)
    }

    /// Cancel the operation.
    ///
    /// Once cancelled, `schedule` and `execute` both return `Err(AlreadyCancelled)`.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Return `true` if the operation has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = b;
        k
    }

    #[test]
    fn zero_delay_rejected() {
        assert!(TimelockController::new(0, key(1), key(2)).is_err());
    }

    #[test]
    fn happy_path() {
        let mut ctrl = TimelockController::new(100, key(1), key(2)).unwrap();
        assert!(!ctrl.is_ready(1_000));

        // Schedule at height 1_000; earliest execution = 1_100
        ctrl.schedule(key(1), 1_000).unwrap();
        assert_eq!(ctrl.earliest_execution_height(), Some(1_100));

        assert!(!ctrl.is_ready(1_099));
        assert!(ctrl.is_ready(1_100));

        assert!(ctrl.execute(key(2), 1_100).is_ok());
    }

    #[test]
    fn wrong_proposer_rejected() {
        let mut ctrl = TimelockController::new(50, key(1), key(2)).unwrap();
        assert!(matches!(
            ctrl.schedule(key(9), 100),
            Err(Error::NotProposer)
        ));
    }

    #[test]
    fn wrong_executor_rejected() {
        let mut ctrl = TimelockController::new(50, key(1), key(2)).unwrap();
        ctrl.schedule(key(1), 100).unwrap();
        assert!(matches!(ctrl.execute(key(9), 200), Err(Error::NotExecutor)));
    }

    #[test]
    fn cancel_blocks_schedule_and_execute() {
        let mut ctrl = TimelockController::new(50, key(1), key(2)).unwrap();
        ctrl.cancel();
        assert!(matches!(
            ctrl.schedule(key(1), 100),
            Err(Error::AlreadyCancelled)
        ));
        assert!(matches!(
            ctrl.execute(key(2), 200),
            Err(Error::AlreadyCancelled)
        ));
    }

    #[test]
    fn execute_before_delay_fails() {
        let mut ctrl = TimelockController::new(100, key(1), key(2)).unwrap();
        ctrl.schedule(key(1), 1_000).unwrap();
        assert!(ctrl.execute(key(2), 1_099).is_err());
    }
}
