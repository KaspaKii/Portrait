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
| Compiled with args | genesisPk=[0x00]*32, genesisAmount=1000, genesisIdentifierType=0x00, genesisIsMinter=false, maxCovIns=2, maxCovOuts=2 |
| Build command | `silverc ktt.sil --constructor-args ktt_ctor.json -c` |
| state_layout | start=1, len=46 |

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
