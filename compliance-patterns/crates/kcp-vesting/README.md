# kcp-vesting

> **Pre-production, unaudited, testnet-only.**

Linear DAA-height vesting schedule for the Kaspa BlockDAG.

EVM equivalent: `VestingWallet` (Solidity pattern-library v5 shape).

Uses Kaspa DAA heights (Blue Score) as the on-chain clock — approximately one
unit per second at the 1 BPS target rate.

## Quick start

```rust
use kcp_vesting::schedule::VestingSchedule;

// 1,000,000 sompi vesting linearly from DAA height 100,000 over 50,000 DAA units
let schedule = VestingSchedule::new(
    beneficiary_xonly_key,
    100_000,  // start DAA height
    50_000,   // duration in DAA units (~14 hours at 1 BPS)
    1_000_000, // total sompi
)?;

let releasable = schedule.releasable(current_daa);
if releasable > 0 {
    let (updated_schedule, amount) = schedule.release(current_daa)?;
    // persist updated_schedule, transfer `amount` sompi to beneficiary
}
```

## Before live use

- Replace synthetic beneficiary keys with real Schnorr x-only pubkeys.
- Use real DAA heights from a connected `kaspad` node.
- Persist the `VestingSchedule` after each `release()` call — the crate is
  stateless and does not persist for you.

## Licence

MIT — Stichting Kii Foundation
