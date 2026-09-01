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
  template values, **read off the committed artifact's own state region** and
  asserted in `tests/covenant_engine.rs`. Each live UTXO is compiled with its
  own constructor args.
- **Script size**: 812 bytes

> **⚠ `sealed-lineage_ctor.json` is NOT the constructor input that produced
> `sealed-lineage.compiled.json`.** It gives `genesisTBucket=0`, while the
> artifact's state region encodes `1700000000`. The `*_ctor.json` files post-date
> the artifacts; the real constructor input is `[FACT-NEEDED]`. Do not cite them
> as provenance.

## Engine proof — reproducible in this repo

```sh
cargo test -p kcp-sealed-lineage --features wrpc --test covenant_engine
```

`tests/covenant_engine.rs` loads the **committed artifact above** and runs it
through `TxScriptEngine::from_transaction_input` with `covenants_enabled: true`
and a real `CovenantsContext::from_tx` — the pinned rusty-kaspa v2.0.0 script
VM, not a stub. Per-state scripts are produced by splicing the artifact's
`state_layout` region (`kcp_common::covenant`), so there is still no
`silverscript-lang` dependency and no key fixture; each test derives a
deterministic keypair from a fixed, never-funded seed.

Every REJECT below is a **two-sided** case: the violating transition must be
refused by the covenant's own `require`, *and* the same transition with only the
offending field restored must be accepted. A bare `VerifyError` cannot say which
`require` fired, so the paired control is what pins the rejection to the named
invariant.

| Test | Verdict | Invariant |
|---|---|---|
| `sl_committed_artifact_matches_script_hex` | — | `sealed-lineage.compiled.json`'s `script` == `sealed-lineage.script.hex`; splicing state leaves the program body untouched; `state_region` reproduces the artifact's own genesis-template region |
| `sl_engine_cost_matches_recorded_live_preflight` | — | the committed artifact costs exactly the 107 149 script units recorded for the live `[KCP-SL-003]` preflight (corroboration of the deployed↔committed link — see Scope) |
| `sl_engine_accepts_valid_append_from_genesis` | ACCEPT | baseline |
| `sl_engine_accepts_valid_append_to_append` | ACCEPT | the chain continues past the first step |
| `sl_engine_accepts_close_transition` | ACCEPT | L-3 — CLOSE is a legal append |
| `sl_engine_accepts_t_bucket_exactly_90_days` | ACCEPT | L-4 boundary |
| `sl_engine_rejects_seq_not_incremented` | REJECT | L-1 |
| `sl_engine_rejects_seq_skip` | REJECT | L-1 |
| `sl_engine_rejects_lineage_id_change` | REJECT | L-2 |
| `sl_engine_rejects_append_after_close` | REJECT | L-3 — **terminality**: a CLOSE state can never be spent |
| `sl_engine_rejects_genesis_in_output_event_class` | REJECT | L-3 |
| `sl_engine_rejects_t_bucket_exceeds_90_days` | REJECT | L-4 |
| `sl_engine_rejects_t_bucket_decreasing` | REJECT | L-4 |
| `sl_engine_rejects_wrong_signature` | REJECT | ownership |

**Scope.** This reproduces the **engine** proof only, and only at the script-VM
tier:

- *No transaction-level validation.* Only input 0's script runs — no transaction
  mass, no KIP-9 storage mass, no standardness. The covenant places no floor on
  the output value, so a caller must still respect `MIN_CHANGE_SOMPI` itself.
- *The live half is not reproducible.* The testnet-10 covenant-id-bound
  deployment `[KCP-SL-003]` still rests on the recorded, perishable transaction
  ids. Nothing in this repo records the deployed script, its scriptPubKey or its
  on-chain `covenant_id`, so the committed↔deployed correspondence rests on an
  archived out-of-repo capture. The one in-repo corroboration is the
  execution-cost fingerprint above — evidence, not a binding.

### Archived wider harness

A wider harness was run before the in-repo test existed and is kept as an
archived research artifact outside this published repo. Its cases are now all
ported here — the L-3 CLOSE rules, the L-4 boundary, `seq_skip` and
`append_to_append` are each a different *value* in the same spliced state
region, so they needed nothing the in-repo harness lacks. Only the in-repo test
runs in the gate.

## How the library uses this

The library (`kcp-sealed-lineage`) stays on workspace tag `v2.0.0` and embeds
the compiled script as **data only** — no dependency on `silverscript-lang`.
The `.script.hex` file is the artifact a future runtime integration would use
to build the P2SH/covenant-bound UTXO scriptPubKey.

The engine-level proof above is reproducible here; the covenant was then
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
