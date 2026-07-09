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

Engine-proof is kept as an archived research artifact outside this published
repo.

## Invariants enforced

| Invariant | Description |
|---|---|
| TR-1 | `newState.seq == prevState.seq + 1` — monotone transfer count |
| TR-2 | `newState.record_id == prevState.record_id` — record identity preserved |
| TR-3 | `checkSig(s, prevState.controllerPk)` — current controller must sign |
| Structural | `from=1, to=1` — single-input single-output; fan-out and duplication structurally precluded by the covenant binding shape |
