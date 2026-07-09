# ConfidentialTransfer

**A cross-layer (vProg) hidden-amount transfer pattern — Portrait vProg catalogue.**

*Pre-production, unaudited, testnet-only. Perishable evidence. Local-only —
nothing settled to kaspanet.*

> **STATUS — emit-verified template (M2 skeleton).** The covenant compiles via
> `silverc`; `portrait atelier-build` emits a RISC Zero guest **SKELETON** that
> builds the 104-byte journal and advances `seq`. The actual predicate — what the
> guest proves (here, that the hidden transfer opens, is non-negative, and
> conserves value: `sum(inputs) == sum(outputs)`) — is **authored by the developer
> into the guest, NOT synthesised by the compiler**. The emitted guest reads the
> proposed new commitment but does not itself prove conservation. **Not settled
> live** — only `CsciInstrument` has settled live on TN10.

A confidential-transfer UTXO carries only a `commitment` (bytes32) to a hidden
balance/transfer state plus a monotonic `seq`. The actual amounts are **never** on
chain. A RISC Zero STARK (the `transfer_rules` vProg companion) proves off-L1 that
a transfer is valid over those hidden amounts; the on-chain `settle` covenant
advances the committed state, **bound to that proof**.

This follows the proven CSCI cross-layer shape (see
`../../state/CsciInstrument`, which settled live on TN10 with the STARK verified in
consensus). It is built **source-only** inside that proven shape.

## The pair

| Layer | Role | What it does |
|---|---|---|
| **L1 covenant** | `settle` (`#[covenant]`) | Adopts the new committed `commitment`, advances `seq` by one, authorized against the COMMITTED owner key, bound to the proof via the covenant-id binding. The amount is never an argument. |
| **vProg guest** | `transfer_rules` (NonCovenant) | The off-chain predicate proven by the STARK; opens the sender commitment, checks `amount >= 0` and `sum(inputs) == sum(outputs)`, commits the new state-commitment into the journal's `new_state_hash`. |

Portrait emits both from this one source:
- `portrait engrave ConfidentialTransfer.portrait` → `ConfidentialTransfer.sil` (silverc exit 0)
- `portrait atelier-build ConfidentialTransfer.portrait` → `confidentialtransfer_guest_main.rs`

## State

| Field | Type | Meaning |
|---|---|---|
| `owner` | `pubkey` | Committed owner key. The settle authority. |
| `commitment` | `bytes32` | Hiding commitment to the balance/transfer state. |
| `seq` | `int` | Monotonic CSCI sequence number (genesis = 0). |

## The journal (the proven CSCI shape)

```text
covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]
```

Here `new_state_hash` is the NEW state-commitment after the confidential transfer
— the same field `settle` adopts as `commitment` on chain.

## What the covenant enforces (on chain)

| Rule | Enforced by |
|---|---|
| Owner authorization | `require(checkSig(auth, owner))` against the committed owner key |
| Monotonic CSCI seq (`next.seq == prev.seq + 1`) | `seq + 1` + `invariant monotonic_seq` |
| State advance | adopt `next_commitment` as the new `commitment` |
| Covenant-id binding (journal's covenant_id == this covenant) | `require(proof_cov_id == OpInputCovenantId(0))` — emitted because the role has a vProg companion (`has_vprog`) |

The covenant never sees the amount; confidentiality + conservation soundness live
in the guest, not on chain.

## Files

- `ConfidentialTransfer.portrait` — the Portrait source (what a builder writes).
- `ConfidentialTransfer.sil` — the emitted silverscript (verify against the real silverc).
- `confidentialtransfer_guest_main.rs` — the emitted RISC Zero guest main (the vProg template).
- `README.md` — this file.

## Honest scope

- **Emit-verified template, not settled live.** The covenant compiles via silverc
  (exit 0) and the guest is emitted as well-formed Rust. Live settlement of this
  pattern reuses the *same* proven CSCI harness; only CSCI itself has been settled
  live on TN10.
- **The emitted guest is an M2 skeleton.** It builds the 104-byte journal and
  advances seq; the confidentiality + non-negativity + conservation predicate is
  authored by the developer into the guest, NOT synthesised by the compiler.
- **The covenant-side checks are structural/relational (no SMT).** The soundness
  of "the transfer opens, is non-negative, and conserves value" rests on the RISC
  Zero guest + the tag-0x21 verifier, **not** on Portrait.
- **The tag-0x21 STARK verification is an engine-level operation**, not a
  silverscript op. The settlement harness feeds the journal's covenant_id in as
  `proof_cov_id`.
