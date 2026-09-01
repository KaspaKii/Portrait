# kcp-governance

`kcp-governance` provides DAG-native governance primitives: a content-addressed
proposal, k-of-n approval tracking, a post-pass timelock, and a combined
`GovernorState` lifecycle. It is the `Governor` + `TimelockController` equivalent
for the Kaspa BlockDAG.

DAA heights serve as the on-chain clock. **Kaspa's DAG does not have
globally-sequential block numbers.** Use DAA heights as *approximate* clocks.

## Running a governance cycle

```rust
use kcp_governance::{
    action::TimelockAction, governor::GovernorState,
    proposal::GovernanceProposal, vote::MultiSigVote,
};

// 1. Create vote and apply approvals before building GovernorState
let mut vote = MultiSigVote::new(vec![key_a, key_b, key_c], 2)?;
vote.approve(key_a)?;
vote.approve(key_b)?; // quorum reached

// 2. Build GovernorState with quorum already met
let proposal = GovernanceProposal::new("fund auditor", current_height, voting_deadline)?;
let action   = TimelockAction::new(500)?;
let mut gov  = GovernorState::new(proposal, vote, action, current_height);

// 3. Advance past deadline → Passed
gov.refresh_status(voting_deadline + 1);

// 4. Schedule and execute
gov.schedule_action(voting_deadline + 1)?;
gov.execute(voting_deadline + 502)?; // after 500-DAA delay
```

## A note on no token-weighted voting

`MultiSigVote` uses a fixed signatory set (k-of-n). Token-weighted voting is
**deferred** until a KRC20-equivalent with snapshotted balances exists on Kaspa
mainnet. This is an intentional honest limitation.

**Verify Schnorr signatures** before calling `MultiSigVote::approve()` — the vote
tracker records approvals by key but does NOT verify cryptographic signatures.

## Anchoring the governor lifecycle

The opt-in `kcp_governance::lineage` module maps a `GovernorState` run onto a
`kcp-sealed-lineage` append-only lineage — one sealed event per lifecycle
snapshot, event-classed `GENESIS` (seq 0) / `CLOSE` (a terminal status) /
`APPEND` (otherwise). Each event's `lineage_id` is derived from the governor's
**immutable config** (proposal id, voting window, signatory set, threshold,
timelock delay); each commitment seals the **full canonical `GovernorState`**.

```rust
use kcp_governance::lineage::{anchor_step, governor_lineage_id, verify_governor_lineage, AnchoredStep};

let lineage_id = governor_lineage_id(&gov)?;
let payload = anchor_step(&gov, lineage_id, seq, t_bucket, &blind)?;
// … collect AnchoredStep { payload, state, blind } per snapshot …
verify_governor_lineage(&steps)?; // structure + identity + commitments + lattice
```

A worked, offline auditor demo is in
`crates/kcp-governance/examples/governance_lineage.rs`
(`cargo run -p kcp-governance --example governance_lineage`).

**What this proves — and what it does not.** `verify_governor_lineage` runs
off-chain: it proves a disclosed `(state, blind)` bundle is internally
consistent with its payloads (chain invariants, config-derived identity,
full-state commitments, and a well-formed lifecycle lattice). Obtain the
payloads themselves from the lineage independently of whoever discloses the
states — they are the trust anchor.

On the **default** anchoring path a sealed lineage is written as plain
pay-to-address UTXOs, so consensus does not introspect the payload and the
chain invariants are validated off-chain only (as the `kcp-sealed-lineage` and
`kcp-transferable-record` modules state). Consensus rejects a malformed
successor **only if** the run is anchored under the separate covenant-bound
sealed-lineage chain (`[KCP-SL-003]`, demonstrated on testnet-10), which no
library API auto-wires. Even then it enforces the lineage *structure*, not the
governance *rules*: quorum before `Passed`, timelock elapse before `Executed`,
and signatory legality all remain off-chain. A dedicated Governor covenant that
binds those on-chain is the remaining gap (see `KNOWN-ISSUES.md`).

## Extensions

- **Vault integration** — use the same key set as the governance signatories in a `kcp-vault` P2SH. See `examples/governance-demo`.

→ API reference: [`GovernanceProposal`], [`MultiSigVote`], [`TimelockAction`], [`GovernorState`], [`proposal_id`]
