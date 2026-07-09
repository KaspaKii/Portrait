# Migrating from Solidity to Kaspa

*The on-ramp for Ethereum developers. You keep your mental model; this guide shows you the Kaspa equivalent of each pattern you already know — and, just as importantly, where Kaspa is **not** Ethereum, so you move correctly rather than just quickly.*

> **Status.** This is the conceptual migration guide for the Kii Rosetta on-ramp (under active development). The pattern mappings and the differences below are stable and correct today; the exact `kcp` CLI/SDK surface is stabilising — treat code shapes as the intended ergonomics, and check each command against the toolkit version you have.

---

## 1. The one thing to understand first

You are not "deploying a contract to an address." On Kaspa there are no accounts and no contract storage slots. Value lives in **UTXOs** (unspent transaction outputs), and a **covenant** is a set of spend rules attached to a UTXO that the network enforces when someone tries to spend it. Your "contract logic" becomes **conditions on how value is allowed to move**, not a stateful object you call methods on.

This is the single mental shift. Everything below follows from it. The good news: most of what the established Solidity pattern libraries give you — tokens, ownership, timelocks, vaults, vesting, escrow, attestations — maps cleanly onto a covenant pattern. The patterns are already built (`kcp-*`). What changes is the *shape* of how state and authority work, and there are a few sharp edges where the Ethereum reflex will bite you.

Two layers, briefly: this guide covers the **covenant layer** (Kaspa L1 — UTXO-local rules, value, lifecycle, attestation). Apps that need **shared mutable state across many users** (an AMM pool, a global registry everyone writes to) belong to the **vProgs** layer, which settles back to L1 by ZK proof. That's the unifying language, and it's a later tier — see §7.

---

## 2. The mapping at a glance

| You know (Solidity pattern) | Kaspa equivalent | Familiar API | Underlying `kcp-*` crate |
|---|---|---|---|
| `ERC20` | **KTT** (Kaspa Trust Token) | `kii-solidity-compat::erc20::Token` | `kcp-ktt-token` |
| `Ownable` | single-key UTXO ownership | `kii-solidity-compat::ownable::OwnershipRecord` | `kcp-common::access` |
| `AccessControl` | covenant-gated roles | `kcp-common::access` | `kcp-governance` |
| `TimelockController` (action queue) | queued action with delay | `kii-solidity-compat::timelock::TimelockController` | `kcp-governance` |
| `VestingWallet` (linear release) | linear DAA-height vesting | `kcp-vesting::VestingSchedule` | `kcp-vesting` |
| `ERC4626` / `Escrow` | covenant-custodied vault | `kii-solidity-compat::vault::Vault` | `kcp-vault` |
| attestation / registry / SBT-ish | sealed lineage + paired attestation | — | `kcp-sealed-lineage`, `kcp-paired-attestation` |
| RWA / transferable record | UTXO-bound transferable record | — | `kcp-transferable-record` |
| *(no Ethereum analog)* | post-quantum signature ZK anchor | — | `kcp-pq-anchor` |

The `kii-solidity-compat` crate is the **Rosetta layer** — Solidity-shaped method names over the `kcp-*` primitives so you can read and reason about Kaspa covenants in terms you already know. Each row below: *what you did in Solidity → what you do on Kaspa → what's different and what can bite you.*

---

## 3. Token: `ERC20` → KTT

**Solidity:** a contract holds a `mapping(address => uint256) balances`, and `transfer` mutates two entries. Supply is a number in storage.

**Kaspa:** a **KTT** is value represented by covenant-controlled UTXOs. Issuance, transfer rules, and supply constraints are enforced by the covenant whenever the token UTXOs are spent. There is no global balances mapping — a holder's balance is the sum of the KTT UTXOs they can spend, exactly like native coin balances are the sum of your UTXOs.

```text
# intended Rosetta ergonomics (coming in Week 1 — current: kcp scaffold ktt-token)
kcp new --from-solidity erc20  MyToken --symbol MYT --supply 1_000_000
# → scaffolds a kcp-ktt-token covenant, runs `silverscript check`,
#   deploys to TN10, and emits a Hallmark manifest (a provenance artifact)
```

**What's different / what can bite you:**
- **No `approve` / `transferFrom` allowance object.** The allowance pattern is an account-model construct. On UTXO you authorise spends with covenant conditions (e.g. a delegated spend rule), not a stored allowance integer. If your Solidity design leans on `approve`, that logic must be re-expressed as a spend condition — it does not port 1:1.
- **Supply is covenant-enforced, not a settable variable.** "Mint more later" must be a rule you build into the covenant (an authorised issuance path), not an owner writing to a storage slot. If you didn't design a mint path in, there isn't one — by design.
- **It is the trusted token standard, KTT — never "KRC-20T".** (Naming matters for provenance.)

---

## 4. Ownership & roles: `Ownable` / `AccessControl` → `OwnershipRecord` / `kcp-common::access`

**Solidity:** `onlyOwner` checks `msg.sender == owner`; `AccessControl` checks a role mapping. Owner is a mutable storage slot you can transfer.

