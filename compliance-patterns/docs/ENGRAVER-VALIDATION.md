# Engraver Validation

**Status:** Pre-production, unaudited, testnet-only. Portrait M1 / Engraver first validation pass.

**Engraver pipeline:** `.portrait` → portrait-syntax → portrait-ir → portrait-pounce → portrait-emit → silverc

## Validated contracts (2026-06-28)

All 5 contracts below were compiled with `portrait engrave`.
Each produced a `.sil` file compiled by silverc to a valid contract artifact (exit 0).
(Originally 3; SimpleEscrow + OwnableCounter were added in the rejection-boundary
increment, growing the ACCEPT set 3 → 5.)

### 1. SimpleToken — Green-tier ERC20 transfer-only

**Source:** `portrait/examples/engraver-demo/SimpleToken.portrait`

**Solidity equivalent:**
```solidity
contract SimpleToken {
    uint256 public balance;
    function transfer(uint256 amount) external returns (uint256);
}
```

**Pounce allocation:**
- `transfer` → **Covenant** (state-output transition, `to = 1`)

**Silverc result:** `[silverc] ok`

**Limitation (under-specified source, not an emitter gap):** the `.portrait`
body has **no** `requires` clause, so the covenant enforces only the output
state shape. This is the emitter being honest to an under-specified source —
adding `requires amount >= 0; requires amount <= balance;` to the `.portrait`
body and re-engraving would emit the corresponding `require(...)` calls (the
guard-lowering path is proven by SimpleEscrow/OwnableCounter above). It is NOT
that guards written in the source are dropped.

---

### 2. PausableToken — Green-tier ERC20 + Pausable

**Source:** `portrait/examples/engraver-demo/PausableToken.portrait`

**Solidity equivalent:**
```solidity
contract PausableToken is ERC20, Pausable {
    uint256 public balance;
    bool public paused;
    function transfer(uint256 amount) external whenNotPaused;
    function setPaused(int flag) external onlyOwner;
}
```

**Pounce allocation:**
- `transfer` → **Covenant**
- `set_paused` → **Covenant**

**Silverc result:** `[silverc] ok`

**Limitation (under-specified source, not an emitter gap):** the `transfer` and
`set_paused` bodies carry no `requires` clause, so neither the `whenNotPaused`
(`require(prev_states[0].paused == 0)`) nor an `onlyOwner` sig check is emitted.
Adding those as body `requires` clauses and re-engraving would emit them via the
same lowering path SimpleEscrow/OwnableCounter use. There is no separate
`Guard::Sig` IR step required — guards ride through the entrypoint body.

---

### 3. VestingWallet — Green-tier time-locked token release

**Source:** `portrait/examples/engraver-demo/VestingWallet.portrait`

**Solidity equivalent:**
```solidity
contract VestingWallet {
    uint64 public released;
    function release(uint64 amount) external returns (uint64 released);
}
```

**Pounce allocation:**
- `release` → **Covenant**

**Silverc result:** `[silverc] ok`

**Limitation (under-specified source, not an emitter gap):** the `release` body
carries no `requires` clause, so the `amount <= (releasable - released)` guard is
not emitted. Adding it as a body `requires` clause and re-engraving would emit
the corresponding `require(...)` via the proven lowering path.

---

### 4. SimpleEscrow — Green-tier one-shot arbiter release

**Source:** `portrait/examples/engraver-demo/SimpleEscrow.portrait`

**Solidity equivalent:**
```solidity
contract SimpleEscrow {
    address public arbiter;
    uint256 public released;  // 0 = held, 1 = released
    function release(bytes sig) external;  // arbiter releases once
}
```

**Pounce allocation:**
- `release` → **Covenant**

**Silverc result:** `[silverc] ok SimpleEscrow.sil`

**Guards emitted (verified).** The body `requires` clauses ARE lowered to
silverscript `require(...)` calls. The committed `SimpleEscrow.sil` contains:

```silverscript
require(checkSig(auth, prev_states[0].arbiter));
require(prev_states[0].released == 0);
return({ arbiter: prev_states[0].arbiter, released: 1 });
```

The `checkSig(auth, arbiter)` authorization guard and the `released == 0`
one-shot guard are both enforced — not shape-only. Wired into the golden harness
(`accept_simple_escrow_projects_and_compiles`).

---

### 5. OwnableCounter — Green-tier onlyOwner-gated counter

**Source:** `portrait/examples/engraver-demo/OwnableCounter.portrait`

**Solidity equivalent:**
```solidity
contract OwnableCounter is Ownable {
    uint256 public count;
    function increment(bytes sig) external onlyOwner;
}
```

**Pounce allocation:**
- `increment` → **Covenant**

**Silverc result:** `[silverc] ok OwnableCounter.sil`

**Guards emitted (verified).** The body `requires checkSig(auth, owner)` clause
IS lowered to a silverscript `require(...)`. The committed `OwnableCounter.sil`
contains:

```silverscript
require(checkSig(auth, prev_states[0].owner));
return({ owner: prev_states[0].owner, count: prev_states[0].count + 1 });
```

The `onlyOwner` access-control guard is enforced — not shape-only. Wired into
the golden harness (`accept_ownable_counter_projects_and_compiles`).

---

## Rejection fixtures

