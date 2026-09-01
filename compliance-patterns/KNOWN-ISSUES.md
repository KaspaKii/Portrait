# Known issues and documented next steps

Honest accounting of what is incomplete. None of these is hidden; each is a
deliberate scope boundary or a flagged limitation.

## Scope boundaries (next steps, not defects)

- **Vault on-chain enforcement is complete** for all condition shapes —
  multisig, timelock, and composite `Any(2)`/`All` `[KCP-VT-002, KCP-VT-003]`.
- **State-continuity enforcement is now engine-proven for all three patterns.**
  `ktt-token`, `sealed-lineage`, and `transferable-record` each ship a
  SilverScript covenant (`covenant/*.sil`, compiled to embedded script bytes)
  whose state-transition rules are proven against the real engine
  `[KCP-KTT-002, KCP-SL-002, KCP-TR-002]` — mirroring how upstream verifies
  KCC20 `[SS-026]`. These covenants are **covenant-id-bound** (the compiled
  script is the `scriptPubKey`), architecturally distinct from the P2SH-wrapped
  signature-and-condition covenants. Each of the three crates now carries an
  **in-repo, gate-run** engine proof (`tests/covenant_engine.rs`) that loads the
  committed `covenant/*.compiled.json`, splices per-state scripts into it, and
  runs them through the pinned v2.0.0 script VM. Every rejection is two-sided
  (the violating transition is refused *and* the same transition with the
  offending field restored is accepted), so the failure is pinned to the named
  invariant. The library still stays on `tag=v2.0.0` and embeds only the compiled
  scripts; no `silverscript-lang` dependency. Four residual gaps, each narrower
  than the old "proof lives outside the repo" one:
  - *The committed→deployed link is still out-of-repo.* Nothing here records the
    deployed script, its scriptPubKey, or the `-003` genesis `covenant_id` (which
    derives from the funding outpoint, not the script, so it cannot be recomputed
    from the artifact). The correspondence is attested by an archived byte
    capture. The one in-repo corroboration is an execution-cost fingerprint —
    each committed artifact consumes exactly the script-unit count recorded for
    its live preflight. Evidence, not a binding; see `docs/EVIDENCE.md`.
  - *Script-VM tier only.* The tests run input 0's script; no transaction mass,
    no KIP-9 storage mass, no standardness. The covenants place no floor on the
    output value, so callers must respect `MIN_CHANGE_SOMPI` themselves.
  - *KTT's 2-covenant-**input** (merge) shape has no in-repo coverage.* The
    committed artifact permits `maxCovIns = maxCovOuts = 2`; the tests cover 1→1
    and 1→2 (including the conserving/inflating split that is the only shape
    where KTT-1's sum is non-degenerate), but not the sibling-signature parse
    surface a 2-input merge exercises.
  - *The wider archived harnesses stay archived* (`CAVEATS/05..07`). Their
    sealed-lineage and transferable-record cases have all been **ported in-repo**
    — they were only different *values* in the same spliced state region, not
    something the SilverScript clone was needed for. What remains out-of-repo is
    whatever the archived harnesses covered beyond those; that inventory is
    `[FACT-NEEDED]`.
  - The **live** half of the claim is not reproducible at all: submitting a
    transaction cannot be a test.
- **The `covenant/*_ctor.json` files are not the constructor inputs that produced
  the committed artifacts.** `ktt_ctor.json` says `genesisAmount=0, maxCovIns=1,
  maxCovOuts=1` where the artifact bytes say `1000`, `2`, `2`;
  `sealed-lineage_ctor.json` says `genesisTBucket=0` where the artifact encodes
  `1700000000`. They post-date the `*.compiled.json` files they sit beside. The
  covenant READMEs now derive provenance from the artifact bytes (asserted in
  `tests/covenant_engine.rs`) and flag the ctor files as unreliable; the real
  constructor inputs are `[FACT-NEEDED]` until the artifacts are regenerated.
  - **Now demonstrated LIVE on testnet-10 `[KCP-RE-003]`.** The covenant-id-bound
    deployment is no longer a gap: the anchor-only reserve covenant (same
    covenant-id-bound shape) was deployed in a live genesis+append on testnet-10
    (genesis `fcecef64…`, append `980ca03a…`), consensus-enforced
    (`validateOutputState` + `checkSig`). Toccata covenants are active
    `[KCP-NET-002]`; the engine proof transfers across versions `[KCP-COV-SKEW-001]`;
    genesis is release-engine-proven `[KCP-COV-GEN-001]`. The last construction
    blocker — the silverc covenant-decl satisfier sigscript — was resolved
    **without** a `silverscript-lang` dependency by capturing the bytes once and
    splicing the tx-dependent oracle sig (see
    `crates/kcp-common/examples/reserve_covenant_live.rs` + design doc
    [docs/NEXT-STEPS-covenant-live-deploy.md](docs/NEXT-STEPS-covenant-live-deploy.md)).
    The method generalises: **all three pattern covenants are also live** —
    sealed-lineage (genesis `34d0e6f7…`, append `c7c24194…`) `[KCP-SL-003]`,
    transferable-record `[KCP-TR-003]`, and ktt-token `[KCP-KTT-003]` — each zero
    new code, just a new byte-capture fed to the same covenant-agnostic runner.
    Engine-tier design: [docs/NEXT-STEPS-introspection-enforcement.md](docs/NEXT-STEPS-introspection-enforcement.md).
- **v0 patterns are off-chain-validated.** `transferable-record` and
  `sealed-lineage` carry their invariants (sequence, identity, temporal
  envelope, mate proof) in the transaction payload and validate them off-chain.
  Expressing them as covenant declarations that consensus rejects bad
  successors is the documented next step.
- **ktt-token state transitions are off-chain in v0.** The 4-field KCC20 state
  is modelled and carrier-anchored `[KCP-KTT-001]`; the on-chain binding target
  (`validateOutputStateWithTemplate`) is verified to be real and engine-enforced
  `[SS-026]`, but the Kii covenant against it is not yet authored.
- **Governance lifecycle is anchorable, but enforcement is off-chain on the
  default path.** `kcp-governance::lineage` maps a `GovernorState` run onto a
  `kcp-sealed-lineage` lineage and re-verifies it **off-chain**. Two residuals to
  be honest about:
  - *Default anchoring is not consensus-enforced.* The shipped anchoring path
    (`kcp_sealed_lineage::tx::{create,append}_lineage_tx`) writes plain
    pay-to-address UTXOs; consensus does not introspect the payload, so the chain
    invariants (append-only sequence, stable identity, event-class, temporal) —
    and everything `verify_governor_lineage` checks — are validated off-chain
    only, exactly as the `kcp-sealed-lineage`/`kcp-transferable-record` modules
    state. The `[KCP-SL-003]` covenant that *does* make consensus reject a
    malformed successor is a separate covenant-id-bound artifact, and **no
    library API — including this binding — wires a governance payload into it**;
    doing so is a manual step. So enforcement is *available*, not *automatic*.
  - *Governance rules are never on-chain.* Even under the covenant, consensus
    enforces the lineage *structure*, not the governance *rules* — quorum before
    `Passed`, timelock elapse before `Executed`, and signatory legality all stay
    off-chain. The verifier's per-state quorum/timelock check only rejects a
    self-contradictory disclosed state; it does not prove the rules were followed.
  A dedicated Governor covenant that binds these transitions in consensus is the
  remaining gap.
- **SHA-256, not Poseidon.** `sealed-lineage` and `paired-attestation` use
  SHA-256 blinded commitments for reviewability; the donor's BN254 Poseidon
  (ZK-friendly) commitment is a KIP-16-era upgrade.
- **CSCI ZK binding: `OpZkPrecompile` half is deferred.** `portrait engrave` now emits
  `require(proof_cov_id == OpInputCovenantId(0))` in covenant transition functions when the
  role has a VProg counterpart (W15). This is the covenant-ID binding check: it verifies that
  the `proof_cov_id` argument (from the STARK journal bytes 0..32) matches the on-chain covenant
  ID. The STARK validity check itself — `OpZkPrecompile(0x21, journal)` — requires engine
  support for tag 0x21 which is not yet in the covenant VM. When engine support lands, a second
  `require` call covering the full journal will complete the binding. Until then, the emitted
  check is syntactically correct SilverScript accepted by silverc but does not enforce proof
  validity on-chain. See `docs/TIER3-CROSS-LAYER-BINDING.md` for the full protocol.

- **No post-quantum credential anchor.** Every credential anchor in this library
  (vault, paired-attestation, sealed-lineage, transferable-record) roots in
  secp256k1 Schnorr, which is not post-quantum-safe. KIP-16 `OpZkPrecompile`
  tag-0x21 (RISC Zero succinct STARK, PQ-safe: FRI + Poseidon2, no pairings) is
  live on testnet-10 and provides a concrete upgrade path: a RISC Zero guest
  verifies an ML-DSA-44 / SLH-DSA signature, the STARK is verified on-chain. A
  separate Kii project (`kii-ml-dsa`) has validated this pipeline on TN10 for
  ML-DSA-44 (FIPS-204), SLH-DSA (FIPS-205), and FN-DSA/Falcon-512 (FIPS-206
  draft). No PQ anchor code is in this library. SilverScript has no ZK-verify
  builtin; the pattern requires hand-rolled opcode assembly. Target home:
  `kcp-common::pq_anchor` or a dedicated `kcp-pq-anchor` crate. Phase 2+.
  See [docs/NEXT-STEPS-pq-anchor.md](docs/NEXT-STEPS-pq-anchor.md).

## Threat-model findings (2026-09-01, per-pattern threat-model pass)

Found while writing the per-pattern threat models (D7). Each is disclosed in the
crate's own `## Threat model` section; the ones below are the items where the
honest answer was "document it", not "fix it in this change".

- **`kcp-paired-attestation` v1 two-datasig is NOT custody — theft in flight.**
  `two_datasig_redeem_script` verifies two CSFS signatures over a fixed 32-byte
  `msg_hash` and contains **no `OP_CHECKSIG` over the spending transaction**, so
  the satisfier is a bearer credential that is not bound to the spend. The
  moment a spend enters any mempool, `sig_a`/`sig_b` are public in its signature
  script and **any observer (trivially, any miner) can spend the same outpoint
  to themselves** with the same satisfier; the identical satisfier also replays
  against any other UTXO under the same redeem script. Dominant attack is theft
  of the outpoint in flight, not the replay.
  *Ship-anyway rationale:* the pattern's purpose is **attestation signalling** —
  proving in-consensus that two named oracles signed one message — and it does
  that correctly (`[KCP-PA-002]`, engine-checked). Only dust-level value is ever
  placed under it in the examples. Binding the spend needs an `OP_CHECKSIG` over
  the transaction inside the redeem script, which (a) changes the redeem shape
  and therefore the P2SH address, invalidating the recorded `[KCP-PA-002]`
  evidence and its measured script-units budget, (b) requires deciding **whose**
  key binds the spend — a protocol-design question the two-oracle model does not
  answer, and (c) reorders the spend path, which today relies on CSFS being
  sighash-independent so the compute commitment can be set after signing. That
  is a new consensus-facing covenant shape and belongs in its own change with
  its own engine tests and red-team pass, not improvised here.
  *Directive until then:* **do not use the v1 covenant for custody.** Attestation
  signalling with dust only; never re-fund a two-datasig address.
- **`kcp-pq-anchor`: the tag-0x21 budget ceiling has ~0.25% headroom.**
  A version-0 input commits its budget as a `u8` `SigopCount`, so 255 is the
  **maximum expressible** budget — `255 × 100_000 + 9_999 = 25,509,999` script
  units. The shipped reference proof measures **25,446,182** units through a real
  P2SH spend (`tests/budget_ceiling.rs`), leaving **63,817 units — 0.25%**. A
  larger seal, a longer control-inclusion path, or a different guest can exceed
  the ceiling, and then the spend can never be budgeted: because the proof fields
  live inside the redeem script (and therefore inside the P2SH address), there is
  no alternative spending path and the value is **permanently unrecoverable**.
  Mitigation shipped: `sigop::measure_pq_anchor_units` +
  `sigop::fits_pq_verify_budget` — measure your proof shape **before funding**.
  Raising the ceiling itself is an engine-level matter (a compute-budget field,
  not a `u8`), not a library one.
- **`kcp-sealed-lineage`: the covenant does not bind the seal, and the publisher
  key can rotate silently.** The `[KCP-SL-003]` covenant state is
  `{lineage_id, seq, event_class, t_bucket, publisherPk}` — there is **no
  `commitment` field**, so the 87-byte payload's blinded seal is *never* enforced
  by consensus on any path, covenant or not. And `append` constrains
  `newStates[0]`'s seq, lineage_id, event_class and t_bucket but leaves
  `publisherPk` free, so an authorised publisher may hand control to a different
  key at any append without the covenant objecting. Both are structural
  properties of the committed artifact; changing either means recompiling the
  covenant (new script → new covenant-id → new evidence), so they are documented
  rather than patched. `kcp-governance` inherits both through its anchoring
  binding.
- **`kcp-vault`: `TimelockUnixSeconds` is a misnomer, and the off-chain
  evaluator disagrees with consensus about the unit.** Kaspa's
  `LOCK_TIME_THRESHOLD` is `500_000_000_000` and header timestamps come from
  `unix_now()`, which is **milliseconds** — so a `lock_time` at or above the
  threshold is Unix *milliseconds*, not seconds. A caller who supplies a genuine
  unix-seconds deadline (e.g. `1_800_000_000`) is below the threshold and gets a
  **DAA-score** timelock instead. Separately, `evaluator::EvalContext` compares
  the same deadline against a seconds-valued `unix_seconds` field, so the pure
  evaluator and consensus interpret one deadline in different units. Renaming the
  public variant and re-basing the evaluator is a breaking API change; for now
  the docs state the threshold and the unit, and callers should pass
  milliseconds.

## Engine-level findings carried forward

- **v0 `compile_condition` timelock is not P2SH-spendable.** It emits
  `... OP_CHECKLOCKTIMEVERIFY OP_DROP ...`; because Kaspa's CLTV *pops* the
  deadline, the spendable P2SH redeem omits `OP_DROP`
  (`compile_timelock_p2sh_redeem`). The v0 script is used only for the
  digest-anchor and the pure evaluator; v1 uses the corrected redeem `[KCP-VT-002]`.
- **CSFS covenant budget.** `OP_CHECKSIGFROMSTACK` is not a legacy sig-op, so a
  spend must commit a measured script-units budget or the node rejects it for
  under-budget. Handled by `measure_p2sh_script_units` + `covering_sigop_count`
  `[KCP-PA-002]`.

## Upstream (SilverScript) items

- **`checkDataSig` is a compiler no-op** `[SS-025]`. A proven one-function fix
  exists `[SS-024-v4]`; filing upstream is pending Foundation sign-off. The
  library does not depend on it.
- **Licence inconsistency** (ISC vs MIT) in upstream — reconciliation requested
  as issue #129 `[SS-023]`; gates publication.
- **AI-bot-credited commits** in load-bearing upstream files — legal flag
  pending `[SS-022]`; gates publication.
- **Silverscript stateless covenant declarations (May 2026)** — upstream now
  supports a `stateless` verification mode distinct from state-transition
  covenants, plus ternary operators and tuple field access (`.split()` returns
  tuples). Generated `.silver` files from the wizard CLI would benefit from
  these idioms. This library does not emit `.silver` source today; the compiler
  improvements are noted as a scaffold-enhancement opportunity for a future
  phase when the wizard generates SilverScript alongside Rust.
- **Silverscript `--constructor-args` now required (2026-06-28, c46e0e2)** — the
  new silverc requires `--constructor-args <CTOR.json>` for any contract with
  constructor parameters (`static_check_contract` enforces count equality). Our
  `.sil` source files in `crates/*/covenant/` fail to recompile without a CTOR
  JSON. The embedded compiled script bytes in the library are unaffected (compiled
  once; embedded; runtime uses those bytes, not the `.sil` source). CTOR JSON
  files will be added alongside each `.sil` to restore source recompilability.
  Portrait's `emit_ctor()` already generates CTOR JSON; the CLI invocation just
  needs `--constructor-args`. `[SS-027]`
- **`checkDataSig` → `checkSigFromStack` rename (2026-06-28, c46e0e2)** — the
  typed `checkSigFromStack(sig, data, pubkey)` builtin is now exposed. The old
  `checkDataSig` was a no-op `[SS-025]`; the new builtin is real. This does not
  affect the library today but unlocks inline data-signature verification for
  the CSCI covenant and `kcp-paired-attestation` in a future revision. `[SS-028]`

## Security-review hardening backlog (2026-06-12)

An adversarial security review (5 dimensions — sighash/signature, P2SH satisfier,
covenant binding, fee/dust/mass, panics/overflow — each finding independently
verified) found **no live CRITICAL/HIGH bugs** in the consensus-critical plumbing.
Two LOW/latent items were logged, then **applied same-day (Foundation-authorized
frozen-file change, commit `d1b29d1`)** — kept here as the record of what was
found and fixed:

- **Multisig lock/spend use two separate-but-byte-identical compilers.** The lock
  path derives the redeem via `p2sh_redeem_for`→`compile_condition_p2sh`
  (`kcp-vault/src/onchain.rs`), while `spend_multisig_vault` reaches into
  `crate::script::compile_condition`. They are byte-identical today (so funds are
  spendable), but a future edit to one and not the other would mint unspendable
  multisig vaults. Fixed: the spend now reuses `compile_condition_p2sh` (single source of truth) with an explicit shape guard, plus the CI test `multisig_redeem_identical_across_compilers` asserting redeem byte-equality for representative k-of-n shapes. (Composite Any/All paths already share the lock
  compiler — only the multisig leaf diverges.)
- **`amount - fee_sompi` underflow on caller error.** `spend_p2sh_tx` /
  `spend_p2sh_tx_with_locktime` / `lock_to_p2sh_tx` subtract caller-supplied
  fee/value with plain `-`; a fee exceeding the resolved UTXO panics (debug) or
  wraps (release) into a node-rejected tx. No fund loss (nothing valid is signed),
  but a poor failure mode on a value path. Fixed: `checked_add`/`checked_sub` with typed errors in lock_to_p2sh_tx, spend_p2sh_tx, spend_p2sh_tx_with_locktime.

## Phase-2 primitives (v0.2 work)

- **`ConditionInvalid` is defined independently in `kcp-common::error` and
  `kcp-vault::error`.** The two variants share a name and semantics but are
  separate types with no `From` conversion between them. Any caller bridging
  the two crates must write explicit mapping. A unified `kcp-common` error
  hierarchy (with `kcp-vault` converting via `From`) is the target shape for
  v0.2 once the full primitives layer stabilises.
- **`Ownable::validate()` is shape-only.** It confirms the 32-byte length but
  does NOT verify the key is on the secp256k1 curve. Callers embedding an
  `Ownable` key in a script must verify curve validity independently. A
  curve-validity check (using the engine's key primitives) is deferred to a
  future pass to avoid adding a secp256k1 dependency to the default feature set.
- **`kcp-common::cryptography::sign_schnorr` has no dedicated unit test.** The
  identical signing path is exercised by the `kcp-common::p2sh` engine
  round-trip tests (which run under the `wrpc` feature). A standalone keypair
  fixture that exercises `sign_schnorr` directly is deferred to v0.2.
- **`MerkleProof` and standalone CSFS helper are deferred** from
  `kcp-common::cryptography`. No Merkle tree in the current codebase to lift
  from; CSFS (`OP_CHECKSIGFROMSTACK`) is used in `kcp-paired-attestation` and
  will be extracted when that module is refactored.

## Portrait compiler / Tier 3

- **`silverc -c <file>` outputs a JSON with a `"script"` field** (array of u8 bytes).
  KovId = `sha256(script bytes)`. `portrait atelier-build` derives this automatically
  (W14) and embeds the result as `const COV_ID: [u8; 32]` in the generated RISC Zero
  guest. The field is named `"script"`, not `"bytecode"` — this was corrected in W16 docs.
- **Cross-layer ZK verification is engine-level (2026-06-28).** The on-chain STARK
  validity check is the engine KIP-16 tag-0x21 precompile, invoked from a
  `kcp-pq-anchor` raw P2SH redeem script — **not** a `silverc` surface opcode
  (`OpZkPrecompile` probes as `unknown function call`, identical to a fake builtin).
  Portrait emits the covenant-id binding only (`OpInputCovenantId`). See
  `docs/PORTRAIT-PHASE-A-UNBLOCK.md`.

## Operational

- **Testnet evidence is perishable.** Testnets reset by design `[ENV-TESTNET-RESET-2026]`;
  re-run the examples to refresh. Durable evidence belongs on mainnet
  post-Toccata activation. (The live covenant deployment `[KCP-RE-003]` is
  likewise testnet evidence of the mechanism, refreshable by re-running
  `reserve_covenant_live.rs`.)
