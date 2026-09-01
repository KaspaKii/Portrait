# Example covenants

This page documents the example `.portrait` sources shipped in the Portrait
compiler repo (`portrait/examples/`). They are the compiler's own demos —
minimal, single-UTXO covenants used to exercise the build pipeline
(`pounce` → `engrave` → `silverc`) and, for the tier-3 demo, the vProg path
(`atelier-build`).

**Maturity note:** Pre-production, unaudited, testnet-only, perishable evidence.
Covenant type-checks here are structural/relational (no SMT). The
declared invariant in every example is `no_undeclared_state` — a structural
check that the entrypoint touches no state outside its `state { }` block. The
state transitions shown are the actual return-expression field updates from the
source; none of these examples carries an inline `require(...)` guard (each
source comment notes that value guards belong to a future covenant `require()`
revision). The tier-3 demo's vProg side is EMIT-VERIFIED only: the covenant
engraves and the guest emits the 104-byte CSCI journal, but the compliance
predicate is the developer-authored body (`balance - amount`), not an SMT-proven
rule. The `ComplianceToken` here is a worked *example* and is **not** settled
live — distinct from the five `library/vprog/` patterns, all of which **have**
been settled live on TN10 (see [Cross-layer (vProg) patterns](./vprog.md)).
`CsciInstrument` (in the `state` family) is also settled live.

Each example below emits a single `.sil`. (No example in this directory emits
more than one `.sil`.)

## Counter (`counter.portrait`)

**Purpose.** The M0 build target — `portrait build counter.portrait` must emit a
`Counter.sil` that passes `silverscript check`.

**State.**
- `value: int`

**Transitions.**
- `counter.bump(int delta) : (int value)` — returns `value + delta`. No guard
  (`require`) in source. Lifecycle: `live -> live via counter.bump`.

**Invariants.**
- `no_undeclared_state`

**Honest scope.** M0 structural demo; no value guard, no SMT — the entrypoint
just adds a delta to a single state field.

## SimpleToken (`engraver-demo/SimpleToken.portrait`)

**Purpose.** Green-tier ERC20 subset (a single `balance` with a `transfer`),
mapped onto a Kaspa UTXO covenant (no global shared state, no mappings).

**State.**
- `balance: int`

**Transitions.**
- `token.transfer(int amount) : (int balance)` — returns `balance - amount`. No
  guard in source. Lifecycle: `live -> live via token.transfer`.

**Invariants.**
- `no_undeclared_state`

**Honest scope.** Subtraction-only balance update; no overflow/underflow or
authorization guard — those are deferred to a future `require()`.

## PausableToken (`engraver-demo/PausableToken.portrait`)

**Purpose.** Green-tier ERC20+Pausable subset. `paused` is a covenant state
field; the source notes the pause gate check lives off-chain or in a future
covenant `require()` guard — the state transitions themselves are
covenant-enforced.

**State.**
- `balance: int`
- `paused: int`

**Transitions.**
- `token.transfer(int amount) : (int balance)` — returns `balance - amount`. No
  guard in source (no `whenNotPaused` enforcement on-chain yet). Lifecycle:
  `live -> live via token.transfer`.
- `token.set_paused(int flag) : (int paused)` — returns `paused - paused + flag`
  (i.e. sets `paused = flag`). No guard in source (no `onlyOwner` enforcement on
  chain yet). Lifecycle: `live -> live via token.set_paused`.

**Invariants.**
- `no_undeclared_state`

**Honest scope.** `paused` is tracked as state but not yet enforced as a guard on
`transfer`; the `whenNotPaused` / `onlyOwner` modifiers are documentation, not
covenant guards in this revision.

## VestingWallet (`engraver-demo/VestingWallet.portrait`)

**Purpose.** Green-tier time-locked token release — tracks `released` (amount
released so far) in single-UTXO local state, no global shared mappings.

**State.**
- `released: int`

**Transitions.**
- `wallet.release(int amount) : (int released)` — returns `released + amount`. No
  guard in source (the release-amount/time guard belongs to a future covenant
  `require()`). Lifecycle: `live -> live via wallet.release`.

**Invariants.**
- `no_undeclared_state`

**Honest scope.** Accumulates released amount only; no vesting-schedule or
time-lock guard is enforced on-chain in this revision.

## ComplianceToken (`tier3-demo/ComplianceToken.portrait`)

**Purpose.** Tier-3 demo emitting both a covenant (L1) and a vProg (off-L1) from
one source. `transfer` (with `#[covenant(mode = transition)]`) lowers to a
silverscript covenant; `verify_compliance` (no `#[covenant]` attribute) lowers to
a NonCovenant / VProg transition that Atelier emits as a RISC Zero guest
(`compliancetoken_guest_main.rs`) and Engraver ignores.

**State.**
- `balance: int`

**Transitions.**
- `token.transfer(int amount) : (int balance)` — **covenant (L1)**. Returns
  `balance - amount`. No `require` guard in source. Lifecycle:
  `live -> live via token.transfer`.
- `token.verify_compliance(int amount)` — **vProg (off-L1)**, not part of the
  lifecycle. Body returns `balance - amount`; in the generated guest this lowers
  to `let new_balance = balance - amount;` so `new_state_hash =
  sha256(new_balance)`. The guest commits a 104-byte journal:
  `covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]`, where
  `rule_hash = sha256("verify_compliance")`.

**Invariants.**
- `no_undeclared_state`

**Honest scope.** EMIT-VERIFIED only on the vProg side: the covenant engraves and
the guest emits the CSCI journal, but `verify_compliance` is the
developer-authored body (a plain subtraction), not an SMT-proven compliance
predicate. The cross-layer binding (covenant ID → STARK → `OpZkPrecompile` tag
`0x21`) is the documented design, not a live-settled path in this example.

## PersonalVault (`app-composition/time-vault.portrait`)

**Purpose.** App-composition demo: instantiates the library covenant
`custody::TimeVault` inside an app, supplying deployment params and declaring the
app's legal lifecycle. The Composer binds the `.sil`, generates the
covenant-ID genesis tx and per-entrypoint templates, and verifies the declared
lifecycle against the transitions in `TimeVault.sil`.

**State.** No inline state — state is defined by the library covenant
`custody::TimeVault`, not in this app source. (This page documents only what the
example source declares; the TimeVault state/guards live in the library, not
in `examples/`.)

**Contract instantiation.**
- `contract vault = TimeVault { owner = param pubkey, recovery = param pubkey,
  delay = 1440 }` — `owner` and `recovery` supplied at deployment; `delay` is
  1440 relative-age units.

**Transitions (declared lifecycle).** The transitions are TimeVault entrypoints;
this app declares the legal lifecycle and `portrait build` fails if any covenant
entrypoint could drive the UTXO into an unlisted state:
- `idle -> pending via vault.schedule`
- `pending -> settled via vault.settle` (terminal — funds leave the covenant)
- `pending -> idle via vault.cancel`

**Invariants.** None declared in this app source (no `invariant` line); the
guards/invariants belong to the `custody::TimeVault` library covenant.

**Honest scope.** This is a composition/wiring example: the actual guards
(`require(...)`) and invariants live in the `custody::TimeVault` library
covenant, not in this file. The file declares only the contract params and the
legal lifecycle that the Composer checks against the bound `.sil`.
