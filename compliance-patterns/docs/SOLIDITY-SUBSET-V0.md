# Solidity-Subset v0 — Kaspa Covenant Mapping

**Stichting Kii Foundation — kaspa-compliance-patterns**

> **Status:** Pre-production, unaudited, testnet-only. This document classifies which
> Solidity language constructs project directly onto Kaspa L1 covenants, and which
> constructs require the vProgs execution layer. Classification is bottom-up from the
> `kcp-*` algebra and verified against three real Ethereum contracts from target verticals.

---

## 1. Overview

The Solidity-subset v0 divides the Solidity API surface into three tiers:

| Tier | Label | Meaning |
|------|-------|---------|
| **Green** | Covenant-native | Maps cleanly onto Kaspa UTXO covenants + silverscript |
| **Yellow** | Partial / adapted | Possible but requires a non-trivial mapping (shape change) |
| **Red** | Rejection set | Requires the vProgs execution layer; not expressible in silverscript alone |

---

## 2. Classification Table

### 2.1 Token standards

| Solidity construct | Tier | Kaspa equivalent | Notes |
|---|---|---|---|
| `ERC20.transfer(to, amount)` | Green | `KttStateTransition::transfer` | kcp-ktt-token: 4-field state machine, KCC20-profile |
| `ERC20.approve/transferFrom` | Red | vProgs or off-chain | Requires shared allowance state; UTXO model has no mutable shared ledger |
| `ERC20.totalSupply()` | Yellow | Off-chain aggregation or vProg | No global state query in covenant model; must be tracked by minter |
| `ERC20.balanceOf(addr)` | Yellow | Off-chain UTXO query | Individual UTXO amounts are visible on-chain; sum is off-chain |
| `ERC20.mint/burn` | Green | `KttStateTransition::issue/burn` | Minter flag in KTT state enforces issuance gate |
| `ERC20._beforeTokenTransfer` hook | Red | vProgs compliance check | Arbitrary pre-transfer logic needs shared state; maps to CSCI vProg guest |
| `ERC1400` (security token) | Yellow | kcp-ktt-token + CSCI | KCC20-profile covers the base; CSCI adds compliance-rule settlement via vProg |

### 2.2 Access control

| Solidity construct | Tier | Kaspa equivalent | Notes |
|---|---|---|---|
| `Ownable.onlyOwner` | Green | `OwnershipRecord.verify_owner` + `checkSig` | kii-solidity-compat Ownable facade; maps to Schnorr sig check |
| `Ownable.transferOwnership` | Green | `OwnershipRecord.transfer_ownership` | State field update; covenant enforces auth |
| `Ownable2Step` | Green | `kcp-common::access::Ownable2Step` | Pending-owner pattern supported in Rust lib |
| `AccessControl.grantRole` | Yellow | `kcp-common::access::Multisig` | k-of-n; no fine-grained per-role inheritance; maps onto multisig auth |
| `AccessControl.hasRole` | Yellow | Script-level sig check | Per-call role check maps to Schnorr sig; role assignment is off-chain |
| `Pausable` | Green | `kcp-common::security::Pausable` | Boolean state field; covenant enforces pause gate |
| `ReentrancyGuard` | Green (implicit) | UTXO model | Structural non-reentrancy; no guard needed |

### 2.3 Time and scheduling

| Solidity construct | Tier | Kaspa equivalent | Notes |
|---|---|---|---|
| `block.timestamp` | Yellow | `this.age` (relative) or DAA height | Kaspa uses DAA heights, not unix timestamps; approximate mapping |
| `block.number` | Yellow | DAA height | ~1:1 at 1 BPS; not a block number in the Ethereum sense |
| `TimelockController.schedule/execute` | Green | `kcp-governance::TimelockAction` / `kcp-common::security::TimelockController` | DAA-height deadline; Kaspa `OP_CHECKLOCKTIMEVERIFY` enforces it |
| `block.number + delay` (scheduler) | Green | `current_daa + delay_daa` | DAA heights accumulate monotonically |
| `block.timestamp` for expiry | Yellow | `this.age >= deadline_daa` | Relative age check; absolute timestamp is not available in script |

### 2.4 Storage and state

