# PrivateVoting

**A cross-layer (vProg) anonymous-ballot pattern — Portrait vProg catalogue.**

*Pre-production, unaudited, testnet-only. Perishable evidence. Local-only —
nothing settled to kaspanet.*

> **STATUS — emit-verified template (M3).** The covenant compiles via `silverc`
> (exit 0); `portrait atelier-build` emits a RISC Zero guest that builds the
> 104-byte CSCI journal, advances `seq`, and hashes the new tally state into
> `new_state_hash`. The actual predicate — what the guest proves (here:
> eligibility membership + one-vote nullifier-freshness + the tally fold over the
> hidden ballot) — is **authored by the developer into the guest, NOT synthesised
> by the compiler**. The emitted guest reads the proposed `next_tally_root` but
> does not itself run the eligibility/one-vote/fold logic. **Not settled live** —
> only `CsciInstrument` has settled live on TN10.

An electorate casts ballots into a running tally **without revealing who voted
for what**. Each accepted ballot must satisfy two predicates a pure covenant
cannot check without unmasking the voter:

- **Eligibility** — the voter is a member of the committed registry (the
  `registrar` attests a root of eligible voters), proven without revealing which
  member.
- **One-vote** — a nullifier derived from the voter's secret is fresh (the voter
  has not already voted this round), proven without linking the nullifier to an
  identity.

The new running tally is a **fold** of the prior tally with the hidden ballot.
None of {voter identity, ballot choice, nullifier set} can live on chain without
breaking ballot secrecy, so the predicate is discharged in a RISC Zero zk-STARK
guest (the vProg); the covenant records a monotonic-seq tally transition **bound
to that proof**.

This follows the proven CSCI cross-layer shape (see
`../../state/CsciInstrument`, which settled live on TN10 with the STARK verified
in consensus). It is built **source-only** inside that proven shape.

## The pair

| Layer | Role | What it does |
|---|---|---|
| **L1 covenant** | `settle` (`#[covenant]`) | Settles one tally transition on chain; self-enforces the structural state machine + the covenant-id binding. |
| **vProg guest** | `vote_rule` (NonCovenant) | The off-chain predicate proven by the STARK; checks eligibility + one-vote, folds the hidden ballot, commits the new `tally_root` + `seq` into the journal's `new_state_hash`. |

Portrait emits both from this one source:
- `portrait engrave PrivateVoting.portrait` → `PrivateVoting.sil` (silverc exit 0)
- `portrait atelier-build PrivateVoting.portrait` → `privatevoting_guest_main.rs`

## Why two committed keys (`owner` vs `registrar`)

- `owner` is the **on-chain settlement authority** — who may post a transition.
  `settle` is authorized against the committed `owner` key (never a caller arg).
- `registrar` is the committed **authority over eligibility** — its identity is
  folded into the guest's membership check via the registry root. It is committed
  state, so it cannot be swapped by a caller argument.

## What the guest proves

A voter is eligible (member of the committed registry) **and** casts exactly one
vote (fresh nullifier) **without** revealing identity or choice, and the folded
`next_tally_root` is the correct update of the prior `tally_root`. The journal
commits the advanced `tally_root` + `seq` inside the 104-byte CSCI journal:

```text
covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]
```

## What the covenant enforces (on chain)

| Rule | Enforced by |
|---|---|
| Tally advance (`next.tally_root == next_tally_root`) | `tally_root: next_tally_root` in the transition |
| Monotonic CSCI seq (`next.seq == prev.seq + 1`) | `seq + 1` in the transition |
| Covenant-id binding (journal's covenant_id == this covenant) | `require(proof_cov_id == OpInputCovenantId(0))` — emitted because the role has a vProg companion (`has_vprog`) |
| Owner authorization | `require(checkSig(auth, prev_states[0].owner))` against the committed owner key |
| Singleton, no fork / no burn | covenant transition mode (structural) |

## Files

- `PrivateVoting.portrait` — the Portrait source (what a builder writes).
- `PrivateVoting.sil` — engraved silverscript covenant (silverc exit 0).
- `PrivateVoting.json` / `PrivateVoting_ctor.json` — compiled script + genesis ctor.
- `privatevoting_guest_main.rs` — the emitted RISC Zero guest main (M3 skeleton).

## Honest scope

The tag-0x21 ZK **verification** of the STARK proof is an engine-level operation
(`OpZkPrecompile` / `kcp-pq-anchor`), **not** a silverscript op. silverscript here
enforces the state-machine rules + the covenant-id binding; the proof-bytes
verification is layered by the settlement harness, which feeds the journal's
`covenant_id` in as `proof_cov_id`. The ballot-secrecy + eligibility + one-vote
**soundness** rests on the RISC Zero guest + the verifier, **not** on Portrait —
Portrait only carries the structural tally state machine on chain. The
covenant-side checks are structural/relational (no SMT). This is an emit-verified
template, not a pattern individually settled live; live settlement reuses the
proven CSCI harness.
