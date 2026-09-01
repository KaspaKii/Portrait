# finance/Subscription

A recurring, rate-limited pull-payment covenant. A prepaid subscription UTXO lets
a committed `provider` pull a fixed `amount_per_period` from a `subscriber`-funded
`balance`, but no more than once per `period`. The on-chain recurring-billing /
standing-order pattern: the merchant may charge on a cadence, the customer is
protected from being charged faster than the agreed rate, and the running balance
is drawn down one charge at a time until exhausted.

**Status:** 🟡 drafted — pre-red-team, testnet-only, not audited, not mainnet-safe.

## Parameters / State

One constructor param per state field, matched by name (a state field with no
same-named param is a compile error):

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
- **Payout binding (B2).** `pays(1, provider, amount_per_period)` makes CONSENSUS
  bind the fee: the emitted covenant carries
  `require(tx.outputs[1].value == prev_states[0].amount_per_period)` and
  `require(tx.outputs[1].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_states[0].provider)))`,
  so a `charge` that does not actually pay the committed fee to the committed
  provider is rejected. `amount_per_period` is an `int`, not a `coin` and not a
  value-bearing NAME; it qualifies as a bound amount because this same entrypoint
  **draws it down** — the successor sets `balance: balance - amount_per_period`
  under `requires amount_per_period >= 0;`, and that structural link is what proves
  the paid quantity is the quantity the model gives up. It was deliberately NOT
  renamed to `amount` (that would buy the guarantee off a name) and NOT retyped to
  `coin` (which cannot express a drawdown at all — the type checker forbids
  arithmetic on `coin`).

## Honest scope

- **⚠ NON-TERMINAL `pays` — DO NOT DEPLOY YET (KI-3).** `charge` is the catalogue's
  first `pays` on a spend that ALSO produces a covenant successor. silverc's `to`
  counts covenant SUCCESSOR outputs, not total transaction outputs, so `to = 1`
  plus a separate payee output is well-formed at compile time (silverc exit 0) —
  but **which output index the successor occupies at RUNTIME is UNVERIFIED on the
  `v2.0.0` engine pin** (the same composed-on-engine-spend bucket as KI-1). The
  binding assumes the successor takes index 0 and pays the provider at index 1; if
  that is wrong, every `charge` is rejected and the prepaid balance is stuck. See
  `KNOWN-ISSUES.md` KI-3. Do NOT deploy this to a value-bearing UTXO until that
  composed spend is proven.
- **`now_bucket` is caller-asserted and coarse**, exactly as in TimeVault, Escrow
  and DeadMansSwitch. The covenant does NOT read wall-clock time; the consensus
  relative-timelock is enforced by the engine's relative-time rule on the spending
  input's sequence, and the covenant complements it with the bucket bound.
- **The bound output is output[1] only.** The fee leg is consensus-bound, but the
  binding constrains no OTHER output — there is no value-conservation /
  transaction-mass (KIP-9) check, so the residual routing (funding the successor,
  refunding any surplus to the subscriber) is still the wallet's responsibility,
  and the committed `amount_per_period` is not tied to the coin actually deposited
  in the covenant UTXO.
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