**Kaspa:** authority is a **spend condition**, not an identity check against `msg.sender` (there is no `msg.sender`). "Only the owner may do X" becomes "this UTXO may only be spent if the spend is authorised by key/condition K," enforced by the covenant.

The **Rosetta facade** (`kii-solidity-compat::ownable`) gives you familiar method names:

```rust
use kii_solidity_compat::OwnershipRecord;

// Ownable(address initialOwner)
let rec = OwnershipRecord::new(owner_key);

// onlyOwner
rec.verify_owner(signing_key)?;

// transferOwnership(newOwner)
let rec = rec.transfer_ownership(new_owner_key);

// renounceOwnership()
let rec = rec.renounce_ownership();
```

For k-of-n role-based control (`AccessControl`) and full governance cycles (proposal → vote → timelock → execute), use `kcp-common::access` + `kcp-governance`. Scaffold via `kcp new --from-solidity ownable`.

**What's different / what can bite you:**
- **No `msg.sender`.** Authority is proven by satisfying a covenant condition (a signature, a key, a covenant-ID lineage), not by the address that "called" — there is no caller.
- **No silent owner rescue.** On Ethereum an owner can often reach in and fix/rescue. A covenant only permits what it was written to permit. If you want an admin escape hatch, it must be an explicit, visible spend path in the covenant — and for a compliance-grade instrument, an escape hatch is a liability you must justify, not a default.
- **Role changes are state transitions**, expressed via covenant lineage (KIP-20 covenant IDs), not a write to a role mapping.

---

## 5. Timelocks and vesting: `TimelockController` → `TimelockController`; `VestingWallet` → `kcp-vesting`

**Solidity:** `TimelockController` queues an operation and enforces a delay against `block.timestamp`. `VestingWallet` releases a beneficiary's balance linearly over time.

**Kaspa:** The **Rosetta facade** (`kii-solidity-compat::timelock`) gives you familiar method names over `kcp-governance::TimelockAction`:

```rust
use kii_solidity_compat::TimelockController;

// TimelockController(minDelay, proposers, executors)
// min_delay_daa ≈ seconds at 1 BPS
let mut ctrl = TimelockController::new(86_400, proposer_key, executor_key)?;

// schedule(target, …, delay)
ctrl.schedule(proposer_key, current_daa_height)?;

// isOperationReady(id)
if ctrl.is_ready(current_daa_height) { /* … */ }

// execute(target, …)
ctrl.execute(executor_key, current_daa_height)?;

// cancel(id)
ctrl.cancel();
```

For linear release to a beneficiary, use `kcp-vesting::VestingSchedule` directly — no Rosetta wrapper needed for its already-simple API. Scaffold via `kcp new --from-solidity timelock`.

**What's different / what can bite you:**
- **DAA heights, not `block.timestamp`.** At 1 BPS the clock is ≈ 1 second per height, but it is the DAG's difficulty-adjusted score, not wall-clock time. Obtain it from a kaspad node; do not synthesise it.
- **No keeper/executor needed to "release."** There's no queued operation an executor must trigger; the spend simply becomes valid once the age condition is met.

---

## 6. Vaults, escrow, vesting, records

**`ERC4626` / `Escrow` → `kii-solidity-compat::vault::Vault` + `kcp-vault`.** The **Rosetta facade** gives you familiar deposit/evaluate ergonomics:

```rust
use kii_solidity_compat::Vault;
use kcp_vault::{condition::SpendCondition, evaluator::EvalContext};

// "Deploy" a vault with a spend condition.
let vault = Vault::new(SpendCondition::TimelockHeight {
    deadline: 2_000_000,
    controller_xonly: controller_key,
})?;

// deposit(assets, receiver) → returns a VaultDescriptor (the UTXO to create)
let descriptor = vault.deposit(500_000_000)?;

// Check if the vault can be spent.
let ctx = EvalContext { daa_score: 2_000_000, unix_seconds: 0, signers_present: vec![controller_key] };
if vault.evaluate(&ctx) { /* spend is authorised by consensus */ }
```

Scaffold via `kcp new --from-solidity vault`. *Bite:* `deposit()` does not move funds — it returns a `VaultDescriptor` you must include as a UTXO in a transaction. Broadcasting that UTXO creates the vault lock on-chain.

**`kcp-yield-vault` (ERC4626 tokenised vault).** Deposits are custodied in covenant-controlled UTXOs; share/redemption accounting is covenant-enforced rather than tracked in a storage mapping. *Bite:* "shares" are not a freely-mutable balance map; redemptions follow the covenant's accounting path.

**Attestation / registry → `kcp-sealed-lineage` + `kcp-paired-attestation`.** Identity, provenance, and attestations are expressed as covenant lineage (KIP-20 covenant IDs) — a converging on-chain history, not rows in a storage map. *Bite:* there's no mutable registry mapping to overwrite; you append/transition lineage.

**RWA / transferable record → `kcp-transferable-record`.** A record bound to a UTXO whose transfer is covenant-gated (e.g. compliance conditions must hold). *Bite:* transfer restrictions are enforced at the protocol level on spend, not by a `_beforeTokenTransfer` hook you can later remove.