| Solidity construct | Tier | Kaspa equivalent | Notes |
|---|---|---|---|
| `mapping(address => uint)` | Red | vProgs | Mutable shared mappings require account model; UTXO model cannot hold global mappings |
| `mapping(address => bool)` | Red | vProgs | Same as above |
| Simple value storage (`uint256 x`) | Green | Covenant state field | Single-UTXO local state; each UTXO holds its own state |
| Struct storage | Green | Multi-field covenant state | Multiple state fields per UTXO; KTT uses 4-field state |
| Array (fixed, per-UTXO) | Yellow | Silverscript arrays (fixed max) | Arrays with compile-time max work; dynamic arrays require vProgs |
| Events (`emit Transfer(...)`) | Red | L1 transaction introspection + off-chain indexer | Kaspa has no native event log; events must be derived from UTXO graph |

### 2.5 Cross-contract calls

| Solidity construct | Tier | Kaspa equivalent | Notes |
|---|---|---|---|
| `IERC20(addr).transfer(...)` | Red | vProgs (based-ZK bridging) | Synchronous cross-contract calls require account model or vProg composition |
| Interface casting | Red | vProgs | Same: shared state and synchronous calls are not UTXO-native |
| `delegatecall` | Red | Out of scope | No equivalent in UTXO model or covenant script |

### 2.6 Governance

| Solidity construct | Tier | Kaspa equivalent | Notes |
|---|---|---|---|
| `Governor.propose/vote/execute` | Yellow | `kcp-governance::GovernanceProposal` + `MultiSigVote` | Off-chain voting + on-chain timelock execution; token-weighted voting deferred |
| `ERC20Votes` (token-weighted voting) | Red | vProgs or deferred | Snapshot-based vote weight requires global token ledger; needs vProgs |
| `GovernorTimelockControl` | Green | `kcp-governance::TimelockAction` | DAA-height post-vote delay; covenant-enforced |

### 2.7 DeFi primitives

| Solidity construct | Tier | Kaspa equivalent | Notes |
|---|---|---|---|
| `ERC4626.deposit/withdraw` | Green (facade) | `kii-solidity-compat::Vault` + `kcp-vault` | Pure-offline state; TN10 live deploy of P2SH vault proof of concept |
| `ERC4626.totalAssets()` | Red | vProgs or off-chain aggregation | Global vault balance requires shared state |
| `ERC4626.convertToShares()` | Red | vProgs | Ratio computation requires global total supply |
| Uniswap AMM (x*y=k) | Red | vProgs | Requires global liquidity pool state |
| Flash loans | Red | Out of scope | Atomic cross-call loans are incompatible with UTXO model |

---

## 3. Rejection Set Summary

The following Solidity constructs **cannot be implemented in silverscript alone** and route to the vProgs layer (based-ZK execution, settlement on L1):

1. **Shared mutable mappings** — `mapping(address => ...)` of any type
2. **`approve`/`transferFrom` allowance pattern** — requires global allowance ledger
3. **Synchronous cross-contract calls** — `delegatecall`, `call`, interface casts
4. **Global aggregate queries** — `totalSupply()`, `totalAssets()`, `balanceOf(all)`
5. **Token-weighted governance** — snapshot-based vote weight (`ERC20Votes`)
6. **AMM / DeFi math on shared state** — x*y=k, oracle reads, flash loans
7. **Event logs** — Kaspa has no native event log; use UTXO graph + off-chain indexer
8. **Pre/post transfer hooks with shared state** — `_beforeTokenTransfer` compliance check
9. **Dynamic arrays / unbounded loops beyond per-UTXO scope**

**Implication:** The rejection set is not a failure — it is the **vProgs contract**. Portrait's dual-layer architecture is designed for exactly this: silverscript handles UTXO-local invariants; vProgs handles cross-UTXO shared state. The rejection set above defines the scope of the Atelier (vProgs) backend.

### 3.1 Enforcement status

