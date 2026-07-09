# Tier 3 — The Flagship: One Source, Two Layers, Bound by a Covenant ID

**Status:** Live-settled cross-layer on Kaspa testnet-10 (TN10) · Pre-production,
unaudited, **testnet-only** · perishable evidence (testnet UTXOs can be pruned)
**Engine pin:** `rusty-kaspa` tag `v2.0.0` (commit `90dbf07`)
**Flagship source:** `portrait/library/state/CsciInstrument.portrait`
**Authoritative txid source:** [`examples/portrait-settlement/PROVENANCE.json`](../examples/portrait-settlement/PROVENANCE.json) (read-only — never edit)

> Every txid and covenant_id in this document traces to `PROVENANCE.json`. No
> txid is invented. Sections are labelled **LIVE** (a real, REST-verified TN10
> transaction) or **COMPILER-VERIFIED** (built/compiled locally, not on-chain).
> This document performs **no new settlement** — the live txids below are
> cited from `PROVENANCE.json`.

---

## 1. Thesis

> **Describe an app once; project it across BOTH layers; settle it on L1; bind
> the two with a KIP-20 covenant ID.**

A single Portrait source declares one role. Portrait's Pounce pass classifies its
entrypoints: the one carrying `#[covenant(mode = transition)]` becomes an **L1
covenant** (compiled to SilverScript, `.sil`); the one without becomes an
**off-L1 vProg** (compiled by Atelier to a RISC Zero guest). The L1 covenant's
spend is bound to the vProg's STARK by a **per-instance covenant ID**: the
emitted `.sil` requires `proof_cov_id == OpInputCovenantId(0)`, and the STARK
journal commits that same 32-byte value. Same identity on both sides.

The flagship that demonstrates this end-to-end is
**`library/state/CsciInstrument.portrait`** — a Covenant-Settled Compliance
Instrument (CSCI). The live combined transaction below literally spends a
CsciInstrument covenant input.

---

## 2. The pipeline

```
  ╔════════════════════════════════════════════════════════════════════╗
  ║  CsciInstrument.portrait   (ONE source — role `instrument`)         ║
  ║  portrait/library/state/CsciInstrument.portrait      ║
  ╚════════════════════════════════════════════════════════════════════╝
                              │
              ┌───────────────┴────────────────┐
   settle  #[covenant(transition)]      csci_rules  (no attribute)
        → Pounce: Covenant                 → Pounce: VProg
              │                                  │
       portrait engrave                   portrait atelier-build
              │                                  │
              ▼                                  ▼
      CsciInstrument.sil                 csciinstrument_guest_main.rs
      (silverc ok)                       (RISC Zero guest main)
              │                                  │
   emits the KIP-20 binding:            commits the 104-byte journal:
   require(proof_cov_id ==              covenant_id[32] || new_state_hash[32]
     OpInputCovenantId(0))                || rule_hash[32] || seq[8 LE]
              │                                  │
              └───────────────┬─────────────────┘
                              ▼
              KIP-20 cross-layer binding (same per-instance covenant_id)
                              │
                              ▼
       Settlement harness composes a 2-input TN10 transaction:
         input[0] = CsciInstrument SilverScript covenant (seq/auth/cov-id)
         input[1] = tag-0x21 P2SH verifier (OpZkPrecompile over the STARK)
                              │
                              ▼
              LIVE on TN10 — combined settle abc2d13f… (is_accepted=true)
```

---

## 3. One source → two artifacts  *(COMPILER-VERIFIED — built locally, not on-chain)*

Rebuilding both artifacts is idempotent and leaves the git tree clean:

```
$ portrait engrave library/state/CsciInstrument.portrait
[pounce] settle → Covenant
[pounce] csci_rules → VProg
[allocate] instrument.csci_rules (VProg): vProg entrypoint is fully covenant-legal …
[emit]   CsciInstrument.sil
[silverc] ok CsciInstrument.sil

$ portrait atelier-build library/state/CsciInstrument.portrait
[pounce]  settle → Covenant
[pounce]  csci_rules → VProg
[atelier] KovId auto-derived from silverc output
[atelier] guest main written → library/state/csciinstrument_guest_main.rs
```

The emitted covenant `library/state/CsciInstrument.sil` carries the KIP-20
binding automatically — `settle` gains the `proof_cov_id` parameter and the
`require` because the role has a vProg companion (`has_vprog`):

```silverscript
#[covenant(binding = cov, from = max_ins, to = 1, mode = transition)]
function settle(State[] prev_states, sig auth, byte[32] next_state_hash, byte[32] proof_cov_id) : (State) {
    require(proof_cov_id == OpInputCovenantId(0));            // ← KIP-20 cross-layer binding
    require(checkSig(auth, prev_states[0].owner));           // committed-owner authorization
    return({ owner: prev_states[0].owner, amount: prev_states[0].amount, seq: prev_states[0].seq + 1, state_hash: next_state_hash });
}
```

