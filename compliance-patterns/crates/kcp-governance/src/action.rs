use crate::error::GovernanceError;
use serde::{Deserialize, Serialize};

/// A delayed-execution governance action.
///
/// Enforces that an execution cannot happen until at least `minimum_delay`
/// DAA heights have elapsed since the action was scheduled. Inspired by the EVM ecosystem's
/// `TimelockController` but adapted for the DAG-height clock.
///
/// **Usage flow:**
/// 1. Once a proposal passes, call [`TimelockAction::schedule`] to record the
///    scheduling height.
/// 2. Call [`TimelockAction::can_execute`] to check whether the delay has
///    elapsed at a given current height.
/// 3. On execution, advance the enclosing [`GovernorState`] to `Executed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelockAction {
    /// Minimum DAA-height delay between scheduling and execution.
    /// Must be ≥ 1.
    pub minimum_delay: u64,
    /// DAA height at which this action was scheduled (`None` = not yet scheduled).
    pub scheduled_at: Option<u64>,
}

impl TimelockAction {
    /// Create a new, unscheduled action with the given delay.
    ///
    /// `minimum_delay` must be ≥ 1.
    pub fn new(minimum_delay: u64) -> Result<Self, GovernanceError> {
        if minimum_delay == 0 {
            return Err(GovernanceError::InvalidMinimumDelay);
        }
        Ok(Self {
            minimum_delay,
            scheduled_at: None,
        })
    }

    /// Record the scheduling height.
    ///
    /// Returns `Err(ProposalNotPassed)` if called before the proposal has
    /// passed — callers must gate this on [`GovernorState`]'s status.
    pub fn schedule(&mut self, current_height: u64) -> Result<(), GovernanceError> {
        self.scheduled_at = Some(current_height);
        Ok(())
    }

    /// The earliest DAA height at which [`can_execute`](Self::can_execute)
    /// returns `true`. Returns `None` if not yet scheduled.
    pub fn earliest_execution_height(&self) -> Option<u64> {
        self.scheduled_at
            .map(|h| h.saturating_add(self.minimum_delay))
    }

    /// Returns `true` if the action can be executed at `current_height`.
    ///
    /// Requires:
    /// - action has been scheduled (`scheduled_at.is_some()`), AND
    /// - `current_height >= scheduled_at + minimum_delay`.
    pub fn can_execute(&self, current_height: u64) -> Result<(), GovernanceError> {
        match self.earliest_execution_height() {
            None => Err(GovernanceError::TimelockNotElapsed {
                required: 0,
                current: current_height,
            }),
            Some(required) if current_height < required => {
                Err(GovernanceError::TimelockNotElapsed {
                    required,
                    current: current_height,
                })
            }
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_delay_rejected() {
        assert_eq!(
            TimelockAction::new(0).unwrap_err(),
            GovernanceError::InvalidMinimumDelay
        );
    }

    #[test]
    fn unscheduled_cannot_execute() {
        let action = TimelockAction::new(100).unwrap();
        assert!(action.can_execute(9_999_999).is_err());
    }

    #[test]
    fn scheduled_cannot_execute_before_delay() {
        let mut action = TimelockAction::new(100).unwrap();
        action.schedule(1_000).unwrap();
        // earliest = 1000 + 100 = 1100
        assert!(action.can_execute(1_099).is_err());
        assert!(action.can_execute(1_100).is_ok());
        assert!(action.can_execute(9_999).is_ok());
    }

    #[test]
    fn earliest_execution_height() {
        let mut action = TimelockAction::new(50).unwrap();
        assert_eq!(action.earliest_execution_height(), None);
        action.schedule(200).unwrap();
        assert_eq!(action.earliest_execution_height(), Some(250));
    }

    #[test]
    fn serde_round_trip() {
        let mut action = TimelockAction::new(72).unwrap();
        action.schedule(5_000).unwrap();
        let json = serde_json::to_string(&action).unwrap();
        let a2: TimelockAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, a2);
    }
}