This rejection set is no longer prose-only. The Portrait compiler now carries a
single-source-of-truth table — `REJECTION_SET` in
`portrait/portrait/crates/portrait-syntax/src/lib.rs` — and rejects
the blacklisted constructs **fail-loud at parse time**, naming the construct and
routing it to the vProgs layer. This replaced the prior silent degradation, where
an out-of-subset statement became a verbatim `Stmt::Raw` "untyped hole" that the
checker skipped and the emitter lowered (the worst-case failure mode: too
permissive, silent miscompile).

**Embedded-vector hardening.** The first cut of
this check ran in `parse_block` *after* the `require`/`return` keyword branches,
so a blacklisted construct embedded in a guard or result —
e.g. `require strategy.call(amount);` or `return strategy.call(amount);` — slipped
past the rejection set and degraded to `Stmt::Raw` (a FALSE ACCEPT; the `return`
case then mis-failed with the wrong "no return statement" diagnostic). The
require/return fallback paths now consult the same single-source-of-truth
`REJECTION_SET` **before** degrading, emitting the identical fail-loud diagnostic
as the standalone path. In addition, a **fail-CLOSED guard in `portrait-sema`**
now rejects *any* `Stmt::Raw` that survives to a **covenant-role** entrypoint
(transition / verification) — closing the whole class, not just the blacklisted
constructs, so no untyped statement the emitter would silently drop can reach a
covenant. (Verified empirically: all 31 covenant sources lower with zero
`Stmt::Raw` in any covenant transition, so this breaks no legitimate covenant.)

| §3 item | Construct | Enforced? | Where |
|---|---|---|---|
| 1 | shared mutable mapping | **Fail-loud** (`map<K,V>` state field) | `parse_state_block`, `REJECTION_SET` lead `mapping` |
| 2 | `approve` / `transferFrom` | **Fail-loud** (statement head) | `REJECTION_SET` leads `approve`, `transferFrom` |
| 3 | synchronous cross-contract call | **Fail-loud** (`.call(`, `.delegatecall(`) | `REJECTION_SET` leads `call`, `delegatecall` (method-call match) |
| 7 | event logs | **Fail-loud** (`emit ...` statement) | `REJECTION_SET` lead `emit` |
| 9 | unbounded loops | **Fail-loud** (`for`, `while`) | `REJECTION_SET` leads `for`, `while` |
| 4 | global aggregate queries (`totalSupply()`) | Prose-only | no `.portrait` surface — not expressible to begin with |
| 5 | `ERC20Votes` token-weighted governance | Prose-only | no `.portrait` surface |
| 6 | AMM / DeFi math on shared state | Prose-only | no `.portrait` surface |
| 8 | pre/post-transfer hooks with shared state | Prose-only | needs shared-state primitive absent from the DSL |

**Why the prose-only rows are not a gap:** items 4, 5, 6, 8 have no syntactic form
in the `.portrait` DSL — there is no way for a developer to *write* a global
aggregate query or a snapshot vote-weight in the covenant grammar, so there is no
construct at which to fail. The fail-loud rows are precisely the constructs a
developer *could* transcribe from Solidity and that must be caught.

**`Stmt::Raw` is not a silent accept anymore.** A genuinely-unrecognised
(non-blacklisted) statement form still *parses* to `Stmt::Raw` — but if it
survives to a **covenant-role** entrypoint (transition / verification), the
`portrait-sema` fail-CLOSED guard rejects it, naming the statement and routing it
to the vProgs layer (the emitter consumes only `Require`/`Return`, so an untyped
`Raw` there would otherwise be silently dropped). `Stmt::Raw` is therefore only
*tolerated* in **non-covenant (vProgs / Tier-3) entrypoints**, which are not
projected to a `.sil` covenant here (Atelier owns them); there it remains a
recorded hole visible to the checker. The net boundary is precise
(blacklisted constructs fail-loud at parse with a named diagnostic) **and**
fail-closed (no untyped statement can silently reach a covenant).

**Validation:** REJECT fixtures
(`portrait/examples/engraver-demo/rejected/{AllowanceToken,LoopAirdrop,CrossCallVault,RequireCrossCallVault,ReturnCrossCallVault}.portrait`
— the last two are the embedded-vector regression fixtures)
and ACCEPT fixtures
(`examples/engraver-demo/{SimpleEscrow,OwnableCounter}.portrait`, ACCEPT set 3 → 5)
are wired into the golden harness (`crates/portrait-cli/tests/golden.rs` §4–5) and
`portrait-syntax` unit tests. See `docs/ENGRAVER-VALIDATION.md`.