The vProg companion (`csci_rules`) compiles to a RISC Zero guest
(`csciinstrument_guest_main.rs`) that commits the 104-byte journal
`covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]`.

> **Scope of this section:** this is a *build* fact (silverc exit 0,
> atelier-build exit 0). The `require(proof_cov_id == OpInputCovenantId(0))` is a
> static emission fact, read directly from the `.sil`. Its *on-chain
> satisfaction* is what the LIVE evidence in §4 demonstrates.

---

## 4. The live cross-layer settlement  *(LIVE on TN10 — REST-verified)*

This is the headline. Source: `PROVENANCE.json → combined_live_and_single_redeem.item1_combined_live`.

### 4a. Both layers in ONE transaction

One TN10 transaction whose acceptance requires **both** inputs to pass:

| Field | Value |
|---|---|
| **Combined settle txid** | `abc2d13f10e4a8ad1de2f0bdce804853820efd49bd9d06b9bd28af647d5bf728` |
| `is_accepted` | `true` |
| `accepting_block_hash` | `17214c587f506e700754e81bb91229b92794d06ebb5064cea4b595dbce2b796c` |
| `n_inputs` | 2 |
| `mass` | 477206 |
| input[0] | spends `c6912cde…:0` — CsciInstrument **SilverScript covenant** UTXO (seq / auth / cov-id binding) |
| input[1] | spends `fab70e35…:0` — **tag-0x21 P2SH verifier** UTXO (OpZkPrecompile over the real STARK) |
| per-instance covenant_id | `039568671d98396a355220003474574e903b676cae947d8d8b002637b1bb12e8` |
| verified_via | `submit_transaction` (live) + api-tn10.kaspa.org (`is_accepted=true`, `n_inputs=2`) |

A transaction is valid only if every input's script passes, so this is genuine
**atomic co-enforcement of both layers in one tx**. The covenant input is a
CsciInstrument SilverScript program — the exact `.sil` produced by §3.

### 4b. The cross-binding settle (same covenant_id on both sides)

A 1-input SilverScript settle proving the journal commits the *same* per-instance
covenant_id the `require()` checks. Source: `PROVENANCE.json → cross_bound`.

| Field | Value |
|---|---|
| **Cross-bound settle txid** | `60738affe00221d06af26718084c6f66e5287b2527ab319c0e8e2d68169e01c3` |
| spends | `de20bb76…:0` (the locked covenant UTXO) |
| `is_accepted` | `true` |
| per-instance covenant_id (both sides) | `e1e562311cd5eed38395052b77c55237ad5d1f42b858b3297084018641c70d11` |

`OpInputCovenantId(0)` returns `e1e562…`; the STARK journal `journal[0..32]` also
commits `e1e562…`. Same 32-byte id on both sides → genuinely cross-bound.

### 4c. Negative controls (the safety proof)  *(LIVE rejections)*

| Control | Attempted txid | Node verdict | On-chain? |
|---|---|---|---|
| Tampered journal in the combined tx (`journal[0]^=1`) | `1f49c3dc466ff81e3ffb90612a54a2d45a6af772cbfe9151ac67186c9dc84a5c` | `ZK Integrity: Verification failed` | No — REST 404 |
| Cross-binding: covid_A proof against instance B | `60e9effc817bb449e78df7b8cc183ee7dc23b501dde970a81d1fe209ccfc6c83` | `script ran, but verification failed` | No — REST 404 |
| SilverScript seq not incremented (live, single-layer) | `f11c8875e5b3e1fbf02f01a5f9269d9b98b5c53273484f26ec4e08a75f8de42b` | `script ran, but verification failed` | No — REST 404 |

A rejected transaction has no on-chain txid; the evidence is the live node's
reject error plus the REST 404 (the tx never entered the chain).

---

## 5. The vProg pattern catalogue  *(all 5 LIVE on TN10)*

Each pattern starts as an Atelier emit-verified template, graduates to a real
RISC Zero predicate guest (compile-verified), then settles live: the spend is
accepted **only because the STARK verified in-consensus** (tag-0x21
OpZkPrecompile) over the pattern's journal. Source: the `vprog_*_settled_live`
blocks in `PROVENANCE.json`.

| Pattern | Lock txid | Settle txid | Status |
|---|---|---|---|
| ProofOfReserves (solvency) | `64fd5e81…` | `3d7c4a1603d0268bf5aaab486a9be3d0f9bc682c0c99eed8af64aa50b09b6df5` | **LIVE** |
| ComplianceCredential (ZK-KYC) | `c865ccb6…` | `52616496ffb83d97d29857a0b373478d0d60563ecc155389caee4ea7d4fb5591` | **LIVE** |
| ConfidentialTransfer (hidden amounts) | `0ed82a05…` | `2d8166f947ddfbcacb19c0121c4a2dca93bb97befb7fe2854db3e642c51d8d2f` | **LIVE** |
| BatchRollup (N→1 fold, N=5) | `2ce1dbd1…` | `0237d2de2638a182645951a884af02d87f7e8683d578cc1a790259b8b9c284bd` | **LIVE** |
| PrivateVoting (ballot privacy) | `cda62be3…` | `112780ec67b1aaa160567e4c18952c97586b75462b2148abc242b42d86f832a6` | **LIVE** |

