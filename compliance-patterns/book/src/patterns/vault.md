# kcp-vault

`kcp-vault` provides covenant-locked custody for Kaspa: a P2SH redeem script
that enforces multisig, timelock, or composite conditions at the consensus layer.
It is the `SafeERC20`/escrow equivalent for Kaspa — the foundation for any
pattern that requires locked, condition-guarded assets.

## Constructing a vault covenant

The core type is [`SpendCondition`]. Build one, compile it to a redeem script,
and derive the P2SH hash from `vault_script_digest`.

```rust
use kcp_vault::{
    condition::SpendCondition,
    onchain::compile_condition_p2sh,
    script::vault_script_digest,
};

// 2-of-3 multisig vault
let condition = SpendCondition::MultiSig {
    threshold: 2,
    xonly_keys: vec![key_a, key_b, key_c],
};
let redeem    = compile_condition_p2sh(&condition)?;
let p2sh_hash = vault_script_digest(&redeem); // blake2b hash for P2SH scriptPubKey
```

To spend, build a satisfier that provides the required signatures in the correct order and submit via the kaspa_txscript P2SH spend helper.

**It is critical** that `threshold ≤ xonly_keys.len()` — the condition validator enforces this but callers should not rely on it as a contract boundary.

## A note on CLTV: DAA heights vs. unix-seconds

Kaspa's `OP_CHECKLOCKTIMEVERIFY` pops the deadline from the stack (unlike Bitcoin's peek). This means the spendable redeem script must **not** emit `OP_DROP` after `OP_CHECKLOCKTIMEVERIFY`. `compile_condition_p2sh` handles this correctly; do not use `compile_condition` for P2SH redeem scripts — it emits the extra `OP_DROP` that makes the script unspendable.

```rust
// CORRECT — use for P2SH redeem
let redeem = compile_condition_p2sh(&SpendCondition::TimelockHeight { deadline, key })?;

// WRONG — emits OP_DROP after CLTV; unspendable
// let redeem = compile_condition(&...)?;
```

## Extensions

- **Composite conditions** — `SpendCondition::All(vec![TimelockHeight {...}, MultiSig {...}])` requires both conditions simultaneously.
- **Governance integration** — use the same key set as your `kcp-governance` signatories so a governance vote directly authorises a spend. See `examples/governance-demo`.
- **PQ spend authorisation** — replace the Schnorr satisfier with a RISC Zero guest via `kcp-pq-anchor`. See [PQ Anchor](./pq-anchor.md).

→ API reference: [`SpendCondition`], [`compile_condition_p2sh`], [`vault_script_digest`], [`evaluate`]
