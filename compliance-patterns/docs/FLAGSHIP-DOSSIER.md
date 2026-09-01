# FLAGSHIP-DOSSIER — CsciInstrument (Covenant-Settled Compliance Instrument)

**Status:** Pre-production, unaudited, testnet-only. Stichting Kii Foundation. MIT licence.
**Engine pin:** rusty-kaspa v2.0.0 (`90dbf07`) — do not bump.
**Captured:** 2026-06-29. Stichting Kii Foundation.

> **Maturity stamp (mandatory, non-negotiable).** Nothing here is audited or
> reviewed by a security professional. All evidence is testnet-10 (TN10) only,
> synthetic data, no real-world value. **Testnet evidence is perishable** — TN10
> resets by design; every txid below is evidence at a point in time and must be
> re-verified against `examples/portrait-settlement/PROVENANCE.json` (the
> authoritative, read-only evidence file) before any public claim. **Do not
> trust the stamps in this document — reproduce them.**

This dossier stitches a single artifact end-to-end:
**one Portrait source → a silverscript covenant `.sil` (with a KIP-20 binding) +
a vProg guest → a threat model → reject vectors (compile-time + on-chain) →
golden coverage → a rederivable Hallmark → REST-verified live TN10 txids.**
It supersedes nothing in `PROVENANCE.json`; it indexes and reconciles it.

---

## 1. Thesis

`CsciInstrument` is the flagship: **one Portrait source compiles to a
silverscript covenant whose `settle` self-enforces the CSCI state machine on
Kaspa L1, and to a RISC Zero vProg guest that proves the compliance predicate
off-L1.** The covenant binds to the ZK-settled journal via the KIP-20
covenant-id mechanism. It is **settled LIVE on TN10**, cross-layer, with on-chain
negative controls for three distinct rejection classes.

- **Source:** `library/state/CsciInstrument.portrait` (portrait repo)
- **Emitted covenant:** `library/state/CsciInstrument.sil`
- **Emitted guest companion:** `library/state/csciinstrument_guest_main.rs`
- **Hallmark:** `library/state/CsciInstrument.hallmark.json` (4 claims, all PASS)
- **Live evidence:** `examples/portrait-settlement/PROVENANCE.json` (this repo)

---

## 2. Source → .sil (the KIP-20 binding)

The source declares one role with two entrypoints. `settle` carries
`#[covenant(mode = transition)]` → the Engraver compiles it to the on-chain
`.sil`. `csci_rules` carries **no** `#[covenant]` attribute → it lowers as a
NonCovenant (vProg) body (Atelier emits its RISC Zero guest main). The mere
presence of the vProg companion flips `has_vprog`, **which is what causes the
covenant-id binding `require()` to be emitted into `settle`.**

Source `settle` (portrait):

```
#[covenant(mode = transition)]
entrypoint function settle(sig auth, bytes32 next_state_hash)
    : (pubkey owner, int amount, int seq, bytes32 state_hash) {
  requires checkSig(auth, owner);          // committed-owner authorization
  return CsciInstrument {
    owner:      owner,                      // owner key carried unchanged
    amount:     amount,                     // value conserved (carry f:f)
    seq:        seq + 1,                    // CSCI sequence advances by one
    state_hash: next_state_hash            // adopt the new committed state hash
  };
}
```

Emitted `CsciInstrument.sil` (the compiled covenant):

```
contract CsciInstrument(int max_ins, int max_outs, pubkey owner, int amount, int seq, byte[32] state_hash) {
    ...
    #[covenant(binding = cov, from = max_ins, to = 1, mode = transition)]
    function settle(State[] prev_states, sig auth, byte[32] next_state_hash, byte[32] proof_cov_id) : (State) {
        require(proof_cov_id == OpInputCovenantId(0));               // KIP-20 binding
        require(checkSig(auth, prev_states[0].owner));               // committed-owner auth
        return({ owner: prev_states[0].owner, amount: prev_states[0].amount, seq: prev_states[0].seq + 1, state_hash: next_state_hash });
    }
}
```