The critical property of the pipeline is the **subset boundary**: an out-of-subset
Solidity construct must be rejected **fail-loud, naming the offending construct**,
NOT silently miscompiled. As of this increment the rejection set
(`docs/SOLIDITY-SUBSET-V0.md §3`) is **enforced in code** — `portrait-syntax`
carries a single-source-of-truth `REJECTION_SET` table and rejects blacklisted
constructs at parse time (replacing the prior silent `Stmt::Raw` degradation for
those forms). The three fixtures below are wired into the golden harness as REJECT
cases that assert the diagnostic names the construct and routes it to the vProgs
layer.

| Fixture | Out-of-subset construct | SUBSET-V0 §3 | `portrait engrave` result |
|---|---|---|---|
| `rejected/AllowanceToken.portrait` | `map<K, V>` state field (shared mutable mapping) + `transferFrom` | items 1 + 2 | rejected: `unsupported construct \`map<K, V>\` state field … (deferred to the vProgs layer; see SOLIDITY-SUBSET-V0 §3 item 1)` |
| `rejected/LoopAirdrop.portrait` | unbounded `for` loop | item 9 | rejected: `unsupported construct \`for\`: unbounded loops cannot be expressed in a covenant … (deferred to the vProgs layer; see SOLIDITY-SUBSET-V0 §3 item 9)` |
| `rejected/CrossCallVault.portrait` | synchronous cross-contract `.call(...)` | item 3 | rejected: `unsupported construct \`call\`: synchronous cross-contract calls … (deferred to the vProgs layer; see SOLIDITY-SUBSET-V0 §3 item 3)` |

Source: `portrait/examples/engraver-demo/rejected/`.
Harness: `crates/portrait-cli/tests/golden.rs` §4 (`reject_*_names_construct`) +
unit tests in `portrait-syntax` (`tests::rejects_*`).

**Boundary scope (honest):** the enforced rejection set covers the statement- and
state-field-level constructs in the `REJECTION_SET` table (`for`, `while`, `emit`,
`mapping`/`map<K,V>`, `approve`, `transferFrom`, `.call(`, `.delegatecall(`).
Other §3 prose items that have no syntactic surface in the `.portrait` DSL
(e.g. global aggregate queries like `totalSupply()`, `ERC20Votes`, AMM math) remain
prose-only because a developer cannot express them in `.portrait` in the first
place — they have no construct to reject. Genuinely-unrecognised (but not
blacklisted) statement forms still fall back to `Stmt::Raw` (visible to the
checker), so the boundary is precise, not over-broad.

---

## Guard lowering — how it actually works

There is exactly **one** guard-lowering path, and it works:

- A `requires <expr>;` clause inside an entrypoint body parses to
  `Stmt::Require(Expr)` (portrait-syntax), rides through `Transition.body`
  unchanged (portrait-ir), and is lowered by portrait-emit
  (`crates/portrait-emit/src/lib.rs`) to a silverscript `require(emit_expr(...))`.
  State-field references are rewritten to `prev_states[0].field`; constructor
  params and entrypoint args pass through. This is why SimpleEscrow,
  OwnableCounter, and the hand-authored library covenants (CsciInstrument,
  MultisigTreasury, Htlc, …) all emit real `require(checkSig(...))` / `blake2b`
  guards.

> **Correction (prior versions of this doc were wrong).** Earlier text claimed a
> "Cartoon IR `Guard::Sig`/`AgeAtLeast`/`Eq`" path was the carrier and that
> guards were "not emitted in M1". That `Guard` enum (`portrait-ir/src/lib.rs`)
> was **dead vestigial code** — `Transition.guards` was initialised empty
> everywhere and read by nothing. Guards have always ridden through the body as
> `Stmt::Require`. A covenant whose `.portrait` body carries `requires` IS
> guarded; a covenant whose body omits `requires` is honestly shape-only.

## Known emit gaps (portrait-emit)

| Gap | Detail | Priority |
|---|---|---|
| Source omits `requires` | SimpleToken/PausableToken/VestingWallet `.portrait` bodies carry no `requires` clause, so they emit shape-only. Not an emitter defect — add the `requires` to the source and re-engrave. | Medium — source completeness |
| Unrecognised guard form (`@` age syntax) now **fails loud** | `requires v @ 1;` parses to `Stmt::Raw`. portrait-emit previously had no `Stmt::Raw` arm and **silently dropped** it — a soundness/honesty hazard (a covenant that LOOKS gated but enforces nothing). As of this increment, emit returns an **error naming the offending guard and contract** rather than emitting an unguarded `.sil`. A future pass may add `@` as a recognised temporal guard form. | Resolved (fail-loud); temporal-guard lowering is future |
| Literal-only return not supported | `return 1` doesn't reference a field; emit falls back to pass-through. Workaround: use `return field - field + literal` | Medium — workaround exists |
| Return field name not carried | Parser discards the `(int fieldName)` return-type name; emit uses field-name substring search | Low — causes wrong field selection on complex expressions |

## Engraver pipeline status

| Stage | Status |
|---|---|
| portrait-syntax parse | proven (M0); rejection set enforced fail-loud (`REJECTION_SET`) |
| portrait-ir lower | proven (M1 — args + body) |
| portrait-pounce allocate | proven (M0 — Covenant/VProg classification, 3 tests) |
| portrait-emit emit | proven (M1 — return-type syntax, to = 1, multi-field return) |
| silverc compile | proven — all 5 ACCEPT Engraver contracts exit 0; 3 REJECT fixtures fail-loud |
