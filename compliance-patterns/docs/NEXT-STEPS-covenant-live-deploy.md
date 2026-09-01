# Next step: covenant live deployment (engine-proven → on-chain-live)

This note scopes the work to take a state-continuity covenant from
**engine-proven** (where the library is today) to **live on testnet-10**. It is
the sequel to [`NEXT-STEPS-introspection-enforcement.md`](NEXT-STEPS-introspection-enforcement.md),
which scoped the engine-tier enforcement that is now proven. This is a design
and a gap analysis, **not** a claim of completed work.

## Where we are

- **Activation is no longer the blocker.** Toccata covenant introspection is
  **active on testnet-10 today** — `covenants_enabled =
  toccata_activation.is_active(block_daa_score)`
  (`consensus/src/processes/transaction_validator/tx_validation_in_utxo_context.rs:171`),
  testnet-10 activates at DAA `467_579_632`
  (`consensus/core/src/config/params.rs`), and the live node reports virtual DAA
  `488_553_862` — past activation by ~21M DAA. Recorded as `[KCP-NET-002]`.
- **The covenants are engine-proven** against the real script engine with
  `covenants_enabled: true`: reserve `[KCP-RE-001]`, sealed-lineage
  `[KCP-SL-002]`, transferable-record `[KCP-TR-002]`, ktt `[KCP-KTT-002]`.
- **What remains is purely construction.** A live deploy needs a real
  covenant-bound transaction the node will accept — and that is blocked on three
  concrete things below, not on consensus eligibility.

## The on-chain covenant lifecycle (grounded in v2.0.0 @ 90dbf07)

`CovenantsContext::from_tx` (`crypto/txscript/src/covenants.rs:101`) distinguishes
two cases per output that carries a `CovenantBinding { covenant_id,
authorizing_input }`:

1. **Genesis** — the authorizing input's UTXO does **not** carry this
   `covenant_id`. The output establishes a new lineage. Its `covenant_id` is
   **validated by reconstruction**: it must equal
   `hashing::covenant_id::covenant_id(input.previous_outpoint, [(out_idx,
   output)…])` (`covenants.rs:152`), else `WrongGenesisCovenantId`. Genesis
   outputs are validated but **do not run the covenant script** — they just mint
   the covenant UTXO with its derived id.
2. **Continuation** — the authorizing input's UTXO **does** carry the same
   `covenant_id` (`covenants.rs:125`). Now the covenant **script runs**, and the
   introspection opcodes (`validateOutputState`, `readInputState`, …) enforce the
   state transition against the successor output.

So a live lineage is: **tx-0 (genesis)** locks value to `P2SH(state_0_script)`
with a `CovenantBinding` whose id is the derived genesis id; **tx-1 (append)**
spends that UTXO (now carrying the covenant id) and produces
`P2SH(state_1_script)` with the same id — triggering enforcement.

## What is constructible from the v2.0.0 deps the library already has

These need **no** `silverscript-lang` and are reachable from `kaspa-consensus-core`
/ `kaspa-txscript` (the workspace pins, under the `wrpc` feature):

- Genesis covenant-id derivation — `hashing::covenant_id::covenant_id(...)` is
  `pub` (`consensus/core/src/hashing/covenant_id.rs:16`).
- `CovenantBinding { authorizing_input: u16, covenant_id: Hash }` — public fields
  (`consensus/core/src/tx.rs:193`); `TransactionOutput.covenant: Option<…>`.
- The P2SH lock to a covenant script (given the **compiled script bytes**), tx
  assembly, schnorr sighash + oracle signing, offline engine preflight
  (`kcp_common::p2sh::verify_p2sh_spend_offline` pattern, extended with a
  `CovenantsContext`), and mempool submission via the existing wRPC client.

## The three real blockers

1. **The covenant-call sigscript is silverc-emitted, and the sig is
   tx-dependent.** The append spend's signature script is
   `build_sig_script_for_covenant_decl("append", [newStates: State[],
   oracleSig], leader)` + the redeem-script push (see
   `CAVEATS/08-reserve-covenant/covenant_engine_tests.rs:145`). The `State[]`
   argument uses silverc's covenant-decl calling ABI, and the oracle signature
   signs the actual tx sighash — so the sigscript **cannot be fully pre-staged**
   offline. Resolving this means either (a) reimplementing that ABI byte-exact in
   pure `kaspa-txscript` (and re-proving it against the engine), or (b) running
   the final build+sign+submit from a context that still has `silverscript-lang`.
   **`silverscript-lang` must never become a dependency of a `kcp-*` crate** —
   that ban is a standing guardrail (it would make the library unbuildable for
   others and re-pin the workspace off `v2.0.0`).