The three load-bearing on-chain rules, each pinned by a golden test (§5):

1. **KIP-20 covenant-id binding** — `require(proof_cov_id == OpInputCovenantId(0));`
   ties the silverscript layer to the settled UTXO's per-instance covenant id.
2. **Committed-owner authorization** — `require(checkSig(auth, prev_states[0].owner));`
   (never a caller-supplied pubkey).
3. **Sequence monotonicity** — `seq: prev_states[0].seq + 1` (no replay / no skip).

---

## 3. Threat Model

The full written threat model — consensus-trust vs zkVM-trust vs operator-trust
boundary diagram, plus the malicious-guest / input-forgery / rule-staleness /
standalone-vs-synchronous / testnet-perishability failure cases — is in
[`docs/FLAGSHIP-DESIGN.md` §6](./FLAGSHIP-DESIGN.md). The summary below is a
condensed version; the reconciliation subsection is the honest bridge from the
design ideal to what is actually built.

### 3.1 What each layer enforces

**Covenant (silverscript, consensus-final).** The compiled `settle` enforces, at
consensus:
- **State continuity / sequence monotonicity** — `seq` advances by exactly one;
  replay is rejected because the predecessor UTXO is already spent.
- **Committed-owner authorization** — the spend is authorized against the owner
  key carried in the *prior* committed state, not a caller argument.
