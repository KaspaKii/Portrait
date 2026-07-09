# finance/Subscription

A recurring, rate-limited pull-payment covenant. A prepaid subscription UTXO lets
a committed `provider` pull a fixed `amount_per_period` from a `subscriber`-funded
`balance`, but no more than once per `period`. The on-chain recurring-billing /
standing-order pattern: the merchant may charge on a cadence, the customer is
protected from being charged faster than the agreed rate, and the running balance
is drawn down one charge at a time until exhausted.

**Status:** 🟡 drafted — pre-red-team, testnet-only, not audited, not mainnet-safe.

## Parameters / State

One constructor param per state field, in field order:

| Field | Type | Meaning |
|---|---|---|
| `provider` | `pubkey` | Committed provider key. The only key that may `charge`. |
| `subscriber` | `pubkey` | Committed subscriber key (the funding party). |
| `amount_per_period` | `int` | Fixed per-period fee (bounded non-negative). |
| `period` | `int` | Minimum buckets between charges (the rate limit). |
| `last_charged` | `int` | Coarse time bucket of the last accepted charge. |
| `balance` | `int` | Running prepaid balance, drawn down per charge. |

## Lifecycle

```
live --charge(providerSig, now_bucket)  [now_bucket >= last_charged + period, balance >= amount_per_period] --> live  (last_charged := now_bucket; balance -= amount_per_period)
```

## Why it's safe by shape

- **Committed-key authorisation (C2).** `charge` `checkSig`s against the committed
  `provider` key, never a caller-supplied pubkey. The `authorized` invariant makes
  the no-auth fail-safe a stated, enforced property.
- **Rate limit (`temporal_guard`).** `charge` is gated on `now_bucket >=
  last_charged + period`; the `temporal_guard` invariant makes the cadence gate an
  enforced property — a future edit that drops it fails the checker.
- **Value conservation.** `balance` is a value-bearing `int` decreased by a single
  additive subtraction (`balance - amount_per_period`) — the only mutation the
  hardened `value_conserved` invariant permits for a non-mint/burn entrypoint. A
  charge that would overdraw is rejected.

## Honest scope

- **`now_bucket` is caller-asserted and coarse**, exactly as in TimeVault, Escrow
  and DeadMansSwitch. The covenant does NOT read wall-clock time; the consensus
  relative-timelock is enforced by the engine's relative-time rule on the spending
  input's sequence, and the covenant complements it with the bucket bound.
- **State, not coin movement.** `amount_per_period` is a committed integer
  conserved against `balance`; the actual coin movement (paying the provider,
  refunding residual to the subscriber) is the wallet's responsibility.
- **Semantic checks are structural/relational, not an SMT solver** (per-field, no
  cross-field flow proof).
- Pre-production, unaudited, testnet-only.

## Files

- `Subscription.portrait` — the canonical covenant source. `portrait engrave`
  lowers it to `.sil` + CTOR JSON that `silverc` accepts (exit 0).
- `Subscription.sil` — the emitted Silverscript component.
- `Subscription_ctor.json` — the emitted CTOR JSON consumed by `silverc --ctor`.
- `Subscription.json` — the `silverc`-compiled script.

## Reproduce

```sh
cd portrait
cargo run --bin portrait -- check   ../library/finance/subscription/Subscription.portrait
cargo run --bin portrait -- engrave ../library/finance/subscription/Subscription.portrait
```
