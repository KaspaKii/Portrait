# Portrait Pattern Catalogue

> **Maturity: pre-production, unaudited, testnet-only.** No external security
> audit or external review has taken place — internal adversarial hardening is
> not external review. Nothing is on mainnet; live evidence is perishable Kaspa
> testnet-10 (TN10) evidence (the testnet resets).

> **Status note (2026-07):** this is the design taxonomy. The shipped
> covenant-patterns library now stands at **35 covenant sources / 10 cross-layer
> (vProg) patterns**, of which **5 are settled live on TN10** (perishable
> testnet evidence) and 5 are emit-verified only. The per-pattern status keys
> below may lag the shipped library.

The reusable building blocks, organised the way the established Solidity pattern libraries organise their contracts — but mapped onto Kaspa's UTXO/covenant model. Each pattern, when stable, ships as: a Silverscript component (`<Name>.sil`), a Portrait wrapper (`<name>.portrait`), golden tests, a `THREAT_MODEL.md`, and a README.

**Status key:** 🟢 implemented & tested · 🟡 drafted (code, pre-review) · ⚪ planned

> Kaspa-specific reality checks baked into every entry: state is **local to a UTXO** (no global state, reentrancy-free by construction); loops are **compile-time unrolled** (`MAX_ITERATIONS`), so anything "iterate over all holders" must be redesigned as per-UTXO or batched; persistence across transitions rides **KIP-20 covenant IDs**, not storage slots.

---

## access/ — authorization & control
Who is allowed to move the covenant forward.

| Pattern | Intent | Status |
|---|---|---|
| `SingleKey` | One owner key authorises every transition. The `Ownable` baseline. | ⚪ |
| `DualKey` | Warm key for normal ops + cold recovery key for clawback/cancel. | ⚪ |
| `MultiSig` | k-of-n approval over a transition; uses bounded `for` over `sig[]`. | ⚪ |
| `TimelockedAdmin` | Privileged actions only after a relative-age delay (`this.age`). | ⚪ |
| `RoleGuard` | Distinct keys gate distinct entrypoints (proposer / executor / canceller). | ⚪ |

## custody/ — holding & releasing value
Funds that sit under enforced spending rules.

| Pattern | Intent | Status |
|---|---|---|
| `TimeVault` | Two-key, time-delayed withdrawal vault with cold-key clawback. | 🟡 |
| `Escrow` | Two parties + optional arbiter; release / refund / dispute paths. | ⚪ |
| `HTLC` | Hashed-timelock contract — the leg of an atomic swap. | ⚪ |
| `StreamingPayment` | Fixed amount claimable per elapsed period (Mecenas-style). | ⚪ |
| `Clawback` | Wrap any custody pattern with a recovery-key reversal window. | ⚪ |

## exchange/ — trading & price discovery
Moving value between parties under agreed rules.

| Pattern | Intent | Status |
|---|---|---|
| `AtomicSwap` | Trustless cross-party swap of two UTXOs (HTLC pair). | ⚪ |
| `DutchAuction` | Descending-price sale; first valid bid settles. | ⚪ |
| `EnglishAuction` | Ascending bids, highest at timeout wins, losers refunded. | ⚪ |
| `BatchAuction` | Net-settled frequent batch auction at block cadence. | ⚪ |
| `AMMPool` | Constant-product pool as local UTXO state; swap/add/remove. | ⚪ |

## state/ — machines, registries, persistence
The patterns that make "an app that remembers" possible.

| Pattern | Intent | Status |
|---|---|---|
| `StateMachine` | Generic declared-transition guard; rejects undeclared transitions. | ⚪ |
| `Counter` | Minimal transition covenant — the "hello world" of persistence. | ⚪ |
| `Registry` | Append/update a key→value map across covenant-ID lineage. | ⚪ |
| `Scoreboard` | Persistent, updatable standings. | ⚪ |
| `Allowlist` | Membership set gating who may interact. | ⚪ |

## commit/ — commitments & script trees

| Pattern | Intent | Status |
|---|---|---|
| `HashCommitment` | Commit-then-reveal a preimage (`blake2b(preimage) == commit`). | ⚪ |
| `MerkleBranch` | Reveal one branch of a committed script/state tree — MAST-style. | ⚪ |
| `CommitRevealBeacon` | Two-phase randomness/decision beacon. | ⚪ |

## token/ — native assets (Toccata)
Standards on top of native assets / KRC20-style issuance.

| Pattern | Intent | Status |
|---|---|---|
| `TrustedToken` | The trusted-token standard. | ⚪ |
| `Vesting` | Time-released allocation to a beneficiary. | ⚪ |
| `PaymentSplitter` | Split an inflow across fixed shares (e.g. rent waterfall legs). | ⚪ |
| `Faucet` | Rate-limited dispenser with per-period cap. | ⚪ |

## examples/ — full multi-contract apps
Not primitives — end-to-end demonstrations of the Composer.

| App | Composes | Status |
|---|---|---|
| `ChessLeague` | per-game `Game` UTXO + persistent `Scoreboard` covenant | ⚪ |
| `DigitalReit` | `TrustedToken` + `PaymentSplitter` rent waterfall | ⚪ |
| `VaultWithGovernance` | `TimeVault` + `MultiSig` + `TimelockedAdmin` | ⚪ |

---

## Build order

1. **state/`Counter` + `StateMachine`** — proves the transition + lifecycle-checking core end-to-end against the debugger. Everything else leans on these.
2. **custody/`TimeVault`** *(drafted)* + **access/`DualKey`, `MultiSig`**.
3. **commit/`HashCommitment` + `MerkleBranch`** — settles the MAST question concretely.
4. **exchange/`HTLC` → `AtomicSwap` → `DutchAuction`**.
5. **examples/`ChessLeague`** — the first true multi-contract app.

Each step is gated by `CONTRIBUTING.md`: no pattern is marked 🟢 until its threat model's CRITICAL/HIGH findings are resolved or explicitly flagged.