**`kcp-pq-anchor`** has no Ethereum analog: it anchors a post-quantum signature, verified on L1 via a succinct ZK proof. It's a Kaspa-native capability worth knowing exists — and it's the settlement primitive the higher tiers build on.

---

## 7. The sharp edges (read this section twice)

These are the places where the Ethereum reflex is wrong on Kaspa. Getting these right is the difference between "moved" and "moved correctly."

1. **No accounts, no balances mapping.** State is carried in UTXOs and covenant conditions, transitioned by spending. Stop thinking "what's in the contract's storage"; start thinking "what UTXOs exist and what rules govern spending them."
2. **No shared mutable global state at this layer.** Two users cannot both mutate one shared object in one covenant the way two addresses write to one mapping. If your design needs a global, concurrently-written state (order book, pool, leaderboard), that is a **vProg**, not a covenant — don't force it into a covenant, route it to the execution layer (a later tier).
3. **Covenant-final means final.** There is no proxy upgrade, no admin pause, no owner rescue unless you explicitly designed that spend path in. This is a feature (no rug vector) and a footgun (no fixing a deployed mistake). Design the lifecycle — including any intended upgrade/rescue path via KIP-20 lineage — *before* you deploy value.
4. **Gasless, but not free.** No gas means no gas-griefing and no out-of-gas mid-execution — but transactions carry **mass/fees** (post-Toccata the fee schedule and `storage_mass` semantics apply — `[FACT-NEEDED: re-verify exact fee schedule and mass terminology against live TN10 before citing]`). Size and structure your covenants with mass in mind.
5. **Reentrancy doesn't exist the way it does in Solidity** — there are no external calls mutating shared state mid-execution — so the classic reentrancy footgun is gone. Do **not** read that as "no concurrency concerns": UTXO contention (two spends racing for the same output) is the model's own consideration, resolved by consensus, and your UX must handle a spend that loses the race.
6. **The allowance/`approve` pattern doesn't port.** Re-express delegated spending as a covenant condition; don't assume `approve`+`transferFrom`.
7. **No `block.timestamp`-style absolute time by default.** Prefer relative age; build absolute-time constructions deliberately if you truly need them.

---

## 8. What this on-ramp is — and isn't

**Is:** a way to ship the **L1-native, compliance / RWA / industrial** patterns you'd reach to a Solidity pattern library for, made familiar to an Ethereum dev, with the asset never leaving Kaspa's security — and a re-derivable, proof-carrying build manifest (a Hallmark) on the result.

**Isn't:**
- **Not full Solidity / EVM equivalence.** If you want to run arbitrary Solidity, that's an EVM-on-Kaspa L2 — a different trust model and a different goal. This on-ramp is deliberately the covenant-native compliance path, not general computation.
- **Not the shared-state app layer yet.** Concurrent global state lives in **vProgs**, which settle to L1 by ZK proof. A unifying language that lets one program target both covenants and vProgs is future work.

---

## 9. Provenance & the Hallmark manifest

Every pattern here is built to carry **verifiable provenance**: build through the toolkit and you can emit a **Hallmark** — a re-derivable, proof-carrying build manifest that ties a covenant's source, its model checks, and its compiled script bytes together, so a third party can reproduce them. The library is free and open-source; the Foundation does not certify, attest to, or guarantee any covenant, and the Hallmark is a provenance artifact, not a certification. Scope is intentionally conservative: the strongest reproducibility guarantees apply only to fully-verifiable pattern classes.

---

## 10. Your first migration

The `kcp new --from-solidity` wizard scaffolds a Solidity-pattern-shaped Rust project for each pattern. Each generated project includes a binary, smoke tests, and a README with the method-map table.

```sh
# ERC20 → KTT (Kaspa Trust Token)
kcp new --from-solidity erc20 --name HelloKTT --symbol HKT --supply 1000000 \
    --workspace-path /path/to/kaspa-compliance-patterns

# Ownable → OwnershipRecord
kcp new --from-solidity ownable \
    --workspace-path /path/to/kaspa-compliance-patterns

# TimelockController → kcp-governance TimelockAction
kcp new --from-solidity timelock --min-delay-daa 86400 \
    --workspace-path /path/to/kaspa-compliance-patterns

# ERC4626 / Escrow → kcp-vault Vault
kcp new --from-solidity vault --deadline-daa 2000000 \
    --workspace-path /path/to/kaspa-compliance-patterns
```

Each command:
1. Generates a ready-to-compile Rust project using the `kii-solidity-compat` facade.
2. Includes smoke tests you can run with `cargo test`.
3. Carries the pre-production maturity stamp — replace synthetic keys with real ones before live use.

Start with `erc20` if you're coming from an ERC20 token; start with `vault` if you're coming from an escrow or custody contract. Read the generated README for the exact method-map table before you ship.

When that round-trips — familiar input, a working KTT on testnet, a txid you can click — you've done the thing this whole on-ramp exists to make trivial: moved from Ethereum to Kaspa, correctly, in an afternoon.

---

*Build target is TN10 (which already runs the full Toccata bundle); mainnet covenant deploys move there after Toccata activates on 30 June 2026. Re-verify any deploy against the live toolchain. Token standard is KTT.*