---

## 4. Vertical-specific validation

Validation exercise: classify three real Ethereum contracts from target verticals.

### 4.1 ERC20 (canonical implementation)

**Contract:** the canonical widely-used `ERC20.sol` + `ERC20Burnable.sol` + `ERC20Pausable.sol` implementation

| Feature | Tier | Notes |
|---|---|---|
| `transfer`, `mint`, `burn` | Green | KTT state machine covers all three |
| `approve` / `transferFrom` | Red | UTXO allowance pattern impossible without global state |
| `pause`/`unpause` | Green | `kcp-common::security::Pausable` |
| `_beforeTokenTransfer` | Red | Compliance hook needs global check → vProg guest |
| `totalSupply` | Red | Global query |

**Verdict:** Core `transfer`/`mint`/`burn` + `pause` maps cleanly. `approve`/`transferFrom` and compliance hooks route to vProgs.

### 4.2 TimelockController (canonical governance implementation)

**Contract:** the canonical widely-used `TimelockController.sol` implementation

| Feature | Tier | Notes |
|---|---|---|
| `schedule(target, value, data, delay)` | Green | `kcp-governance::TimelockAction` covers proposal + DAA-height delay |
| `execute` | Green | Enforced by `kcp-common::security::TimelockController` condition |
| `cancel` | Green | Lifecycle state in governance record |
| `hasRole` (proposer/executor) | Yellow | Multisig key list; no dynamic role assignment |
| `getTimestamp` | Yellow | DAA score query (approx. via node RPC) |
| batch operations | Yellow | Multi-input covenant patterns (within limits) |

**Verdict:** Full core governance flow maps. Dynamic role management and batch ops need adaptation.

### 4.3 Centrifuge `Tranche` / RWA token (institutional target vertical)

**Contract:** Centrifuge Liquidity Pools — tranche token with `updatePrice`, `burn`, `mint`, epoch-based redemptions.

| Feature | Tier | Notes |
|---|---|---|
| `mint`/`burn` per epoch | Green | KTT-based: minter flag controls issuance; epoch boundary is off-chain |
| `updatePrice` | Red | Shared NAV price requires global oracle → vProgs |
| Epoch-based redemptions queue | Red | Queue is global shared state → vProgs |
| Compliance hooks (KYC gate) | Yellow | CSCI + vProg guest for rule-hash-bound compliance settlement |
| `isTrustedForwarder` | Red | Cross-contract meta-transaction pattern → not applicable |

**Verdict:** Token lifecycle (mint/burn) maps to KTT. Price feed, redemption queue, and KYC gate route to vProgs + CSCI respectively. This is exactly the CSCI flagship use case.

---

## 5. Key insights for the compiler

1. **State field projection is straightforward.** Every Solidity `uint256 x;` that is UTXO-local becomes a covenant state field. The Portrait compiler handles this in `Pounce` (projection).

2. **The access control pattern maps 1:1 for the common case.** `onlyOwner` → `checkSig(s, prevStates[0].ownerPk)` in silverscript. Portrait can emit this directly from the role's ownership invariant.

3. **Mappings break the model.** Any Solidity construct that requires `mapping(k => v)` is a signal to route to the vProgs layer. The rejection set is precisely this set.

4. **Events become UTXO graph queries.** Solidity `emit` has no direct equivalent; off-chain indexers reconstruct events from UTXO transitions.

5. **The `from/to` binding is the UTXO equivalent of Solidity's `msg.sender` + `storage`.** A covenant binding `from = max_ins, to = max_outs` replaces the sender check + storage write in a single construct.

---

## 6. Caveats

- Classification reflects the current Toccata engine (`rusty-kaspa` v2.0.0 = `90dbf07`) and silverscript `@2c46231`.
- Yellow/Red tiers may shift Green as vProgs tooling matures.
- This is a self-assessment, not a formal equivalence proof. External audit required for v1.0.
- KRC-20 (native asset layer, Toccata) may enable some Yellow/Red tiers to become Green for token-only use cases.
