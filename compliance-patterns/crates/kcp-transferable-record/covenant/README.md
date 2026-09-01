# kcp-transferable-record covenant

## Files

| File | Description |
|---|---|
| `transferable-record.sil` | SilverScript source for the covenant |
| `transferable-record.script.hex` | Compiled P2SH-ready script (548 bytes, hex) |
| `transferable-record.compiled.json` | Full compiler output including AST and ABI |

## Provenance

Compiled with `silverc` (silverscript@2c46231), validated against the released
rusty-kaspa v2.0.0 (the Toccata engine). The library workspace stays at
`tag=v2.0.0` (commit `90dbf07`); the compiled script is embedded as data only.
The `silverscript-lang` crate is NOT added as a dependency of any library crate.

## Invariants enforced

| Invariant | Description |
|---|---|
| TR-1 | `newState.seq == prevState.seq + 1` — monotone transfer count |
| TR-2 | `newState.record_id == prevState.record_id` — record identity preserved |
| TR-3 | `checkSig(s, prevState.controllerPk)` — current controller must sign |
| Structural | `from=1, to=1` — single-input single-output; fan-out and duplication structurally precluded by the covenant binding shape |

## Engine proof — reproducible in this repo

```sh
cargo test -p kcp-transferable-record --features wrpc --test covenant_engine
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
| `tr_committed_artifact_matches_script_hex` | — | `transferable-record.compiled.json`'s `script` == `transferable-record.script.hex`; splicing state leaves the program body untouched; `state_region` reproduces the artifact's own genesis-template region |
| `tr_engine_cost_matches_recorded_live_preflight` | — | the committed artifact costs exactly the 105 047 script units recorded for the live `[KCP-TR-003]` preflight (corroboration of the deployed↔committed link — see Scope) |
| `tr_engine_accepts_valid_first_transfer` | ACCEPT | baseline |
| `tr_engine_accepts_second_transfer` | ACCEPT | the chain continues past the first hop |
| `tr_engine_rejects_seq_not_incremented` | REJECT | TR-1 |
| `tr_engine_rejects_seq_skip` | REJECT | TR-1 |
| `tr_engine_rejects_record_id_change` | REJECT | TR-2 |
| `tr_engine_rejects_wrong_signature` | REJECT | TR-3 |

**Scope.** This reproduces the **engine** proof only, and only at the script-VM
tier:

- *No transaction-level validation.* Only input 0's script runs — no transaction
  mass, no KIP-9 storage mass, no standardness. The covenant places no floor on
  the output value, so a caller must still respect `MIN_CHANGE_SOMPI` itself.
- *The live half is not reproducible.* The testnet-10 covenant-id-bound
  deployment `[KCP-TR-003]` still rests on the recorded, perishable transaction
  ids. Nothing in this repo records the deployed script, its scriptPubKey or its
  on-chain `covenant_id`, so the committed↔deployed correspondence rests on an
  archived out-of-repo capture. The one in-repo corroboration is the
  execution-cost fingerprint above — evidence, not a binding.

A wider harness run before the in-repo test existed is kept as an archived
research artifact outside this published repo. Its transferable-record cases are
now all ported here. Only the in-repo test runs in the gate.
