# KTT covenant artifacts

## Files

| File | Description |
|---|---|
| `ktt.sil` | SilverScript source — KTT-profiled covenant (KCC20 4-field shape) |
| `ktt.compiled.json` | Full compiled artifact (script bytes, ABI, AST, state_layout) |
| `ktt.script.hex` | Compiled script as a lowercase hex string (one line, no prefix) |

## Provenance

| Item | Value |
|---|---|
| SilverScript version | 0.1.0 — silverscript@2c46231 |
| rusty-kaspa engine | tag v2.0.0, commit 90dbf07 |
| Compiled script size | 1540 bytes |
| Compiled with args | genesisPk=[0x00]*32, genesisAmount=1000, genesisIdentifierType=0x00, genesisIsMinter=false, maxCovIns=2, maxCovOuts=2 — **read off the committed artifact**, not from `ktt_ctor.json` (see the warning below) |
| Build command | `silverc ktt.sil --constructor-args <the real args> -c` |
| state_layout | start=1, len=46 |

> **⚠ `ktt_ctor.json` is NOT the constructor input that produced
> `ktt.compiled.json`.** It says `genesisAmount=0, maxCovIns=1, maxCovOuts=1`;
> the committed artifact says otherwise on all three, three independent ways:
> its own state region decodes to `amount = 0x03e8 = 1000`; its program body
> bounds the covenant arity with `OpCovInputCount OpDup Op2 OpLessThanOrEqual`
> (and the same for `OpCovOutputCount`) rather than the `Op1` that
> sealed-lineage and transferable-record carry; and the engine accepts a
> 1-covenant-input → 2-covenant-output conserving split
> (`ktt_engine_accepts_conserving_split`). The `*_ctor.json` files post-date the
> artifacts and must not be cited as provenance — the real constructor input is
> `[FACT-NEEDED]`. The values in the table above are derived from the artifact
> bytes and are asserted in `tests/covenant_engine.rs`.

## State shape decision: 4-field (KCC20 shape)

The KTT covenant uses the exact KCC20 4-field state layout:

| Field | Type | Meaning |
|---|---|---|
| `ownerIdentifier` | `byte[32]` | Pubkey, script-hash, or covenant-id of the holder |
| `identifierType` | `byte` | 0x00=Pubkey, 0x01=ScriptHash, 0x02=CovenantId |
| `amount` | `int` | Token balance |
| `isMinter` | `bool` | True if this branch controls issuance |

A `byte complianceTier` 5th field was evaluated and deferred. The reason: the KCC20
compiler embeds all state fields into the script body and the `validateOutputState`
introspection primitive operates on the exact field layout determined at genesis. Adding a
5th field changes the script hash, breaking the covenant-id scheme and the
KCC20Minter composition pattern. Compliance-tier enforcement is handled off-chain via the
`transfer_rules` bitmask in `kcp-ktt-token` until the Kaspa ecosystem defines a standard
covenant-state extension mechanism. See `ktt.sil` lines 13-20 for the full rationale.

## Invariants enforced on-chain

| Code | Rule | Mechanism |
|---|---|---|
| KTT-1 | Supply conservation (!isMinter) | `checkAmounts` — engine-enforced via covenant introspection |
| KTT-2 | No minter escalation | `checkMintingTransfer` — engine-enforced |
| KTT-3 | Owner authorisation | `checkSigs` — checkSig / P2SH match / covenant-id match |

**Arity.** `#[covenant(binding = cov, from = maxCovIns, to = maxCovOuts)]` with
`maxCovIns = maxCovOuts = 2` in the committed artifact: **fan-out and merge are
permitted, bounded at 2 covenant inputs and 2 covenant outputs.** KTT-1 is a
*sum* over the covenant outputs, not a copy of a single amount — splitting
1000 → 400 + 600 is legal; 400 + 601 is not.

## Engine proof — reproducible in this repo

```sh
cargo test -p kcp-ktt-token --features wrpc --test covenant_engine
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
| `ktt_committed_artifact_matches_script_hex` | — | `ktt.compiled.json`'s `script` == `ktt.script.hex`; splicing state leaves the program body untouched; `state_region` reproduces the artifact's own genesis-template region (which is what fixes `genesisAmount = 1000`) |
| `ktt_engine_cost_matches_recorded_live_preflight` | — | the committed artifact costs exactly the 111 410 script units recorded for the live `[KCP-KTT-003]` preflight (corroboration of the deployed↔committed link — see Scope) |
| `ktt_engine_accepts_valid_handoff` | ACCEPT | baseline — supply-conserving 1→1 transfer |
| `ktt_engine_accepts_minter_changing_supply` | ACCEPT | positive control for KTT-1/KTT-2: a minter may mint |
| `ktt_engine_accepts_conserving_split` | ACCEPT | **1→2**: 1000 → 400 + 600 |
| `ktt_engine_rejects_amount_inflation` | REJECT | KTT-1 (1→1) |
| `ktt_engine_rejects_inflating_split` | REJECT | **KTT-1 on the shape it exists for** (1→2): 400 + 601 ≠ 1000 |
| `ktt_engine_rejects_minter_escalation` | REJECT | KTT-2 (1→1) |
| `ktt_engine_rejects_minter_escalation_on_split` | REJECT | KTT-2 on the *second* output of a 1→2 split |
| `ktt_engine_rejects_wrong_signature` | REJECT | KTT-3 |

**Scope.** This reproduces the **engine** proof only, and only at the script-VM
tier:

- *No transaction-level validation.* Only input 0's script runs — no transaction
  mass, no KIP-9 storage mass, no standardness. The covenant places no floor on
  the output value, so a caller must still respect `MIN_CHANGE_SOMPI` itself.
- *The 2-covenant-input shape is not exercised.* Merge is permitted by the
  artifact but the sibling-signature parse surface has no in-repo coverage; see
  `KNOWN-ISSUES.md`.
- *The live half is not reproducible.* The testnet-10 covenant-id-bound
  deployment `[KCP-KTT-003]` still rests on the recorded, perishable transaction
  ids. Nothing in this repo records the deployed script, its scriptPubKey or its
  on-chain `covenant_id`, so the committed↔deployed correspondence rests on an
  archived out-of-repo capture. The one in-repo corroboration is the
  execution-cost fingerprint above — evidence, not a binding.

A wider harness run before the in-repo test existed is kept as an archived
research artifact outside this published repo. Only the in-repo test runs in the
gate.