Each live pattern also has its own negative controls (guest panics on a false
predicate, node ZK-reject on a tampered journal) recorded in `PROVENANCE.json`.

> **All five vProg patterns are settled live on TN10** (each `is_accepted=true`,
> REST-confirmed via `api-tn10.kaspa.org`). Honest residuals (all 5): the live
> covenant is the `tag-0x21` verifier P2SH (image-id-pinned), not yet a
> SilverScript state machine; inputs are fixed sample data over small fixed sets
> (no Merkle-rooted registry, no persistent nullifier set for PrivateVoting);
> commitments are `sha256(value‖blinding)`, not Pedersen; the audit key is a v1
> symmetric pad. Pre-production, unaudited, testnet-only; evidence is perishable.

---

## 6. Stepping stones (the on-ramp)

Two simpler sources in `examples/tier3-demo/` teach the basics before the
flagship. They prove dual emission but do **not** emit the KIP-20 binding:

- **`ComplianceToken.portrait`** — dual emission basics: one source → a covenant
  `.sil` + a vProg guest. Use this to understand Pounce classification.
- **`HeavyAirdrop.portrait`** — layer-aware allocation: its vProg body has a
  `for` loop, a heavy construct rejected in a covenant but accepted in a vProg.
  Use this to understand why some logic belongs off-L1.

The destination is `CsciInstrument.portrait`, the only one of the three whose
spend binds a per-instance covenant_id and which settled live cross-layer.

---

## 7. Honest caveats

- **tag-0x21 verification is a separate verifier input.** The SilverScript
  `require(proof_cov_id == OpInputCovenantId(0))` checks that a witness arg
  equals the runtime covenant_id — it does **not** itself verify a STARK exists.
  The proof's *necessity* comes from the tag-0x21 verifier input (input[1] in the
  combined tx). The 2-input combined design is the way both layers are
  enforced in one tx; `abc2d13f…` proves it live.
- **Single self-cross-binding redeem remains open.** One raw P2SH redeem doing
  *both* the tag-0x21 verify and the SilverScript bindings was engine-rejected in
  both orderings on the pinned engine — `silverc`/portrait cannot emit
  `OpZkPrecompile` (`portrait-emit` carries a `[PENDING …]` note). There is no
  txid; this is the honest residual. Closing it needs a SilverScript-surface
  `OpZkPrecompile`, intentionally off the table (no upstream path).
- **Per-pattern residuals.** The live vProg patterns use fixed sample inputs and
  v1 primitives (e.g. ConfidentialTransfer's audit key is a symmetric-pad
  demonstrator, not Pedersen/ElGamal); their covenants are tag-0x21 verifier
  P2SH locks, not yet full SilverScript state machines (a planned next step).
  See each `honest_scope` in `PROVENANCE.json`.
- **Testnet evidence is perishable.** TN10 UTXOs can be pruned; txids are
  re-verifiable against the chain only while the data persists.

---

## 8. Reproduction

```sh
# One source → two artifacts (idempotent; leaves the tree clean)
cd portrait/portrait
cargo run -p portrait-cli --bin portrait -- engrave       ../library/state/CsciInstrument.portrait
cargo run -p portrait-cli --bin portrait -- atelier-build ../library/state/CsciInstrument.portrait
```

The live settlements are **not** reproduced here — they were performed once by
the settlement harness and are recorded in `PROVENANCE.json`. Re-running the
harness would spend testnet funds and is out of scope for this doc.

---

## 9. References

- [CsciInstrument.portrait](../../kii-portrait/portrait/library/state/CsciInstrument.portrait) — the flagship source
- [CsciInstrument.sil](../../kii-portrait/portrait/library/state/CsciInstrument.sil) — emitted covenant (carries the KIP-20 binding)
- [csciinstrument_guest_main.rs](../../kii-portrait/portrait/library/state/csciinstrument_guest_main.rs) — emitted vProg guest
- [PROVENANCE.json](../examples/portrait-settlement/PROVENANCE.json) — **authoritative txid source** (read-only)
- [PORTRAIT-PROGRAM-WRITEUP.md](PORTRAIT-PROGRAM-WRITEUP.md) — combined-tx write-up
- [PORTRAIT-VPROG-PATTERNS.md](PORTRAIT-VPROG-PATTERNS.md) — the vProg pattern catalogue
- [FLAGSHIP-DESIGN.md](FLAGSHIP-DESIGN.md) — CSCI architecture
- `examples/tier3-demo/ComplianceToken.portrait`, `HeavyAirdrop.portrait` — stepping stones
</content>
</invoke>
