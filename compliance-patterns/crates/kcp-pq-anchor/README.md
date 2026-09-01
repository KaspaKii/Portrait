# kcp-pq-anchor

> **Pre-production, unaudited, testnet-only.**

KIP-16 tag-0x21 post-quantum credential anchor for the Kaspa BlockDAG.
Script assembly helpers for RISC Zero succinct STARK proofs — the first
post-quantum credential anchor in any Kaspa library.

This crate assembles the verifiable redeem script. Proof generation requires
RISC Zero v3.0.5 running your own guest program. The library solves the hardest
part: correct KIP-16 tag-0x21 opcode assembly.

## Quick start

```rust
use kcp_pq_anchor::{
    anchor_script::{build_pq_anchor_redeem, PqAnchorScriptFields},
    journal_spec::JournalSpec,
    // measure_pq_anchor_units / fits_pq_verify_budget need `--features wrpc`
    // (they run the real consensus VM); the constants are always available.
    sigop::{fits_pq_verify_budget, measure_pq_anchor_units, sigop_count_for_pq_verify},
};

let journal = JournalSpec::PairedAttestation {
    attestation_id: my_attestation_id,
    spend_outpoint: my_outpoint_bytes,
}.journal_hash();

let script = build_pq_anchor_redeem(&PqAnchorScriptFields {
    claim: my_claim_bytes,
    control_index: 0,
    control_digests: my_control_digests_concat,
    seal: my_stark_seal,
    journal,
    image_id: MY_IMAGE_ID,
    control_id: MY_CONTROL_ID,
})?;

// MEASURE FIRST (needs `--features wrpc`): 255 is the maximum a u8 sigOpCount
// can express, and the reference proof already uses 99.75% of the budget it
// buys. A proof shape that does not fit can never be spent, and the funds are
// permanently unrecoverable — check BEFORE funding the address.
let units = measure_pq_anchor_units(&script)?;
assert!(fits_pq_verify_budget(units), "unbudgetable — do not fund this address");

// Then set sigOpCount = 255 when submitting the spending transaction
let sig_ops = sigop_count_for_pq_verify(); // 255 = the ceiling, not a measurement
```

## Key invariant: canonical hashfn push

The `hashfn` field (Poseidon2 = integer 1) **must** be pushed as a 1-byte data
push `[0x01, 0x01]` (a `0x01` push-length opcode followed by the byte `0x01`),
never as a numeric `OP_1` (0x51). The engine's `parse_hashfn` requires exactly a
1-byte data push, so a numeric `OP_1` would be **rejected**.
`build_pq_anchor_redeem` emits the correct 1-byte data push internally (see
`push_data(&mut script, &[HASHFN_POSEIDON2])` in `src/anchor_script.rs`).

## Threat model

> **Pre-production, unaudited, testnet-only; testnet evidence is perishable.**
> This section distils what the crate documents. It is **not a security audit**
> and not an assurance that the properties below hold.

**Assets** — whatever value is locked under the tag-0x21 anchor P2SH, and the
credibility of the credential claim the guest attests.

**Attacker capabilities (assumed)** — on a UTXO chain the **spender is the
adversary**: they assemble the redeem script and the spending transaction and
pick the destination. Additionally, **any observer of a spend** is an adversary
here, because the seal, journal digest, image id and control id are pushed in
clear and become public the moment the spend is mined. A developer-side
adversary can supply an `image_id` for a guest that proves nothing.

**What consensus enforces** — the KIP-16 tag-0x21 precompile
(`OpZkPrecompile`, `0xa6`, on the pinned v2.0.0 engine) verifies the RISC Zero
**succinct STARK** against the `image_id`, `control_id` and `journal` digest
carried in the script. Proof *validity* is genuinely in-consensus:
`tests/engine_accept.rs` runs a real proof through the consensus verifier and
shows a tampered journal or image id rejected. Encoding is load-bearing:
`hashfn` must be the 1-byte data push `[0x01, 0x01]` (a numeric `OP_1` is
rejected by the engine's `parse_hashfn`), and the spending transaction must
commit `sigOpCount = 255` (`sigop_count_for_pq_verify`) or the node rejects it.

*Evidence boundary.* That acceptance test is a **weaker tier than the covenant
crates'** script-VM proofs: it runs the redeem through `from_script` — a
standalone script with **no transaction, no P2SH wrap and no compute
commitment** — so it says nothing about transaction mass, KIP-9 storage mass,
standardness, or the budget the node would charge. `tests/budget_ceiling.rs`
adds the transaction-input tier for cost only. No live testnet settlement is
claimed by this crate.

**What this assumes / trusts off-chain** — the **guest predicate is
developer-authored, and this crate never sees it**. The engine proves that a
program with that image id ran and produced that journal; it says nothing about
whether the program checks anything meaningful. Pinning the right `image_id`
(and rebuilding it reproducibly), computing `journal` as the SHA-256 the guest
actually committed, and agreeing the `JournalSpec` layout on both sides are the
developer's responsibility. Proof generation is out of scope — RISC Zero v3.0.5
running your own guest.

**Known limits and non-goals** — **the budget ceiling is 0.25% away.** A
version-0 input commits its covenant budget as a `u8` `SigopCount`, so 255 is
the *maximum expressible* budget, capping a spend at
`255 × 100_000 + 9_999 = 25,509,999` script units. The shipped reference proof
measures **25,446,182** units through a real P2SH spend
(`tests/budget_ceiling.rs`) — **63,817 units of headroom**. A bigger seal, a
longer control-inclusion path, or a different guest can cross that line, and
then the spend can never be budgeted at all: the proof fields are inside the
redeem script and therefore inside the P2SH address, so there is no other
spending path and **the funds are permanently unrecoverable**. Measure your own
proof shape with `sigop::measure_pq_anchor_units` and check
`sigop::fits_pq_verify_budget` **before funding the address** — both need
`--features wrpc`, because measuring runs the real consensus VM (the default
build deliberately does not pull the engine). The constants
`sigop_count_for_pq_verify()` and `MAX_COMMITTABLE_SCRIPT_UNITS` are always
available, but a constant is not a measurement. Beyond that, the
redeem script is a **proof-only bearer lock**: it contains no `OP_CHECKSIG` and nothing binds it to the spending
transaction, so anyone holding a valid seal can spend to any destination, and
once a spend is public the same satisfier can be replayed against any other UTXO
locked under the identical script. Transaction binding must come from *inside*
the journal (for example the `spend_outpoint` field of
`JournalSpec::PairedAttestation`) and is therefore a guest-side obligation this
crate does not enforce. "Post-quantum" describes the STARK (FRI + Poseidon2, no
pairings); a flow that still funds, co-signs, or recovers with secp256k1 Schnorr
keys is not post-quantum end-to-end. Seals are large — mass, standardness and
fee consequences are the caller's problem. Unaudited; not for mainnet value.

## Licence

MIT — Stichting Kii Foundation
