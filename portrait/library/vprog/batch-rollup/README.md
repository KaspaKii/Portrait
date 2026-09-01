# vprog/BatchRollup

A **vProg pattern** (not a pure covenant): aggregate **N** state transitions into
**one** on-chain settlement, rollup-style. An off-L1 RISC Zero guest **proves**
that folding N transitions over `prev_root` yields `next_root`; the settlement
covenant advances the committed `root` to `next_root` in a single spend that is
**bound to that proof** via the covenant-id binding the vProg companion emits.

This is the cross-layer (vProg) shape — the same proven shape as
`library/state/CsciInstrument`, generalized to "aggregate N → 1".

**Status:** 🟡 emit-verified template — pre-red-team, testnet-only, not audited,
not mainnet-safe. NOT individually settled live (see Honest scope).

> **STATUS — emit-verified template (M2 skeleton).** The covenant compiles via
> `silverc`; `portrait atelier-build` emits a RISC Zero guest **SKELETON** that
> builds the 104-byte journal and advances `seq`. The actual predicate — what the
> guest proves (here, that folding N transitions over `prev_root` yields
> `next_root`) — is **authored by the developer into the guest, NOT synthesised by
> the compiler**. The emitted guest reads the proposed new values but does not
> itself prove the fold. **Not settled live** — only `CsciInstrument` has settled
> live on TN10.

## The pair

| Entrypoint | Layer | Role |
|---|---|---|
| `settle` | Covenant (L1) | Advances `root → next_root` and `seq → seq + batch_count`, authorized against the COMMITTED operator key, bound to the proof. |
| `apply_batch` | vProg (off-L1) | NonCovenant companion. Carries no `#[covenant]` attribute → Atelier emits its RISC Zero guest `main`. Its presence flips `has_vprog`, which emits the covenant-id binding `require(proof_cov_id == OpInputCovenantId(0))` into `settle`. |

## State

| Field | Type | Meaning |
|---|---|---|
| `operator` | `pubkey` | Committed operator key. The settle authority. |
| `root` | `bytes32` | Committed rollup state root (the aggregate the proof advances). |
| `seq` | `int` | Batch sequence number (genesis = 0). Advances by the batch size. |

## Lifecycle

```
live --settle(operatorSig, next_root, batch_count)  [batch_count >= 1]
     --> live   (root := next_root; seq := seq + batch_count)
```

## Journal (the proven CSCI shape)

```
covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]
```

The emitted guest reads `next_root`, derives `new_state_hash` over
`(operator, next_root, seq)`, and commits the 104-byte journal.

## Why cross-layer

A pure covenant would have to **replay all N transitions on chain** to advance the
root — infeasible / wasteful. The covenant instead verifies a single STARK that
the fold was applied correctly, settling the whole batch in one spend.

## Why it's safe by shape

- **Committed-operator authorisation (C2).** `settle` `checkSig`s against the
  COMMITTED `operator` state key, never a caller-supplied pubkey.
- **Covenant-id binding.** `require(proof_cov_id == OpInputCovenantId(0))` ties
  the settled spend to a proof whose journal commits THIS covenant's id.
- **Forward batch progress.** `require(batch_count >= 1)` — a batch advances the
  rollup forward; it cannot stall or rewind the sequence.
- **No global state, no reentrancy.** State lives in this one UTXO.

## Honest scope

- **Emit-verified template, not settled live.** The covenant compiles via
  `silverc` (engrave → exit 0) and the guest is emitted (`atelier-build`), within
  the same proven harness as the live CSCI instrument. Live settlement of this
  pattern would use that *same* harness; it has NOT been individually settled.
- **Soundness of the fold rests on the guest + verifier, not on Portrait.** "N
  transitions fold `prev_root → next_root`" is proven by the RISC Zero guest and
  checked by the tag-0x21 verifier. silverc here enforces only the on-chain
  state-machine rules (operator auth, forward progress, covenant-id binding); the
  STARK verification is layered by the settlement harness, which feeds the
  journal's `covenant_id` in as `proof_cov_id`.
- **Two sequence counters.** The on-chain `seq` advances by `batch_count` (the
  batch landed in one settlement). The emitted guest's journal `seq` field
  advances by exactly one per Atelier's CSCI journal convention. `monotonic_seq`
  is therefore NOT declared on this covenant — the on-chain seq advances by N.
- **Semantic checks are structural/relational, not an SMT solver.**
- Pre-production, unaudited, testnet-only, perishable.

## Files

- `BatchRollup.portrait` — the canonical source (role/lifecycle/invariant +
  vProg companion).
- `BatchRollup.sil` — the emitted Silverscript settlement covenant.
- `BatchRollup_ctor.json` — the emitted CTOR JSON consumed by `silverc --ctor`.
- `BatchRollup.json` — the `silverc`-compiled script.
- `batchrollup_guest_main.rs` — the Atelier-emitted RISC Zero guest `main` that
  builds the 104-byte journal.

## Reproduce

```sh
cd portrait
cargo run --bin portrait -- check         ../library/vprog/batch-rollup/BatchRollup.portrait
cargo run --bin portrait -- engrave       ../library/vprog/batch-rollup/BatchRollup.portrait
cargo run --bin portrait -- atelier-build ../library/vprog/batch-rollup/BatchRollup.portrait
```