- **Value conservation** — `amount` is carried from its own prior value.
- **Covenant-id binding** — `proof_cov_id == OpInputCovenantId(0)`: the
  spender-supplied `proof_cov_id` must equal the engine per-instance covenant id
  the spend runs under (the silverscript layer's binding to its own UTXO).

**vProg / tag-0x21 (ZK-settled, not consensus-native).** A RISC Zero succinct
STARK, verified in-consensus via KIP-16 tag-0x21 (`OpZkPrecompile`) running as a
**separate P2SH(redeem) input**, enforces:
- **The compliance predicate** — evaluated inside the zkVM; the chain trusts only
  that the proof is valid for the declared `image_id` (`c6ce0eda…`), pinned at
  genesis. A bug in the guest produces a wrong-but-validly-proved result the
  covenant cannot detect.
- **Journal integrity** — `journal[0..32]` commits the per-instance covenant id;
  tampering the journal breaks the STARK and the spend is rejected.

### 3.2 Trust boundary (summary)

| Layer | Trusted to enforce | Override-able by |
|---|---|---|
| **Consensus** | proof valid for `image_id`; journal integrity; seq+1; committed-owner auth; covenant-id binding | nobody (every Kaspa node) |
| **zkVM** | predicate logic correct; guest commits the right `new_state_hash` | a malicious/buggy guest (mitigate: `image_id` committed + guest source published, independently reproducible) |
| **Operator** | input data fed to the prover is authentic; `rule_hash` was correct at genesis | the operator (residual, named honestly) |

### 3.3 Reconciliation — design ideal vs as-built (honest)

`FLAGSHIP-DESIGN.md` §2–§3 describes a **single-redeem ideal**: one script doing
both the ZK verify (`OpZkPrecompile`) and the covenant checks (`journal[64..96]
== rule_hash`, output-state validation, seq+1) in a single spend path. **That
ideal is NOT achievable on the pinned engine, and `PROVENANCE.json` records this
honestly.** The as-built reality:

- **`silverc` cannot emit `OpZkPrecompile`.** `portrait-emit` literally carries
  `[PENDING: add OpZkPrecompile(0x21, journal) call when engine support lands]`.
  The compiled `CsciInstrument.script` uses `OpInputCovenantId(0xcf)` +
  `OpCheckSig(0xac)` + `OpBlake2b(0xaa)` but **not** `OpZkPrecompile(0xa6)`.
- **The tag-0x21 ZK verification runs as a SEPARATE input** — a `P2SH(redeem)`
  verifier UTXO, distinct from the silverscript covenant UTXO.
- **Both layers are enforced atomically in ONE transaction** via a **2-input
  combined** design: input[0] = the silverscript covenant (seq/auth/cov-id),
  input[1] = the tag-0x21 verifier (`OpZkPrecompile` over the STARK). A tx is
  valid only if **every** input's script passes → genuine atomic co-enforcement.
  This is **live** on TN10 (`abc2d13f…`, `n_inputs=2`, `is_accepted=true`).
- **Cross-binding is closed** by committing the engine **per-instance**
  covenant id into the STARK journal (a two-phase lock→read-covid→prove→spend
  bootstrap), so the silverscript `require` and the proof reference the **same**
  32-byte id (`60738aff…`).
- **The single-redeem branch REMAINS blocked** and is reported with the exact
  limitation (silverscript's terminal stack discipline + the absent surface
  opcode); no single-redeem tx was broadcast (honestly, no txid).

### 3.4 Honest residuals (preserved verbatim in spirit from `PROVENANCE.json`)

- The tag-0x21 ZK *verification* runs as a **separate P2SH input**, not a single
  silverscript redeem; single-redeem both-layers is blocked on the pinned engine.
- The genesis prev-state (amount, `seq=0`) is a **chosen starting point**, not
  itself anchored on-chain by a prior settle (this is the first transition).
- A spender who knows the runtime covid could satisfy the silverscript
  `proof_cov_id` check *without* a valid proof; the **proof's necessity comes
  from the verifier input** being present (the 2-input combined tx). Full
  single-script binding needs `silverc` `OpZkPrecompile` support, which is
  intentionally **off the table** (no upstream/kaspanet path).
- **Testnet evidence is perishable.** All txids are point-in-time.

---

## 4. Reject Vectors

Two tiers, bridged: **compile-time sema rejects** (the §4.4 structural checks)
and **on-chain live negative controls** (consensus rejections on TN10). Each
on-chain reject txid is **`null` by design** — a rejected transaction has no
on-chain txid; the evidence is the node's error string + a REST **HTTP 404**
(never entered the chain).

### 4.1 Compile-time (sema) reject vectors

These are exercised both generically (portrait-sema unit tests) and **directly
against the flagship source** (new golden tests, §5):

| Property | Flagship golden (this dossier) | Generic sema unit test | File:fn |
|---|---|---|---|
| Sequence monotonicity | `reject_csci_non_monotonic_seq` | `c3_rejects_non_monotonic_seq` | `portrait-sema/src/lib.rs::c3_rejects_non_monotonic_seq` |
| Committed-owner auth (no-checkSig under value_conserved) | `reject_csci_drops_owner_auth` | `c2_rejects_unauthorized_mutation_under_value_conserved` | `portrait-sema/src/lib.rs::c2_rejects_unauthorized_mutation_under_value_conserved` |
| Value conservation (dropping transition) | (covered generically) | `rejects_value_conserved_with_dropping_transition` | `portrait-sema/src/lib.rs::rejects_value_conserved_with_dropping_transition` |
| Caller-supplied pubkey auth | (covered generically) | `c2_rejects_caller_supplied_pubkey` | `portrait-sema/src/lib.rs::c2_rejects_caller_supplied_pubkey` |

The flagship goldens are in
`portrait/crates/portrait-cli/tests/golden.rs`:
`reject_csci_non_monotonic_seq` (names `monotonic_seq`) and
`reject_csci_drops_owner_auth` (names the no-auth diagnostic). Each is a single
surgical mutation of the real `CsciInstrument.portrait` source.

### 4.2 On-chain (live TN10) negative controls — three distinct rejection classes

| Rejection class | Compile-time mirror | Live reject txid (attempted) | Node error | REST |
|---|---|---|---|---|
| **Seq-violation** (silverscript continuity require fails) | `reject_csci_non_monotonic_seq` / `c3_rejects_non_monotonic_seq` | `f11c8875e5b3e1fbf02f01a5f9269d9b98b5c53273484f26ec4e08a75f8de42b` | `script ran, but verification failed` | 404 |
| **Cross-binding** (proof for instance A spent against instance B) | (architectural; covenant-id binding) | `60e9effc817bb449e78df7b8cc183ee7dc23b501dde970a81d1fe209ccfc6c83` | `script ran, but verification failed` | 404 |
| **ZK-integrity** (tampered journal, combined 2-input tx) | (ZK layer; out of sema scope) | `1f49c3dc466ff81e3ffb90612a54a2d45a6af772cbfe9151ac67186c9dc84a5c` | `ZK Integrity: Verification failed` | 404 |
| **ZK-integrity** (tampered journal, single-layer base settle) | (ZK layer; out of sema scope) | `e44d4f7bf5ac33d498dda6e6750460336beb380618c6c35fdd7221e0ca4d7d32` | `ZK Integrity: Verification failed` | 404 |

The **distinct error strings** are the point: the silverscript seq/cross-binding
rejects say `script ran, but verification failed` (the covenant `require`
failing), while the ZK-layer rejects say `ZK Integrity: Verification failed`
(the tag-0x21 verifier failing). Two independent enforcement surfaces, each
demonstrated to reject.

---

## 5. Golden Coverage

**Before this work**, `CsciInstrument`'s shape was exercised only *indirectly* by
the portrait-sema `c2_*`/`c3_*` unit tests; it was absent from the sema
round-trip list and had no golden of its own (unlike Counter / ComplianceToken /
KycGatedTransfer / etc.). **This work closed that gap.**

Added to `portrait/crates/portrait-cli/tests/golden.rs` (golden now **37
passed**, was 33):

- **`accept_csci_instrument_flagship`** — parse + sema + full pipeline → exactly
  ONE covenant (`CsciInstrument`/`settle`), emitted `.sil` compiles under the
  real `silverc` (exit 0), and asserts the three load-bearing anchors:
  `require(proof_cov_id == OpInputCovenantId(0));`,
  `require(checkSig(auth, prev_states[0].owner));`, and `seq: prev_states[0].seq + 1`.
- **`csci_kip20_binding_is_present`** — pins that the `csci_rules` vProg companion
  exists (the thing that flips `has_vprog`) AND that the KIP-20 binding appears in
  the emitted `.sil` — guards against the binding silently disappearing.
- **`reject_csci_non_monotonic_seq`** — surgical seq-mutation rejected, naming
  `monotonic_seq`.
- **`reject_csci_drops_owner_auth`** — surgical checkSig-drop rejected (no-auth
  under `value_conserved`).

Added to `portrait/crates/portrait-sema/src/lib.rs`: `CsciInstrument` is now in
`accepts_all_shipped_round_trip_sources` (sema lib now **83 passed**, was 82), so
the flagship is in the shipped round-trip set alongside its siblings.

**Gates green** (changed crates): `cargo fmt --check` clean, `cargo clippy
--tests` clean (no warnings), `cargo test -p portrait-cli --test golden` = 37
passed, `cargo test -p portrait-sema --lib` = 83 passed. `silverc` resolved at
`$HOME/.cargo/bin/silverc` (the differential compile actually ran, not skipped).

---

## 6. Hallmark (rederivable — reproduce it, don't trust it)

`library/state/CsciInstrument.hallmark.json` — regenerated **from the repo
root** so the `source`/`rederive` fields carry clean repo-root-relative
paths (previously `../library/state/…`). All **4 claims PASS**:

| Claim | Check | Result |
|---|---|---|
| Source parses as well-formed Portrait | `parse` | pass |
| Satisfies portrait-sema structural checks | `sema` | pass |
| Lowers/projects to ≥1 silverscript covenant | `emit` (1 covenant: CsciInstrument) | pass |
| Pinned `silverc` accepts the emitted `.sil` (exit 0) | `silverc-accepts[CsciInstrument]` | pass |

Rederive command (from the portrait workspace root):

```
cargo run -p portrait-cli -- verify library/state/CsciInstrument.portrait
# (or, if installed: portrait verify library/state/CsciInstrument.portrait)
```

This re-runs `portrait_syntax::parse`, `portrait_sema::check`, the
lower→project→emit pipeline, and invokes `silverc --constructor-args <ctor> -c
<emitted.sil>`, re-deriving every claim. **Do not trust the stamp — reproduce
it.** The flagship now carries the same Hallmark stamp its siblings
(Escrow / StreamingVesting / MultisigTreasury / DigitalReit) already have.

---

## 7. Live Evidence (REST-verified TN10 txids)

All from `examples/portrait-settlement/PROVENANCE.json`. Engine
rusty-kaspa v2.0.0 (`90dbf07`), TN10, each confirmed `is_accepted=true` via
`api-tn10.kaspa.org`. **Cite only these REST-verifiable txids.**

| What | txid | `is_accepted` | Notes |
|---|---|---|---|
| **Combined cross-layer settle (2-input)** | `abc2d13f10e4a8ad1de2f0bdce804853820efd49bd9d06b9bd28af647d5bf728` | true | `n_inputs=2`: silverscript covenant + tag-0x21 verifier in ONE tx; accepting block `17214c58…` |
| **Cross-bound silverscript settle** | `60738affe00221d06af26718084c6f66e5287b2527ab319c0e8e2d68169e01c3` | true | journal commits per-instance covid `e1e562…`; same 32-byte id on both sides |
| **CSCI silverscript layer settle** | `5731a2039a95780a3c0ea53b7dda4d404074b22d8d3014fdd247935f50acdfea` | true | seq 0→1, owner-signed; genesis `27de5b3c…` |
| **Base real-instrument settle** | `11b4d7d91feb26e71f57a8e754df9abdd3baa3788064e9163bbe2511ad960bfe` | true | covenant_id `869bfb4d…` = blake2b256(redeem); image_id `c6ce0eda…` |
| **Base real-instrument lock** | `1f14b628599431cd50f5d1f26d0f388eae0ffe7beba9ea505ac778a6b82f33db` | true | locks the settled UTXO |

Negative-control txids are **`null` by design** (rejected txs have no txid):
see §4.2 (`f11c8875…`, `60e9effc…`, `1f49c3dc…`, `e44d4f7b…`, each REST 404).

---

## 8. Residuals & Honest Scope (consolidated)

From `PROVENANCE.json` `honest_caveats` + the `cross_bound` / `combined_live`
residuals:

1. **Two-input, not single-redeem.** The covenant self-enforces its state machine
   on-chain; the tag-0x21 ZK *verification* runs as a separate P2SH input. Both
   are co-enforced atomically in one live tx (`abc2d13f…`) and cross-bound to the
   same per-instance covid. A single silverscript redeem doing both **remains
   blocked** on the pinned engine (`silverc` cannot emit `OpZkPrecompile`); this
   is reported truthfully with no txid.
2. **Covenant-id binding is the silverscript layer's own UTXO binding.** The
   silverscript checks `proof_cov_id == OpInputCovenantId(0)`; it does not itself
   verify a STARK exists for that journal — that is the verifier input's job. The
   proof's necessity comes from the verifier input being present.
3. **Genesis prev-state is a chosen starting point**, not anchored by a prior
   on-chain settle.
4. **Image-id reproducibility, not audit.** `image_id c6ce0eda…` is
   content-addressed and rebuild-reproducible (rzup 0.5.0, cargo-risczero 3.0.5,
   risc0 toolchain v1.91.1); the guest source is published so anyone can
   reproduce it. This is *reproducibility*, not a security audit.
5. **Pre-production, unaudited, testnet-only, perishable.** Re-verify every txid
   against `PROVENANCE.json` (and the live REST API while TN10 still carries the
   data) before any public claim.

---

*Cross-references: design + ideal in `docs/FLAGSHIP-DESIGN.md`; authoritative
live evidence in `examples/portrait-settlement/PROVENANCE.json` (read-only);
source/sil/guest/hallmark in the portrait repo `library/state/`; golden + sema
coverage in `portrait/crates/portrait-cli/tests/golden.rs` and
`portrait/crates/portrait-sema/src/lib.rs`.*
