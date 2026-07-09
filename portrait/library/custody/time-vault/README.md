# custody/TimeVault

A two-key, time-delayed withdrawal vault — the canonical Kaspa custody pattern (the **KiiVault** shape). Owner schedules a withdrawal; it can only settle after a relative-age delay; a separate cold **recovery** key can cancel a pending withdrawal at any time.

**Status:** 🟡 drafted — pre-review, TN10 only, not audited, not mainnet-safe.

## Parameters

| Param | Type | Meaning |
|---|---|---|
| `owner` | `pubkey` | Warm key. Schedules and finalises withdrawals. |
| `recovery` | `pubkey` | Cold key. Cancels a pending withdrawal (clawback). |
| `delay` | `int` | Minimum relative UTXO age before `settle` is allowed. |

## State

`pending` (0 idle / 1 scheduled) · `beneficiary` (blake2b commitment to the payout script) · `amount` (sompi to release).

## Lifecycle

```
idle  --schedule(ownerSig, beneficiary, amount)-->  pending
pending  --settle(ownerSig)        [age >= delay]-->  settled (terminal)
pending  --cancel(recoverySig)                  -->  idle
```

## Why it's safe by shape

- **No global state, no reentrancy.** State lives in this one UTXO; there is no external call surface to re-enter.
- **Committed payout.** `schedule` records a `blake2b` commitment to the payout script and the exact amount; `settle` enforces both. The owner cannot redirect funds at settle time.
- **Cold-key escape hatch.** Even if the warm key is compromised *after* a malicious schedule, the cold key cancels before the delay elapses — provided `delay` exceeds the holder's detection-and-response time.

## Threat model (summary)

Full rubric in `CONTRIBUTING.md`. Headline findings on the current draft:

| # | Sev | Axis | Finding | Disposition |
|---|---|---|---|---|
| 1 | **HIGH** | Technical | `tx.outputs[0].script` field name is assumed; if the real introspection accessor differs, `settle` mis-validates the payout. | **Open** — pin against `silverscript-lang/std` before stable. |
| 2 | **HIGH** | Operational | Security rests entirely on `delay` > holder response time. Too small a delay defeats the cold-key protection. | Documented; Composer should warn on low `delay`. |
| 3 | MEDIUM | Economic | `settle` checks `outputs[0]`, not total value conservation; a second output could siphon the remainder/change. | Add explicit change/total-value check; tracked. |
| 4 | MEDIUM | Covenant-lineage | Genesis griefing — anyone can create a look-alike covenant; off-chain code must verify the covenant ID, not just the script. | Belongs in the Composer manifest + indexer guidance. |
| 5 | LOW | Technical | `cancel` returns `beneficiary` unchanged (cosmetic); harmless but should zero it for cleanliness. | Will zero in revised version. |

**Gate:** must not be promoted to 🟢 until findings 1 and 2 are resolved or formally accepted. This is a draft for review, not production code.

## Files

- `TimeVault.sil` — the Silverscript component.
- `TimeVault.portrait` — the canonical covenant source (role/lifecycle/flow/invariant); `portrait engrave` lowers it to `.sil` + CTOR JSON that `silverc` accepts.
- The app-composition wrapper showing idiomatic *use* of TimeVault lives at `../../../examples/app-composition/time-vault.portrait`. It uses the app-composition grammar (`contract vault = TimeVault { ... }`), which is **not** a covenant source — keep the library covenant tree limited to engravable covenant sources.
- `tests/` — *(to add)* golden compiled-script + `cli-debugger` accept/reject vectors per entrypoint.

## Prior art

Time-delayed and two-key vaults are well-trodden in Bitcoin covenant research and CashScript; the relative-age idiom (`this.age`) follows Silverscript's own Mecenas example.