2. **Engine skew: 42b734f (v1.1.1-toc.1) vs v2.0.0 (90dbf07) — now
   measured, and benign for covenant validation `[KCP-COV-SKEW-001]`.** A direct
   source diff of the two cargo-cache checkouts shows the covenant **validation
   semantics are byte-identical**: `crypto/txscript/src/covenants.rs`
   (`CovenantsContext::from_tx`, genesis-id reconstruction),
   `consensus/core/src/hashing/covenant_id.rs` (the id derivation), and
   `crypto/txscript/src/engine_context.rs` are **IDENTICAL**, and every
   introspection-opcode implementation is unchanged (the only covenant-opcode
   diff in `opcodes/mod.rs` is one uncommented *test* line). The covenant
   binding's contribution to the sighash is unchanged. The covenant-mentioning
   deltas are all benign: (a) renamed-but-equal resource constants
   (`MAX_SCRIPT_ELEMENT_SIZE` → `…_POST_TOCCATA`, same value); (b) a
   tx-**construction** field rename (`TxInputMass`/`input.mass` →
   `ComputeCommit`/`input.compute_commit`) — present in the v2.0.0 deps the
   library already pins; (c) test scaffolding. **Consequence:** the embedded
   compiled covenant *script bytes* (opcodes identical) and the engine proof
   transfer to v2.0.0; this blocker reduces to *building the tx with the v2.0.0
   field names and re-running the offline preflight on `90dbf07` to confirm* —
   not a risk that the proof fails to hold. The hard gate below still applies.
3. **Genesis — now engine-proven on the release engine `[KCP-COV-GEN-001]`.**
   `kaspa-txscript`'s own `covenants::tests` were built and run against the
   `90dbf07` checkout: **8/8 pass**, including `test_genesis_single_output`,
   `test_genesis_invalid_covenant_id` (a wrong id is rejected with
   `WrongGenesisCovenantId`), and `test_continuation_with_genesis` (the
   genesis→append handoff). The canonical construction is `create_genesis_tx`
   setting `binding.covenant_id = covenant_id(input.previous_outpoint,
   authorized_outputs)` (`covenants.rs:152`, `:248`). So the **consensus side of
   genesis is proven** on the exact engine testnet-10 runs; what remains is purely
   *building* the genesis tx with the derived id — which the hard-gate preflight
   verifies. (The compiled state scripts depend only on state values, not the tx,
   so they can be pre-staged for a fixed, small lineage of genesis + N appends.)

**Net — DONE 2026-06-12 `[KCP-RE-003]`:** all three blockers resolved; the reserve
covenant is **LIVE on testnet-10** (genesis `fcecef64…`, append `980ca03a…`,
covenant_id `7ba54cfa…`). (2) the engine proof transfers `[KCP-COV-SKEW-001]`;
(3) genesis is release-engine-proven `[KCP-COV-GEN-001]`; (1) the silverc
covenant-decl satisfier ABI was resolved **without** a `silverscript-lang`
dependency by capturing the sigscript bytes once from silverc and splicing the
tx-dependent oracle signature into the sole variable 65-byte region (proven by
diffing two dummy-sig captures). See
`crates/kcp-common/examples/reserve_covenant_live.rs`.

## Two construction paths

- **Path A — recommended.** Confirm/obtain a silverc target for the
  **v2.0.0 release** covenant ABI; re-run the reserve harness against the
  `90dbf07` engine (inside the silverscript clone, embed only the
  re-proven compiled bytes). Reimplement the covenant-decl sigscript assembly in
  pure `kaspa-txscript` in a `kcp-*` **example** (not a library module that ships
  the ABI), derive genesis id with `hashing::covenant_id::covenant_id`, **preflight
  the full genesis+append against the `90dbf07` engine offline**, then submit
  ≤1 KAS. The `kcp-*` crates stay free of `silverscript-lang`.
- **Path B — faster.** Build+sign+submit from inside the silverscript
  clone (which has `silverscript-lang`) using a v2.0.0 wRPC client. Keeps the
  `kcp-*` crates free of the dependency (the submitter is a throwaway harness), but requires the
  clone to talk to a v2.0.0 node and still requires the engine-skew re-proof. Use
  only if Path A's pure-txscript ABI reimplementation proves too costly.

## The hard gate (do not skip)

**No covenant transaction is submitted to any node until the byte-identical
transaction has passed an offline preflight against the v2.0.0 release engine
(`90dbf07`) with `covenants_enabled: true` and a `CovenantsContext::from_tx`
built from that exact tx.** Lock ≤1 KAS. This is the same discipline every P2SH
spend in this library already follows (`verify_p2sh_spend_offline`), extended
with the covenant context. Locking value into a covenant whose spend has not been
release-engine-proven risks **unspendable funds** — which is precisely why this
gate must not be skipped.

