# kcp-vault

> **v1 — unaudited — testnet first.**

A Kaspa compliance pattern: covenant-locked custody with timelock and
multisig spending conditions. Compiles real opcode scripts and anchors
their digest on-chain in a carrier transaction.

Part of the `kaspa-compliance-patterns` workspace, targeting the
[Toccata](https://github.com/kaspanet/rusty-kaspa) hardfork (~30 Jun 2026,
DAA 474,165,565).

---

## v1 on-chain enforcement

As of v1, value is **locked under a real P2SH covenant script** and
**spent by satisfying that script** — consensus-enforced, not just
digest-anchored.

| Condition | On-chain enforcement | Notes |
|---|---|---|
| `MultiSig` (k-of-n) | **Live** | k signatures in key order; no Bitcoin-style dummy element |
| `TimelockHeight` (CLTV) | **Live** | tx.lock_time must be < 500B and >= deadline; sequence != 0xffff...ffff |
| `TimelockUnixSeconds` (CLTV) | **Live** | tx.lock_time must be >= 500B and >= deadline |
| `Any(2)` composite | **Live** (Any(2) branch-selected) | Branch selected by `OP_IF` selector; canonical case: Any(timelock, multisig) |
| `All(leaves)` composite | **Live** (All offline-tested; live path implemented) | All-satisfier in reverse-leaf push order; CLTV uses non-final sequence |

The offline engine preflight (`verify_p2sh_spend_offline`) runs the real
rusty-kaspa script engine over the fully-built spend before any submission.
A wrong satisfier is rejected before any RPC call is made.

### CLTV note (Kaspa vs Bitcoin)

