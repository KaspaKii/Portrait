# sealed-lineage covenant — provenance

## What is here

| File | Description |
|---|---|
| `sealed-lineage.sil` | SilverScript source for the sealed-lineage on-chain enforcement covenant |
| `sealed-lineage.script.hex` | Compiled script (812 bytes, hex-encoded) |
| `sealed-lineage.compiled.json` | Full silverc JSON artifact (contract name, compiler version, script bytes, ABI) |

## Compilation provenance

- **SilverScript compiler**: `silverscript-lang` at commit `2c46231`
  (a local clone of `kaspanet/silverscript`; the compiled bytes embedded here are
  validated against the released rusty-kaspa v2.0.0 engine, see the library FACTS
  `KCP-COV-SKEW-001`)
- **Constructor args used**: zero-state (all-zero lineage_id / publisherPk,
  seq=0, event_class=0x00, t_bucket=1700000000) — these are the genesis
  template values. Each live UTXO is compiled with its own constructor args.
- **Script size**: 812 bytes

## Engine proof

The covenant is proven via an engine harness kept as an archived research
artifact outside this published repo.

12 tests run against `TxScriptEngine::from_transaction_input` with
`covenants_enabled: true` (the real rusty-kaspa engine, not a stub):

| Test | Verdict | Invariant |
|---|---|---|
| `sl_engine_accepts_valid_append_from_genesis` | ACCEPT | baseline |
| `sl_engine_accepts_valid_append_to_append` | ACCEPT | baseline |
| `sl_engine_accepts_close_transition` | ACCEPT | valid CLOSE step |
| `sl_engine_accepts_t_bucket_exactly_90_days` | ACCEPT | L-4 boundary |
| `sl_engine_rejects_seq_not_incremented` | REJECT | L-1 |
| `sl_engine_rejects_seq_skip` | REJECT | L-1 |
| `sl_engine_rejects_lineage_id_change` | REJECT | L-2 |
| `sl_engine_rejects_t_bucket_decreasing` | REJECT | L-4 |
| `sl_engine_rejects_t_bucket_exceeds_90_days` | REJECT | L-4 |
| `sl_engine_rejects_append_after_close` | REJECT | L-3 |
| `sl_engine_rejects_genesis_in_output_event_class` | REJECT | L-3 |
| `sl_engine_rejects_wrong_signature` | REJECT | ownership |

## How the library uses this

The library (`kcp-sealed-lineage`) stays on workspace tag `v2.0.0` and embeds
the compiled script as **data only** — no dependency on `silverscript-lang`.
The `.script.hex` file is the artifact a future runtime integration would use
to build the P2SH/covenant-bound UTXO scriptPubKey.

This was **engine-level proof** (local test harness); the covenant was then
deployed **live on testnet-10** `[KCP-SL-003]` (v0, unaudited, synthetic). It is
**covenant-id-bound** (not P2SH-wrapped) — consistent with the KCC20/KTT
state-continuity model.

## Note on `binding=cov` vs `binding=auth`

The compiler emits a warning: `binding=cov with from=1; binding=auth is usually
a better default`. This warning is advisory. `binding=cov` with `from=1, to=1`
is correct and functional for single-input single-output enforcement. `binding=auth`
would also work for the single-input case but is not structurally different for
this pattern. The covenant compiles and executes correctly either way; `binding=cov`
was chosen to match the KCC20 reference shape.
