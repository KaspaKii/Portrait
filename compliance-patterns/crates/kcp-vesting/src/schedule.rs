//! Linear vesting schedule keyed to Kaspa DAA heights.

use super::error::{Result, VestingError};
use serde::{Deserialize, Serialize};

/// A linear vesting schedule using DAA heights as the clock.
///
/// EVM equivalent: `VestingWallet` — pre-production, unaudited.
///
/// Tokens vest linearly from `start` to `start + duration`. Before `start`,
/// nothing is releasable. After `start + duration`, everything is releasable.
///
/// `released` tracks cumulative releases and is updated by [`release`].
///
/// **This is a pure value type.** [`release`] returns a new schedule and the
/// released amount; callers are responsible for persisting the updated schedule
/// and transferring the released amount.
///
/// **DAA height caveats:** Kaspa's DAA blue score advances roughly once per
/// second at 1 BPS. Use real current DAA heights from a `kaspad` node; do not
/// use synthetic heights in production.
///
/// [`release`]: VestingSchedule::release
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VestingSchedule {
    /// The beneficiary's x-only public key (32 bytes).
    pub beneficiary: [u8; 32],
    /// DAA height at which vesting begins.
    pub start: u64,
    /// Number of DAA score units over which the full amount vests.
    pub duration: u64,
    /// Total amount to vest (in sompi).
    pub total_amount: u64,
    /// Amount already released (cumulative, in sompi).
    pub released: u64,
}

impl VestingSchedule {
    /// Create a new `VestingSchedule`.
    ///
    /// Returns `Err(VestingError::DurationZero)` if `duration == 0`.
    pub fn new(
        beneficiary: [u8; 32],
        start: u64,
        duration: u64,
        total_amount: u64,
    ) -> Result<Self> {
        if duration == 0 {
            return Err(VestingError::DurationZero);
        }
        Ok(Self {
            beneficiary,
            start,
            duration,
            total_amount,
            released: 0,
        })
    }

    /// Returns the DAA height at which vesting ends.
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.duration)
    }

    /// Returns the total vested amount at `current_daa` (cumulative, not
    /// net of prior releases).
    ///
    /// - Before `start`: 0
    /// - After `end`: `total_amount`
    /// - Between: linear interpolation
    pub fn vested_amount(&self, current_daa: u64) -> u64 {
        if current_daa < self.start {
            return 0;
        }
        let elapsed = current_daa.saturating_sub(self.start);
        if elapsed >= self.duration {
            return self.total_amount;
        }
        // Linear: total * elapsed / duration (integer arithmetic, rounds down)
        (self.total_amount as u128)
            .saturating_mul(elapsed as u128)
            .checked_div(self.duration as u128)
            .unwrap_or(0)
            .min(self.total_amount as u128) as u64
    }

    /// Returns the amount currently releasable (vested but not yet released).
    pub fn releasable(&self, current_daa: u64) -> u64 {
        self.vested_amount(current_daa)
            .saturating_sub(self.released)
    }

    /// Release all currently releasable tokens.
    ///
    /// Returns `(updated_schedule, released_amount)`.
    ///
    /// Returns `Err(VestingError::NothingToRelease)` if `releasable == 0`.
    ///
    /// Callers must:
    /// 1. Verify the authorising key is the `beneficiary`.
    /// 2. Persist the returned `VestingSchedule`.
    /// 3. Transfer `released_amount` sompi to the beneficiary.
    pub fn release(&self, current_daa: u64) -> Result<(Self, u64)> {
        let amount = self.releasable(current_daa);
        if amount == 0 {
            return Err(VestingError::NothingToRelease);
        }
        let updated = Self {
            released: self.released.saturating_add(amount),
            ..self.clone()
        };
        Ok((updated, amount))
    }
}
