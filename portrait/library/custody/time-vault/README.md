# custody/TimeVault

A two-key, one-shot custody covenant — the canonical Kaspa custody shape. A hot
**owner** key may `release` the vault once a committed time bucket has opened; a
separate cold **recovery** key may `claw` the vault back at any time before it is
released. Once either path fires, a one-shot flag closes both.

**Status:** 🟡 drafted — pre-review, TN10 only, not audited, not mainnet-safe.

## Honest scope (read this first)

This README describes exactly what the emitted `TimeVault.sil` enforces — nothing
more. The covenant is a **state-authorisation + time-gated** covenant: it binds
*who* may transition the UTXO, *when the transition is one-shot*, and enforces a
**consensus** time gate on `release`. It does **not** move coin, commit a payout
amount, or pin a destination. See `library/ENFORCEMENT.md` for the per-guarantee
classification.

- **No value/payout/payee constraint.** The `.sil` constrains no transaction
  output amount and no output script. There is no `beneficiary`, no `amount`, no
  "release to X". Where the coin actually goes is the spending wallet's
  responsibility; the covenant does not enforce it.
- **The time gate is consensus-enforced (CLTV + finalization).** `release` carries
  an `after(unlock_bucket)` clause, which the emitter lowers to
  `require(tx.time >= prev_states[0].unlock_bucket)` → the engine's
  `OpCheckLockTimeVerify`. The "cannot spend before the deadline" guarantee is
  **two** consensus rules, and the emitted opcode is only half:
    1. **CLTV** enforces that the tx **commits** a `lock_time >= unlock_bucket` AND
       the spending input is **non-final** (defeating the final-sequence bypass). It
       reads only the spender-set `lock_time` field — it has no access to the block
       DAA score, so it does not by itself prove the deadline has elapsed.
    2. The actual "not included before the deadline" rule is the **separate**
       consensus finalization check `check_tx_is_finalized`: a non-final tx with
       `lock_time = L` is admissible into the blockDAG only once
       `block_daa_score > L`.
  Together, consensus bars `release` from a block until the DAA score passes
  `unlock_bucket`. Two ceremony preconditions the covenant cannot check: (a) the
  committed `unlock_bucket` and the spending tx's lock time must be in the SAME
  domain — a DAA score below `LOCK_TIME_THRESHOLD = 500_000_000_000`, or a Unix
  time at/above it; (b) a committed `unlock_bucket` of `0` (which maps to
  "finalized") or any value `<=` the DAA score at instantiation opens the gate
  fully — the instantiation ceremony must commit a real **future** deadline.

## Parameters / state

The constructor initialises each state field from the param of the SAME NAME (a
state field with no same-named param is a compile error), so params and state
share these four fields:

| Field | Type | Meaning |
|---|---|---|
| `owner` | `pubkey` | Hot key permitted to `release` after the time bucket opens. |
| `recovery` | `pubkey` | Cold clawback key; may `claw` at any time before release. |
| `unlock_bucket` | `int` | Committed coarse time bucket at/after which `release` is allowed. |
| `released` | `int` | One-shot spent flag (genesis = 0; set to 1 by either path). |

## Entrypoints

```
release(auth)               requires checkSig(auth, owner)
                            after(unlock_bucket)                   (consensus CLTV gate)
                            requires released == 0                 (one-shot)
                            → { owner, recovery, unlock_bucket, released: 1 }

claw(auth)                  requires checkSig(auth, recovery)
                            requires released == 0                 (one-shot)
                            → { owner, recovery, unlock_bucket, released: 1 }
```

Both transitions preserve `owner`, `recovery`, and `unlock_bucket` unchanged and
set `released` to 1, so neither path can fire again on the successor state.

## What is actually enforced (by the emitted `.sil`)

- **SCRIPT-ENFORCED — authorisation.** `checkSig` binds each path to a key
  committed in the covenant state (`owner` for `release`, `recovery` for `claw`).
- **SCRIPT-ENFORCED — one-shot.** `require(released == 0)` on both paths, with
  `released` set to 1 in the successor, prevents a second spend along either
  path.
- **SCRIPT-ENFORCED — covenant continuity.** The covenant binds `cov`: the
  successor must carry the same covenant program (`binding = cov`), so the state
  cannot be laundered into an unconstrained UTXO.
- **SCRIPT-ENFORCED — time gate (CLTV + finalization).** `after(unlock_bucket)`
  emits `require(tx.time >= prev_states[0].unlock_bucket)` → `OpCheckLockTimeVerify`:
  the tx must **commit** a `lock_time >= unlock_bucket` on a **non-final** input
  (defeating the final-sequence bypass). The "deadline has elapsed" step is the
  separate consensus finalization rule (`block_daa_score > unlock_bucket`); the
  emitted opcode is the lock-time-field half only (see Honest scope above).
- **NOT ENFORCED — value / payout / destination.** The covenant does not move or
  bound any coin.

## Files

- `TimeVault.sil` — the emitted Silverscript component (the source of truth for
  this README).
- `TimeVault.portrait` — the canonical covenant source (role/lifecycle/flow/
  invariant); `portrait engrave` lowers it to `.sil` + CTOR JSON that `silverc`
  accepts (exit 0).
- The app-composition wrapper showing idiomatic *use* of TimeVault lives at
  `../../../examples/app-composition/time-vault.portrait`. It uses the
  app-composition grammar (`contract vault = TimeVault { ... }`), which is **not**
  a covenant source — keep the library covenant tree limited to engravable
  covenant sources.

## Prior art

Time-gated and two-key vaults are well-trodden in Bitcoin covenant research and
CashScript. Like a Bitcoin CLTV timelock, TimeVault's `release` gate is a
consensus lock-time check (`OpCheckLockTimeVerify`), including the bundled
non-final-input check that defeats the final-sequence bypass (see Honest scope).
