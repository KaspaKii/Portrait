# kcp-vesting

> **Pre-production, unaudited, testnet-only.**

`kcp-vesting` provides a linear time-based token release schedule using Kaspa
DAA heights (Blue Score) as the on-chain clock. It is the `VestingWallet`
equivalent for the Kaspa BlockDAG.

## Constructing a vesting schedule

```rust
use kcp_vesting::schedule::VestingSchedule;

// 1,000,000 sompi vesting linearly from DAA height 100,000
// over 86,400 DAA units (~24 hours at 1 BPS target)
let schedule = VestingSchedule::new(
    beneficiary_xonly_key,
    100_000,    // start DAA height
    86_400,     // duration in DAA units
    1_000_000,  // total sompi
)?;

// Check what's available
let releasable = schedule.releasable(current_daa);

// Release and persist
if releasable > 0 {
    let (updated, amount) = schedule.release(current_daa)?;
    // persist `updated`, transfer `amount` sompi to beneficiary
}
```

## A note on DAA heights vs unix-seconds

Kaspa's DAA blue score advances roughly once per second at the 1 BPS target
rate, but the actual rate is governed by the difficulty-adjustment algorithm
and can deviate from wallclock time. For time-sensitive vesting:

- Use DAA heights as an approximate clock, not a precise timestamp.
- Add a safety margin to `duration` if the vest-end time is critical.
- Cross-reference the DAA height against a real-time source if precision matters.

**It is critical** that `start` and `duration` are set using real DAA heights
from a connected `kaspad` node — not synthetic values from tests.

## A note on persistence

`VestingSchedule` is a pure value type. The `release()` method returns a new
schedule; callers must persist it after each release, or the `released` counter
resets and the beneficiary can claim the same amount again.

## Extensions

- **Cliff vesting** — set `start` to the cliff end; amounts before `start` are
  always zero, so the cliff is implicit.
- **Governance-controlled vesting** — gate `release()` behind a `kcp-governance`
  vote to require committee approval for each release.
- **On-chain enforcement** — anchor the persisted `VestingSchedule` to a
  `kcp-sealed-lineage` for a tamper-evident release audit log.

→ API reference: [`VestingSchedule`], [`releasable`], [`release`], [`vested_amount`]