Kaspa's `OP_CHECKLOCKTIMEVERIFY` **pops** the deadline from the stack
(unlike Bitcoin's which merely peeks). The `onchain` module uses
`compile_timelock_p2sh_redeem` (no `OP_DROP`) for P2SH spending; the
`compile_condition` function retains `OP_DROP` for the pure evaluator model
and its existing tests. This difference is documented as an open technical
finding.

### No dummy element for `OP_CHECKMULTISIG`

Kaspa's `OP_CHECKMULTISIG` does **not** consume a Bitcoin-style leading
dummy stack element. The satisfier for a k-of-n multisig is exactly k
signatures in the same order as their keys appear in the script.

---

## v0 evidence model — honest accounting

> v0 behaviour is preserved unchanged.

### What IS done in v0

| Claim | Mechanism |
|---|---|
| Real opcode scripts compiled | `kaspa_txscript::ScriptBuilder` with Toccata opcodes (`OP_CHECKLOCKTIMEVERIFY`, `OP_CHECKMULTISIG`, `OP_IF`/`OP_ELSE`/`OP_ENDIF`) |
| Script digest anchored on-chain | `vault_script_digest` (domain-separated SHA-256) embedded in the carrier transaction payload |
| Condition evaluated offline | Pure `evaluate(condition, ctx)` — no node required |

### What was NOT done in v0 (now addressed in v1)

The vault UTXO in v0 was key-controlled (pay-to-address). In v1, use
`lock_vault_tx` / `spend_multisig_vault` / `spend_timelock_vault` from the
`onchain` module (feature `wrpc`) to lock value under the script and spend
by satisfaction. The `anchor_vault_tx` + digest-only path remains available
for the evidence-payload model.

---

## Spending conditions

| Condition | Parameters | Evaluates true when |
|---|---|---|
| `TimelockHeight { deadline, controller_xonly }` | DAA height; 32-byte x-only key | `ctx.daa_score >= deadline` |
| `TimelockUnixSeconds { deadline, controller_xonly }` | Unix seconds; 32-byte x-only key | `ctx.unix_seconds >= deadline` |
| `MultiSig { threshold, xonly_keys }` | 1 ≤ threshold ≤ keys.len() ≤ 16; no duplicates | distinct present signers among `xonly_keys` ≥ `threshold` |
| `All(children)` | non-empty; max depth 8 | all children evaluate true |
| `Any(children)` | non-empty; max depth 8 | at least one child evaluates true |

Call `SpendCondition::validate()` before anchoring. Every rule above is
checked with a precise error.

---

## Script compilation limits (v0)

The pure evaluator (`crate::evaluator`) handles arbitrary nesting up to
depth 8. Script *compilation* (`crate::script::compile_condition`) is
deliberately limited in v0:

| Shape | Compiled? |
|---|---|
| Leaf (`TimelockHeight`, `TimelockUnixSeconds`, `MultiSig`) | Yes |
| `All(leaves)` | Yes — sequential `OP_VERIFY` |
| `Any` of exactly 2 branches (each a leaf or `All(leaves)`) | Yes — `OP_IF`/`OP_ELSE`/`OP_ENDIF` |
| `Any` of 1 or 3+ branches | `CompileUnsupported` error |
| `All` with composite children | `CompileUnsupported` error |
| Nested `Any` inside `Any` | `CompileUnsupported` error |

Conditions outside these shapes return `Error::CompileUnsupported` with an
explanatory message. Generalised n-ary branch compilation is deferred to a
future version.

---

## Compiled script shapes

### Timelock (height or unix-seconds)

```text
<deadline i64> OP_CHECKLOCKTIMEVERIFY OP_DROP <controller_xonly 32 bytes> OP_CHECKSIG
```

### MultiSig (k-of-n)

```text
<threshold i64> <pk1> … <pkN> <n i64> OP_CHECKMULTISIG
```

### Any(branch_a, branch_b)

```text
OP_IF
    <compiled branch_a>
OP_ELSE
    <compiled branch_b>
OP_ENDIF
```

Spending witness selects with `OP_1` (branch_a) or `OP_0` (branch_b).

---

## Usage

### Pure (no node required)

```rust
use kcp_vault::{
    condition::SpendCondition,
    evaluator::{evaluate, EvalContext},
};

// Build a 2-of-2 multisig condition.
let condition = SpendCondition::MultiSig {
    threshold: 2,
    xonly_keys: vec![[0x01; 32], [0x02; 32]],
};
condition.validate().unwrap();

// Evaluate offline.
let ctx = EvalContext {
    daa_score: 0,
    unix_seconds: 0,
    signers_present: vec![[0x01; 32], [0x02; 32]],
};
assert!(evaluate(&condition, &ctx));
```

### With script compilation (feature `wrpc`)

```rust
use kcp_vault::script::{compile_condition, vault_script_digest};
use kcp_vault::payload::Payload;
use kcp_common::canonical::canonical_hash;

let condition = /* ... */;
let script_bytes = compile_condition(&condition).unwrap();
let digest = vault_script_digest(&script_bytes);
let vault_id = canonical_hash(&condition).unwrap();

let payload = Payload { vault_id, script_digest: digest };
let payload_bytes = payload.encode();
// anchor payload_bytes on-chain via anchor_vault_tx ...
```

### Anchoring on-chain — v0 digest model (feature `wrpc`)

```rust
use kcp_vault::tx::{anchor_vault_tx, DEFAULT_VAULT_VALUE_SOMPI};
// See examples/vault_testnet_evidence.rs for the full flow.
```

### Locking and spending under script — v1 P2SH enforcement (feature `wrpc`)

```rust
use kcp_vault::onchain::{lock_vault_tx, spend_multisig_vault, spend_timelock_vault};
// Lock 1 KAS under a 2-of-2 multisig covenant.
let lock_tx = lock_vault_tx(&rpc, &wallet, &condition, 100_000_000).await?;
// Spend it — engine-verified before submission.
let spend_tx = spend_multisig_vault(
    &rpc, &condition, (lock_txid, 0),
    &[keypair0, keypair1], &dest, Prefix::Testnet, fee,
).await?;
// See examples/vault_onchain_evidence.rs for the full flow.
```

---

## Testnet evidence

Recorded 2026-06-11 on **testnet-10** (local kaspad v2.0.0, synced, DAA ~488,321,962):

- vault_id `1d556f50430c86361da02447c3721c95800d01c2c896ddea9e3102bd49f3a4ff`
- script_digest `bda6260caabc0353a42a64fc158cab9c53c36130f17aab7ef76f95749e009cae`
  (113-byte real-opcode script: Any(timelock-unix, 2-of-2 multisig); offline
  evaluation checked both sides of the deadline)
- anchor tx `eb2b58b82e111b06bf93a0ccde9cdfa3bf76e21e52564352640b3e5592a11fc3`

Honest scope (v0): the compiled script's digest is anchored; value is NOT
locked under the script in v0.

**v1 on-chain enforcement (KCP-VT-002), 2026-06-11 on testnet-10:** value
locked under a real 2-of-2 multisig covenant (P2SH) and released by providing
two valid signatures — consensus-enforced.

- lock tx `973707fc5630e350b63915b1ac62a2c832f797be3669ead25d96634e4df0d06f`
- spend tx `81ab3171bd0f942134504dd4cd28bea4caca0807501747a74c3f2a270517596d`
- engine-preflighted before submit; timelock CLTV on-chain also implemented
  (`spend_timelock_vault`). Composite `Any`/`All` on-chain spend = next step.

Testnet evidence is perishable — testnets reset by design. Record the network
and date with any claim, and refresh by re-running the example.

To reproduce or refresh: fund a testnet wallet for the target network, then run:

```sh
KCP_NODE_URL=ws://localhost:17210 \
KCP_KEY_FILE=/path/to/wallet.key \
cargo run -p kcp-vault --example vault_testnet_evidence --features wrpc
```

3. Record the printed `KCP-VT-001` evidence block alongside your own run (the
   maintainers track it in `docs/EVIDENCE.md`).

The example prints a FACTS-ready block with an honest note:
`"v0 — script compiled + digest anchored; value not yet locked under script"`.

---

## Caveats

- This crate does not mention KCC20, KRC-20, or KTT internally.
- The fee constant `kcp_common::tx::CARRIER_FEE_SOMPI` is fixed and
  conservative for today's testnet; adjust if mempool rules change at Toccata.
- `kaspa-txscript` and sibling crates are pinned to the `v2.0.0` tag of
  `rusty-kaspa`. The API may change before mainnet activation.
- **CLTV script shape discrepancy:** `compile_condition` emits
  `<deadline> OP_CLTV OP_DROP <xonly> OP_CHECKSIG` (Bitcoin-style, where CLTV
  does not pop). Kaspa's CLTV pops the deadline, so the `onchain` module uses a
  different P2SH redeem shape without `OP_DROP` for actual spending. This means
  a vault locked by `lock_vault_tx` with a `TimelockHeight` or
  `TimelockUnixSeconds` condition uses a different script than `compile_condition`
  alone produces. The `onchain` module is self-consistent (lock and spend use
  the same P2SH-correct form).
- **Composite on-chain spend:** `Any(2)` and `All(leaves)` composite conditions
  are now supported via `spend_any_vault` and `spend_all_vault` (feature `wrpc`).
  The P2SH-correct compiler `compile_condition_p2sh` is used for the on-chain
  spend path; `compile_condition` is retained for the pure evaluator. For
  `Any(2)`: branch 0 (OP_IF) is selected by `[0x01]` (truthy); branch 1
  (OP_ELSE) by `[]` (empty/falsy). For `All(leaves)`: satisfier elements are
  assembled in reverse leaf order (last leaf's elements are pushed first). Both
  paths are engine-preflighted. Unaudited — testnet first.
- The `controller_xonly` field on timelock leaves carries the key for the
  compiled script (`OP_CHECKSIG`). In v1 the timelock UTXO is locked under the
  P2SH of the timelock script, so this field controls the actual on-chain
  spending rule.

---

## Threat model

> **Pre-production, unaudited, testnet-only; testnet evidence is perishable.**
> This section distils what the crate documents. It is **not a security audit**
> and not an assurance that the properties below hold.

**Assets** — the KAS locked under a vault P2SH, and the spending policy itself
(which keys, and from which DAA height or unix time, can move it).

**Attacker capabilities (assumed)** — on a UTXO chain the **spender is the
adversary**: they choose the spending transaction, its inputs and outputs, the
satisfier bytes, `lock_time` and `sequence`, and when to broadcast. The
adversary may be a counterparty holding some of the `k` keys, the party who
instantiated the vault and chose the condition, or an unrelated third party who
sees the redeem script the moment the first spend reveals it.

**What consensus enforces** — value is locked under the real P2SH of the
compiled script, so the engine demands a redeem preimage that hashes to the
locked script hash *and* a satisfier that makes it evaluate true: `k` valid
Schnorr signatures over the transaction sighash, in key order, with no
Bitcoin-style dummy element (`MultiSig`); `tx.lock_time >= deadline`, with the
deadline and `tx.lock_time` on the same side of `LOCK_TIME_THRESHOLD`
(`500_000_000_000`), and a non-final sequence (CLTV timelocks); the selected
`OP_IF` branch (`Any(2)`); every leaf (`All(leaves)`).
`verify_p2sh_spend_offline` runs the pinned v2.0.0 engine over the fully-built
spend before any submission `[KCP-VT-002]`.

**What this assumes / trusts off-chain** — key custody is the whole security of
the pattern; a threshold of keys is a threshold of *people*. The caller supplies
`xonly_keys` / `controller_xonly`, and `validate()` checks count, range and
duplication — **not** that each key is a valid curve point, so a malformed key
mints an unspendable vault. The caller also picks the fee;
`CARRIER_FEE_SOMPI` is a fixed conservative constant, not a mempool-policy
calculation. On the v0 digest path the UTXO is key-controlled and only a script
*digest* is anchored: that anchoring shows a script was published, never that it
governs any funds.

**Known limits and non-goals** — the vault constrains **who** may spend and
**when**, never **where the value goes**: once the condition is satisfied the
spender picks the outputs freely, so "covenant-locked custody" here means a
spending condition, not output introspection. It does not protect against key
compromise, coercion, or collusion at or above the threshold. Timelocks resolve
against consensus DAA score / lock-time rules, not a wall clock; a deadline is
not a statement about real-world time. **`TimelockUnixSeconds` is a misnomer**:
values at or above `LOCK_TIME_THRESHOLD` are compared against the block
timestamp, which rusty-kaspa keeps in **milliseconds** (`unix_now()`), so a
deadline expressed in real unix *seconds* falls below the threshold and is
silently treated as a DAA score instead — and the pure `evaluator`, which
compares against a seconds-valued `unix_seconds`, disagrees with consensus about
the unit. Pass milliseconds; see `KNOWN-ISSUES.md`.
`compile_condition` (evaluator and digest shape) emits a *different* script from
the P2SH redeem the `onchain` module locks under, so a v0 anchored digest does
not identify a v1 vault script. Condition shapes outside the compiled set
(`Any` of 1 or 3+ branches, nested `Any`, `All` with composite children) are
rejected outright, not degraded. A timelock leaf carries a **single deadline**:
this crate cannot express a gradual release, so a `kcp-vesting` schedule can
only be enforced on-chain by pre-splitting the value into one timelocked UTXO
per tranche. Unaudited; not for mainnet value.
