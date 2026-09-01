# Custody covenants

The custody family covers single- and two-key vault patterns: a per-transaction
spending cap, a time-gated release with a break-glass clawback, and an
inactivity-triggered inheritance switch. Each is a singleton covenant authored in
Portrait (`pragma portrait ^0.1.0`) and lowered by `portrait engrave` to a
`silverc`-accepted covenant.

> **Maturity / honest scope.** Pre-production, unaudited, testnet-only,
> perishable. Covenant type-checks are structural/relational (no SMT). The
> `value_conserved` check is N-field additive-delta cancellation, **not** an SMT
> proof. Temporal gates use a **caller-asserted coarse time bucket** (`now_bucket`),
> not a wall-clock proof — the real relative-timelock is enforced by the engine's
> sequence rule, which the covenant only complements. No covenant in this family
> reads UTXO coin values or wall-clock time itself.

Source: `library/custody/` in the portrait repo. None of the three covenants in
this family is a vProg pattern (no `_guest_main.rs`), and each emits a single
`.sil`.

## SpendingLimitVault

Source: `library/custody/spending-limit/SpendingLimitVault.portrait`

**Purpose.** A single-owner custody vault whose `withdraw` may only move an
`amount` at or below a committed per-transaction `limit`. Committed-key
authorisation composed with a committed-state bound on each withdrawal.

**State** (constructor params are declared in field order):

- `owner: pubkey` — committed owner key (the withdraw authority)
- `balance: int` — vault balance (value-conserved by field name)
- `limit: int` — committed per-transaction spending cap

**Transitions.**

- `withdraw(sig auth, int amount)` — guards:
  - `requires checkSig(auth, owner);` — committed-owner authorisation (C2)
  - `requires amount >= 0;` — non-negative withdrawal
  - `requires amount <= limit;` — per-transaction spending cap
  - `requires amount <= balance;` — cannot overspend the vault balance
  - field updates: `owner: owner` (carried), `balance: balance - amount` (single
    additive subtraction), `limit: limit` (carried)

**Invariants.** `value_conserved`, `authorized`, `non_negative_amount`,
`spending_cap`, `no_undeclared_state`.

**Honest scope.** Per-transaction cap, **not** a time-windowed rate limit; an
owner can issue many sequential withdrawals. `amount` is caller-asserted; the
covenant does not read UTXO coin values. Emits one `.sil`.

## TimeVault

Source: `library/custody/time-vault/TimeVault.portrait`

**Purpose.** A two-key, time-gated custody vault. The `owner` may `release` once
the temporal gate has opened; a cold `recovery` key may `claw` the funds back at
any time before release (break-glass). `released` is a one-shot flag so the vault
cannot be double-spent across the two paths.

**State** (constructor params in field order):

- `owner: pubkey` — hot key permitted to release after the gate
- `recovery: pubkey` — cold clawback key (break-glass)
- `unlock_bucket: int` — coarse time bucket at/after which release is allowed
- `released: int` — one-shot spent flag (genesis = 0)

**Transitions.**

- `release(sig auth, int now_bucket)` — guards:
  - `requires checkSig(auth, owner);` — only the owner may release
  - `requires now_bucket >= unlock_bucket;` — temporal gate has opened
  - `requires released == 0;` — one-shot: not already spent
  - field updates: `owner`, `recovery`, `unlock_bucket` carried; `released: 1`
- `claw(sig auth)` — guards:
  - `requires checkSig(auth, recovery);` — only the recovery key may claw
  - `requires released == 0;` — one-shot: not already spent
  - field updates: `owner`, `recovery`, `unlock_bucket` carried; `released: 1`

**Invariants.** `value_conserved`, `no_undeclared_state`.

**Honest scope.** The temporal gate is a caller-asserted coarse `now_bucket`
compared against committed `unlock_bucket`; consensus relative-timelock is the
engine's job. The covenant does not read wall-clock time. Emits one `.sil`.

## DeadMansSwitch

Source: `library/custody/dead-mans-switch/DeadMansSwitch.portrait`

**Purpose.** An inactivity-triggered inheritance covenant. The `owner` keeps the
UTXO alive with a periodically signed `heartbeat`; if the owner goes silent for
longer than the committed `timeout`, the `heir` may `claim` control. Both
transitions authorise against a COMMITTED state key, never a caller-supplied one.

**State** (constructor params in field order):

- `owner: pubkey` — committed owner key (liveness / heartbeat authority)
- `heir: pubkey` — committed heir key (inheritance / claim authority)
- `last_active: int` — coarse time bucket of the last proven heartbeat
- `timeout: int` — inactivity window (buckets) after which heir may claim

**Transitions.**

- `heartbeat(sig auth, int now_bucket)` — guards:
  - `requires checkSig(auth, owner);` — only the committed owner may heartbeat
  - field updates: `owner` and `heir` carried unchanged; `last_active: now_bucket`
    (liveness refreshed); `timeout` carried. (Reads `now_bucket` only in the
    return, not in a guard — so it is not a time gate.)
- `claim(sig auth, int now_bucket)` — guards:
  - `requires checkSig(auth, heir);` — only the committed heir may claim
  - `requires now_bucket >= last_active + timeout;` — inactivity window elapsed
  - field updates: `owner: heir` (control transferred to the heir), `heir: heir`
    (carried), `last_active: now_bucket` (fresh liveness window for the new
    owner), `timeout` carried

**Invariants.** `authorized`, `temporal_guard`, `no_undeclared_state`.

**Honest scope.** The inactivity gate is a caller-asserted coarse `now_bucket`
compared against committed `last_active + timeout` — a structural shape match,
not a wall-clock proof; consensus relative-timelock is enforced by the engine's
sequence rule. Emits one `.sil`.
