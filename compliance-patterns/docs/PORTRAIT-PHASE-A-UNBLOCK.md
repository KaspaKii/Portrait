# Portrait Phase-A unblock — cross-layer ZK verification path

> **Pre-production, unaudited, testnet-only.** Dated 2026-06-28.

This document records, precisely and without overclaiming, **why the on-chain
STARK validity check for the Tier-3 cross-layer pattern cannot be emitted by
`silverc` today**, and **what the concrete unblock is**.

## The empirical `silverc` opcode surface

The SilverScript compiler (`silverc`, `~/.cargo/bin/silverc`) exposes these
`Op*` surface functions (probed empirically — each is accepted by `silverc`):

- `OpInputCovenantId`
- `OpCovInputIdx`
- `OpCovInputCount`
- `OpCovOutputCount`
- `OpCovOutputIdx`
- `OpAuthOutputCount`
- `OpAuthOutputIdx`
- `OpTxGas`
- `OpTxInputScriptSigLen`
- `OpTxInputScriptSigSubstr`

A representative covenant that compiles (exit 0):

```silverscript
pragma silverscript ^0.1.0;
contract Name(int max_ins, int max_outs, int start) {
    int balance = start;
    #[covenant(binding = cov, from = max_ins, to = 1, mode = transition)]
    function transfer(State[] prev_states, int amount, byte[32] proof_cov_id) : (State) {
        require(proof_cov_id == OpInputCovenantId(0));
        return({ balance: prev_states[0].balance - amount });
    }
}
```

## The blocker: `OpZkPrecompile` is NOT a `silverc` surface function

Probing `silverc` for `OpZkPrecompile` returns **`unknown function call`** —
**identical** to the response for a fabricated, non-existent builtin. There is no
way to distinguish a real-but-missing builtin from a fake one; `silverc` simply
does not know the symbol.

**Consequence:** the on-chain STARK validity check (KIP-16 tag `0x21`, RISC Zero
succinct STARK) **cannot be emitted by `silverc` today.** A Portrait program that
"wants" `OpZkPrecompile(0x21, journal)` has no SilverScript surface to lower it
onto.

## Where the verification actually lives: the engine-level tag-0x21 precompile

The STARK verification is an **engine-level KIP-16 tag-0x21 precompile**, invoked
from a **raw P2SH redeem script** — not from SilverScript. This library already
implements the redeem-script assembly in `crates/kcp-pq-anchor`:

- `crates/kcp-pq-anchor/src/anchor_script.rs` —
  `build_pq_anchor_redeem(&PqAnchorScriptFields) -> Result<Vec<u8>, PqAnchorError>`
  assembles the tag-0x21 redeem script. It pushes the canonical `hashfn`
  (Poseidon2 = `OP_1`), the tag byte `0x21`, and emits `OP_0` to invoke the
  precompile. (See the "canonical hashfn push" invariant in the crate README —
  consensus rejects non-canonical integer pushes.)
- `crates/kcp-pq-anchor/src/journal_spec.rs` — `JournalSpec` defines the journal
  layouts and `journal_hash()`. The KTT/CSCI layout is
  `covenant_id (32) || new_state_hash (32) || rule_hash (32) || seq (8 LE)`
  (104 bytes total), matching the tier3-demo RISC Zero guest journal.
- `crates/kcp-pq-anchor/src/sigop.rs` — `sigop_count_for_pq_verify()` returns the
  script-units budget (255) the spending transaction must declare; the precompile
  is not a legacy sig-op, so an under-budget spend is node-rejected.

**Honesty note on maturity.** The redeem-script *assembly* is implemented in this
crate. A **live TN10 on-chain spend of an engraved Portrait covenant through this
path is PENDING wallet funding** — there are no Portrait-path TN10 txids yet (see
`docs/CSCI-PROVENANCE.json`: status `PARTIAL — prover guest verified; TN10 txids
pending wallet funding`). The negative-control checks that exist today
(`examples/tier3-demo/`, `examples/csci-demo/`) are dev-mode / off-chain, not a
live on-chain tag-0x21 verification. A *separate* Kii project (`kii-ml-dsa`) has
validated the tag-0x21 PQ pipeline on TN10 for ML-DSA-44 / SLH-DSA / FN-DSA per
`KNOWN-ISSUES.md`; that is evidence for the engine mechanism, **not** for this
library's Portrait path. Do not conflate the two.

## The exact unblock

Phase-A settlement for a Tier-3 Portrait covenant is a **two-surface split**:

1. **Covenant-ID binding — emitted by Portrait/SilverScript.** Portrait emits
   `require(proof_cov_id == OpInputCovenantId(0))` in the transition function for
   any role with a VProg counterpart. `OpInputCovenantId` **is** a real `silverc`
   surface op, so this half compiles and runs on-chain as a covenant-id-bound
   check. It proves that the journal's `covenant_id[0..32]` equals the on-chain
   covenant ID.

2. **STARK validity + state binding — assembled by `kcp-pq-anchor`'s raw script.**
   The on-chain STARK verification and the state-commitment binding
   (`next_state_commitment == journal.new_state_hash`, i.e. journal bytes
   `[32..64]`) go through the engine-level tag-0x21 path via
   `build_pq_anchor_redeem`. The journal produced by the tier3-demo guest is fed
   as `PqAnchorScriptFields { journal, image_id, control_id, seal, … }`.

**Concrete integration point.** The Portrait Phase-A settlement builder calls
`kcp_pq_anchor::anchor_script::build_pq_anchor_redeem` with a `PqAnchorScriptFields`
whose `journal` is the 104-byte CSCI journal emitted by the tier3-demo RISC Zero
guest (`covenant_id || new_state_hash || rule_hash || seq`), and sets the
spending transaction's `sigOpCount` to `kcp_pq_anchor::sigop::sigop_count_for_pq_verify()`.
The same `covenant_id` value is what Portrait's emitted `OpInputCovenantId(0)`
check binds against — so the two surfaces agree on a single covenant ID.

This gives a sound Phase-A loop **across two languages**: SilverScript enforces
the covenant-id binding; the raw engine script enforces STARK validity and the
state-commitment advance.

## What remains for a single-language, emitted-in-one-place loop

To collapse the two-surface split into a single emitted-in-SilverScript loop, the
language needs a **SilverScript surface function for tag 0x21** (e.g. an
`OpZkPrecompile` / equivalent builtin that `silverc` can lower). That is an
**upstream silverscript engine/compiler feature** — credit to **Ori / the
silverscript team**. It is **not done**, and this library does **not** claim it.
Until it lands, the two-surface integration above is the sound path.

## See also

- `crates/kcp-pq-anchor/README.md` — tag-0x21 assembly, canonical hashfn invariant.
- `docs/TIER3-CROSS-LAYER-BINDING.md` — full cross-layer binding protocol.
- `docs/CSCI-PROVENANCE.json` — TN10 evidence status (pending).
- `KNOWN-ISSUES.md` → "Portrait compiler / Tier 3".
