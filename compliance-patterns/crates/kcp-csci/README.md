# kcp-csci

> **Pre-production, unaudited, testnet-only.**

Covenant-Settled Compliance Instrument (CSCI) — **scaffold**. The off-chain data
structures and journal encoding for the cross-layer pattern described in
[`docs/FLAGSHIP-DESIGN.md`](../../docs/FLAGSHIP-DESIGN.md): a KTT-shaped
instrument whose transfer rules are checked inside a RISC Zero vProg, with the
resulting succinct STARK verified in consensus by the KIP-16 tag-0x21
precompile.

## Status

Scaffold. **The CSCI covenant locking script has not been authored.**
`covenant/csci-intent.sil` is documentary and explicitly not compilable — it
records the intended on-chain logic, which needs `sha256()` and ZK-verify
builtins that SilverScript does not have. What the crate provides today:

| Piece | What it is |
|---|---|
| `state::CsciState` | 50-byte state — `KttState` (42 B) + `seq` (8 B LE), plus `rule_hash` and `covenant_id` |
| `state::CsciStateTransition` | off-chain transfer check: `seq + 1`, immutable `rule_hash`, `0 < amount ≤ balance` |
| `binding::{KovId, CovIdBinding}` | the 104-byte journal layout `covenant_id ‖ new_state_hash ‖ rule_hash ‖ seq` and its parser |
| `redeem::{build_csci_redeem, csci_proof_fields}` | a thin wrapper over `kcp_pq_anchor::build_pq_anchor_redeem` |

## Usage

Start from the module docs (`cargo doc -p kcp-csci --open`) and
`tests/csci_smoke.rs`, which exercises the state encoding, the transition rules
and the journal layout end to end:

```sh
cargo test -p kcp-csci
```

## Threat model

> **Pre-production, unaudited, testnet-only; testnet evidence is perishable.**
> This section distils what the crate documents. It is **not a security audit**
> and not an assurance that the properties below hold.

**Assets** — the instrument's balance, the sequence integrity of its transfer
history, and the compliance claim itself: that every transfer satisfied the rule
set committed as `rule_hash`.

**Attacker capabilities (assumed)** — on a UTXO chain the **spender is the
adversary**: the holder builds the spending transaction, chooses the destination
and the outputs, and chooses which journal to present. Because the redeem script
is assembled by `kcp-pq-anchor`, **any observer of a spend** is also an
adversary: the seal, journal digest and image id are pushed in clear. A
developer-side adversary chooses the guest, and therefore what the "compliance"
predicate actually checks.

**What consensus enforces** — exactly and only what `kcp-pq-anchor` enforces:
the tag-0x21 precompile verifies the RISC Zero succinct STARK against the pinned
`image_id`, `control_id` and journal digest. Consensus does **not** enforce any
CSCI-specific rule — not `seq` monotonicity, not `rule_hash` immutability, not
the `covenant_id` instance binding, and not that the new on-chain state matches
the one in the journal. Those are steps 2–6 of `covenant/csci-intent.sil`, which
is documentary and does not compile. `CsciStateTransition::transfer` checks
`seq + 1`, rule-hash equality and the balance bound **in Rust, off-chain**.

**What this assumes / trusts off-chain** — the vProg guest is the entire
compliance engine, it is developer-authored, and this crate never sees it: the
engine proves a program with that image id ran and emitted that journal, nothing
about what it verified. The rule set behind `rule_hash` lives off-chain and is
not published by this crate. The caller must compute the journal the way the
guest computes it, pin the right `image_id`, and validate the transition before
building a redeem — `csci_proof_fields` derives the journal from a
`CsciStateTransition`, but nothing forces a caller to use it, and
`build_csci_redeem` will assemble whatever fields it is handed.

**Known limits and non-goals** — the name says "covenant-settled"; today **no
covenant settles it**. The crate inherits every limit of `kcp-pq-anchor`,
including its **budget ceiling**: a version-0 input can commit at most
`255 × 100_000 + 9_999 = 25,509,999` script units (`SigopCount` is a `u8`, so
there is no larger budget), and the reference proof already measures 25,446,182
of them — 0.25% headroom. A CSCI guest is *not* the reference guest, so measure
the real redeem with `kcp_pq_anchor::sigop::measure_pq_anchor_units` before
funding anything (the helper needs `--features wrpc`, since measuring runs the
real consensus VM): an over-budget spend can never be submitted, and because the
proof fields sit inside the P2SH address there is no fallback path — the funds
are permanently unrecoverable. It also inherits the proof-only bearer lock: no `OP_CHECKSIG`, no binding to the
spending transaction, so a valid seal spends to any destination and can be
replayed against any other UTXO under the identical script unless the guest
commits transaction-binding data inside the journal. This crate ships no
live-settlement path and claims no testnet transaction of its own. It is
`publish = false` and depends on `kcp-ktt-token` and `kcp-pq-anchor` by path.
secp256k1 Schnorr elsewhere in the flow is not post-quantum, whatever the STARK
is. Unaudited; not for mainnet value.

## Licence

MIT — Stichting Kii Foundation
