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

## Threat model

> **Pre-production, unaudited, testnet-only.** This section distils what the
> crate documents. It is **not a security audit** and not an assurance that the
> properties below hold.

**Assets** — the vested allocation and its release schedule; the beneficiary's
expectation that nothing can be released early.

**Attacker capabilities (assumed)** — whoever calls the API. They supply
`current_daa` (any value they like), decide whether to persist the updated
schedule, and decide whether the released sompi is actually transferred. On a
UTXO chain the spender is normally the adversary — but here no UTXO is
involved, so the adversary is simply the caller.

**What consensus enforces** — **nothing.** This crate is a pure offline value
type: no script, no covenant, no UTXO, no signature check. Nothing it computes
is visible to, or enforced by, the Kaspa engine. `beneficiary` is carried as 32
bytes and is never checked against a signature or validated as a curve point.
To make a schedule enforceable on-chain, lock the funds under a `kcp-vault`
DAA-height CLTV covenant — that is where the consensus-enforced part lives, in
a different crate. Note that a vault timelock carries a **single deadline**: it
cannot express gradual release, so an enforced schedule means pre-splitting the
value into one timelocked UTXO per tranche.

**What this assumes / trusts off-chain** — the caller for the clock (a real DAA
height from a synced `kaspad`, not a synthetic one — a caller that passes an
inflated height releases early and the schedule cannot tell); the caller to
verify the authorising key really is `beneficiary` before paying out; the caller
to persist the returned schedule after **every** `release()`, since the crate is
stateless and re-using the pre-release value replays the release; the caller to
actually make the transfer.

**Known limits and non-goals** — `VestingSchedule` derives `Deserialize` with
**no validation on the way in**, so the `duration == 0` that `new()` rejects
round-trips straight through `serde` — and a zero-duration schedule reports
everything vested the instant `current_daa >= start`. Validate after
deserialising, or reconstruct through `new()`. No cliff, no revocation, no
pause, no multi-beneficiary support: vesting is strictly linear from `start`
over `duration`, and `vested_amount` rounds down. DAA heights are an *approximate*
clock — the DAG does not strictly serialise heights across concurrent blocks, so
a schedule boundary is not a precise instant. There is no custody here: this
crate never holds, moves, or can withhold funds, and cannot stop a payer who
ignores it. Unaudited; not for mainnet value.

## Licence

MIT — Stichting Kii Foundation