## Checklist

- [x] ~~Verify the compiled covenant bytes are accepted by `90dbf07`.~~ **Done by source
      diff `[KCP-COV-SKEW-001]`:** covenant opcodes + validation semantics are
      byte-identical across the two engines, so the embedded compiled scripts are
      valid on v2.0.0. (silverc's release-ABI target need not be re-obtained for
      the *script*; it is only relevant to the sigscript-ABI item below.)
- [x] ~~Confirmatory preflight on `90dbf07`.~~ **Done** — the append is
      offline-preflighted on the v2.0.0 engine with a real `CovenantsContext::from_tx`
      (`covenants_enabled`) before every submit, and a `KCP_DRY_RUN` mode proves
      ACCEPT (valid) + REJECT (non-incremented state, wrong oracle sig).
- [x] ~~Prove genesis-id derivation on the release engine.~~ **Done
      `[KCP-COV-GEN-001]`** — `covenants::tests` 8/8 on `90dbf07`.
- [x] ~~Resolve blocker (1): the covenant-decl satisfier sigscript without
      `silverscript-lang`.~~ **Done** — captured the sigscript bytes from silverc
      once (`CAVEATS/08/live-capture.json`) and splice the tx-dependent oracle sig
      into the sole variable 65-byte region (two-dummy-diff proven). Embedded as
      data; no library dep.
- [x] ~~Build genesis tx; derive id; offline-preflight genesis + append.~~ **Done**
      in `reserve_covenant_live.rs` (`covenant_id` derivation; dry-run gate).
- [x] ~~Submit ≤1 KAS genesis + append to testnet-10; record FACT.~~ **Done
      `[KCP-RE-003]`** — genesis `fcecef64…`, append `980ca03a…` on testnet-10
      (kaspad v2.0.0).

**Status (2026-06-12): DONE.** The anchor-only reserve covenant is deployed LIVE
on testnet-10 in a covenant-id-bound genesis+append, consensus-enforced
`[KCP-RE-003]`. Remaining gates are the usual non-technical ones — mainnet,
external audit, and the legal-opinion gate for real reserve data.

## Appendix: blocker (1) — the satisfier ABI, specified

Read from `silverscript-lang/src/compiler/mod.rs` (read-only; the clone was not
modified). This is the spec a pure-`kaspa-txscript` reimplementation must match
byte-for-byte; the reimplementation **must** be validated against silverc's
actual output as an oracle (run `build_sig_script_for_covenant_decl` on the exact
`append` args, compare bytes), because the array/struct encodings have subtle
cases.

The append spend's signature script is, in order:

1. **The covenant-decl call** =
   `build_sig_script_for_covenant_decl("append", [newStates, oracleSig], {is_leader:true})`
   (`mod.rs:209`), which resolves `"append"` to the generated `auth` entrypoint
   if present, else the `leader`/`delegate` entrypoint, then calls
   `build_sig_script(entrypoint, args)` (`mod.rs:179`):
   - For each ABI input in **declaration order**, `push_typed_sigscript_arg`
     (`mod.rs:234`):
     - **`State[]`** → *struct-of-arrays*: for each struct field in **definition
       order** (`lineage_id, seq, attestationHash, oraclePk`), push a dynamic
       array of that field's value across the struct elements (one element for
       `from=1,to=1`), recursively.
     - **scalar** (`push_sigscript_arg`, `mod.rs:340`): `Int`→`add_i64`,
       `Bool`→`add_i64(0|1)`, `Byte`/byte-array→`add_data`, `String`→`add_data`,
       `Date`→`add_i64`.
     - **typed array**: byte-array→`add_data(bytes)`; otherwise
       `encode_array_literal(values,type)`→`add_data` (the fiddly case to match).
   - Then, unless `without_selector`, append `add_i64(function_branch_index(ast,
     entrypoint))` — the branch selector (`mod.rs:202`). Extract this integer once
     from silverc for the chosen entrypoint and treat it as a fixed constant.
2. **The redeem-script push** = `ScriptBuilder::new().add_data(&covenant_script)`
   appended after the call (the P2SH redeem; see
   `CAVEATS/08-reserve-covenant/covenant_engine_tests.rs:158`).

The `oracleSig` element is tx-dependent (schnorr over the sighash, `SIG_HASH_ALL`
byte appended), so it is computed at build time, not pre-staged. Everything else
(the field pushes, the selector, the redeem push) is deterministic from the
state values and can be unit-fixed. With this ABI reproduced and byte-equality-
proven, plus the genesis construction `[KCP-COV-GEN-001]` and the embedded
compiled script `[KCP-COV-SKEW-001]`, the full genesis+append can be built in a
`kcp-*` example, offline-preflighted on `90dbf07`, and submitted.
