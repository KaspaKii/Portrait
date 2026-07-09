# custody/DeadMansSwitch

An inactivity-triggered inheritance covenant. The `owner` keeps the UTXO alive
with a periodic signed **heartbeat**; if the owner goes silent for longer than a
committed `timeout`, the `heir` may **claim** control. The on-chain "dead man's
switch" / inheritance pattern.

**Status:** 🟡 drafted — pre-red-team, testnet-only, not audited, not mainnet-safe.

## Parameters / State

One constructor param per state field, in field order:

| Field | Type | Meaning |
|---|---|---|
| `owner` | `pubkey` | Committed owner key. Heartbeat (liveness) authority. |
| `heir` | `pubkey` | Committed heir key. Claim (inheritance) authority. |
| `last_active` | `int` | Coarse time bucket of the last proven heartbeat. |
| `timeout` | `int` | Inactivity window (buckets) after which the heir may claim. |

## Lifecycle

```
live --heartbeat(ownerSig, now_bucket)                              --> live   (last_active := now_bucket)
live --claim(heirSig, now_bucket)  [now_bucket >= last_active+timeout] --> live   (owner := heir; last_active := now_bucket)
```

## Why it's safe by shape

- **Committed-key authorisation (C2).** Both transitions `checkSig` against a
  COMMITTED state key — heartbeat against `owner`, claim against `heir` — never a
  caller-supplied pubkey. An attacker controlling neither key can neither forge a
  heartbeat to keep the owner "alive" nor forge an early claim. The `authorized`
  invariant makes this a stated, enforced property.
- **Inactivity gate.** `claim` is gated on `now_bucket >= last_active + timeout`;
  each heartbeat pushes the deadline forward by `timeout`.
- **No global state, no reentrancy.** State lives in this one UTXO; no external
  call surface to re-enter.

## Honest scope

- **`now_bucket` is caller-asserted and coarse**, exactly as in TimeVault and
  Escrow. The covenant does NOT read wall-clock time. The consensus
  relative-timelock (the real "can this be relayed yet" decision) is enforced by
  the engine's relative-time rule on the spending input's sequence, set by the
  wallet; the covenant complements it with the bucket bound.
- **Security rests on `timeout` exceeding the owner's heartbeat cadence plus
  slack.** Too small a `timeout` lets the heir claim while the owner is merely
  briefly offline.
- **Semantic checks are structural/relational, not an SMT solver** (per-field,
  no cross-field flow proof).
- Pre-production, unaudited, testnet-only.

## Files

- `DeadMansSwitch.portrait` — the canonical covenant source (role/lifecycle/
  invariant). `portrait engrave` lowers it to `.sil` + CTOR JSON that `silverc`
  accepts (exit 0).
- `DeadMansSwitch.sil` — the emitted Silverscript component.
- `DeadMansSwitch_ctor.json` — the emitted CTOR JSON consumed by `silverc --ctor`.
- `DeadMansSwitch.json` — the `silverc`-compiled script.

## Reproduce

```sh
cd portrait
cargo run --bin portrait -- check   ../library/custody/dead-mans-switch/DeadMansSwitch.portrait
cargo run --bin portrait -- engrave ../library/custody/dead-mans-switch/DeadMansSwitch.portrait
```
