# kcp-governance

> **Pre-production, unaudited, testnet-only.**

DAG-native governance primitives — the `Governor` equivalent for Kaspa. Kaspa's
DAG has no globally sequential block numbers and no on-chain token-weighted
voting, so this crate uses **DAA heights as the clock** and **k-of-n signatory
approval** as the voting mechanism.

Part of the `kaspa-compliance-patterns` workspace, targeting the
[Toccata](https://github.com/kaspanet/rusty-kaspa) hardfork.

## Status

**Value types plus an opt-in anchoring binding.** `proposal`, `vote`, `action`
and `governor` are plain Rust structs with no UTXO or covenant binding, so the
state machine stays offline and testable. The `lineage` module maps a
`GovernorState` run onto a `kcp-sealed-lineage` append-only lineage and
re-verifies it **off-chain**; on the default anchoring path consensus does not
introspect the payload at all. A Governor covenant that makes consensus enforce
the governance *rules* (quorum, timelock, signatory legality) does not exist —
see the threat model below and `KNOWN-ISSUES.md`.

| Module | EVM equivalent |
|---|---|
| `proposal::GovernanceProposal` | `Governor` proposal (content-addressed id, DAA voting window) |
| `vote::MultiSigVote` | `GovernorVotes` — k-of-n signatories, **not** token-weighted |
| `action::TimelockAction` | `TimelockController` — post-pass delay in DAA heights |
| `governor::GovernorState` | `Governor` lifecycle (approve → schedule → execute / cancel) |
| `lineage` | no EVM equivalent — sealed-lineage anchoring binding |
| token snapshots (`IVotes`) | deferred — no KRC20-equivalent with snapshotted balances exists |

## Usage

The module docs are the reference (`cargo doc -p kcp-governance --open`);
`src/lineage.rs` carries the full enforcement boundary. For a runnable
walkthrough — anchoring a governor run and auditing it offline:

```sh
cargo run -p kcp-governance --example governance_lineage
```

## Threat model

> **Pre-production, unaudited, testnet-only; testnet evidence is perishable.**
> This section distils what the crate documents. It is **not a security audit**
> and not an assurance that the properties below hold.

**Assets** — the integrity of a governance decision: that a proposal passed only
with genuine approvals from authorised signatories, that the timelock actually
elapsed before execution, and that the recorded history of a run cannot be
rewritten after the fact.

**Attacker capabilities (assumed)** — whoever drives the state machine. They
call `approve`, `refresh_status`, `schedule_action` and `execute`, and they
supply `current_height` on every one of those calls. If the run is anchored,
the **spender is the adversary** for the lineage: the publisher key holder
chooses each payload and when to append. A separate adversary is the
**discloser**, who hands an auditor a set of `(state, blind)` bundles of their
choosing.

**What consensus enforces** — on the default path, **nothing about governance**.
`vote::approve` records a key in a list; **no signature is verified anywhere in
this crate** — the caller asserts that a signatory approved. Anchoring through
`kcp_sealed_lineage::tx::{create,append}_lineage_tx` writes plain
pay-to-address UTXOs: consensus requires the publisher's signature to spend and
prevents the chain forking (a UTXO spends once), but does not read the payload.
Consensus enforces the lineage *structure* (L-1…L-4) only for runs anchored
under the separate covenant-id-bound chain `[KCP-SL-003]`, and **no library API
— including this one — wires a governance payload into it**; that is a manual
step. Even then, consensus enforces the lineage structure, never the governance
rules.

**What this assumes / trusts off-chain** — the caller for honest heights (a
lying `current_height` opens or closes a voting window early); the caller for
signatory authenticity, since approvals are unsigned key records; the auditor to
fetch payloads from the chain **independently of whoever discloses the states**,
because `verify_governor_lineage` on its own proves only that a disclosed
bundle is internally consistent with its payloads. What that verifier does check
off-chain: the L-1…L-4 chain invariants, that `lineage_id` derives from the
governor's immutable config (so a config swap is detectable), that each
commitment seals the full canonical `GovernorState`, and that the status lattice
is well-formed and each disclosed state self-consistent.

It also inherits `kcp-sealed-lineage`'s commitment assumptions, and they bite
harder here: a `GovernorState` is **very low entropy** (a status, a height, a
signatory list), so the blind is the only thing hiding it — and `anchor_step`
accepts any 32 bytes, including all-zero or a blind reused across steps. With a
weak or reused blind an observer can confirm a guessed state by recomputing the
commitment, including **who approved**. Two structural properties come with the
anchoring covenant too: its state has no `commitment` field, so consensus never
binds the seal on any path; and it leaves `publisherPk` free, so whoever anchors
a governor run can silently rotate the anchoring key mid-run.

**Known limits and non-goals** — every field of `GovernorState` is `pub` and
`MultiSigVote::approve` is window-blind, so a caller can record an approval
directly on `state.vote` after the voting deadline. `refresh_status` therefore
treats `Rejected` as terminal alongside `Cancelled` and `Executed` (regression
test: `rejected_is_terminal_even_if_approvals_arrive_after_the_deadline`) — but
that is a guard on one flip, not access control. A holder of the struct can set
any field to any value; nothing here authenticates a caller. Beyond that, the
crate's name is stronger than its enforcement: this is a governance *bookkeeping* model with a tamper-evident
audit trail, not on-chain governance. Quorum, timelock elapse and signatory
legality are never enforced by consensus; the verifier's per-state check
rejects a self-contradictory disclosed state, it does not prove the rules were
followed. DAA heights are an approximate clock — the DAG does not strictly
serialise heights, so exact-height comparisons are not reliable for
time-critical decisions. No token-weighted voting, no delegation, no
vote-privacy, no quorum change mid-run, and no execution: `execute()` marks a
status, it does not move funds or spend anything. `cancel()` is unconditional
except from `Executed` — the model has no notion of *who* is entitled to call
it, or any other method. secp256k1 Schnorr keys are
carried as opaque 32-byte values and are not curve-checked. Unaudited; not for
mainnet value.

## Licence

MIT — Stichting Kii Foundation
