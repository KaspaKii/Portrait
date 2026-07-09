//! Consensus-enforced P2SH vault: lock value under a compiled covenant script
//! and spend it by satisfying that script.
//!
//! This module upgrades the v0 "script digest anchored" model to full on-chain
//! enforcement: the vault UTXO is locked under
//! `p2sh_lock_script(compile_condition_p2sh(condition))` and can only be spent
//! by providing a valid satisfier for that condition — verified by the real
//! Kaspa script engine before any submission.
//!
//! ## Supported on-chain spend paths (v1)
//!
//! | Condition                | Status | Notes                                                             |
//! |--------------------------|--------|-------------------------------------------------------------------|
//! | `MultiSig` (k-of-n)      | Live   | Engine-proved; satisfier = k ordered sigs                        |
//! | `TimelockHeight` (CLTV)  | Live   | Engine-proved; tx.lock_time must ≥ deadline                      |
//! | `TimelockUnixSeconds`    | Live   | Engine-proved; tx.lock_time must ≥ deadline                      |
//! | `Any(2)` composite       | Live   | Branch-selected; engine-proved; canonical case: Any(tl, multisig)|
//! | `All(leaves)` composite  | Live   | All-satisfier; engine-proved; sequential OP_VERIFY chain         |
//!
//! ## No dummy element for `OP_CHECKMULTISIG`
//!
//! Kaspa's `op_check_multisig_schnorr_or_ecdsa` does **not** consume a
//! Bitcoin-style leading dummy stack element. The script `<k> <pk1..pkN> <n>
//! OP_CHECKMULTISIG` pops `n`, pops n pubkeys, pops `k`, pops k signatures —
//! the satisfier is exactly `k` signatures in the same relative order as their
//! keys (first sig for pk1, second for pk2, etc.), with no leading `OP_0`.
//!
//! ## CLTV lock_time semantics
//!
//! Kaspa uses a single `lock_time` field with a threshold to distinguish types:
//! - `lock_time < 500_000_000_000` → DAA height (use with `TimelockHeight`)
//! - `lock_time ≥ 500_000_000_000` → Unix seconds (use with `TimelockUnixSeconds`)
//!
//! Both the in-script deadline value and `tx.lock_time` must be on the same
//! side of the threshold, and `tx.lock_time ≥ deadline`. The input sequence
//! must be `!= 0xffffffffffffffff` (`MAX_TX_IN_SEQUENCE_NUM`).
//!
//! ## CLTV P2SH redeem script shape (Kaspa vs Bitcoin)
//!
//! `crate::script::compile_condition` produces
//! `<deadline> OP_CLTV OP_DROP <xonly> OP_CHECKSIG`.
//! This mirrors the Bitcoin convention where `OP_CLTV` **does not pop** the
//! deadline from the stack (it merely checks it), so `OP_DROP` is needed to
//! clean up the deadline before CHECKSIG runs.
//!
//! Kaspa's `OpCheckLockTimeVerify` implementation **does pop** the deadline
//! (`pop_raw`), so `OP_DROP` ends up removing the signature rather than the
//! deadline — leaving the stack empty when CHECKSIG tries to run.
//!
//! To work around this, [`spend_timelock_vault`] and the timelock engine tests
//! use [`compile_timelock_p2sh_redeem`], a private helper that emits
//! `<deadline> OP_CLTV <xonly> OP_CHECKSIG` (no `OP_DROP`). For composite
//! conditions, [`compile_condition_p2sh`] uses this P2SH-correct leaf form
//! throughout. The `compile_condition` function is left unchanged because it is
//! correct for the pure software evaluator and its existing tests; only the
//! P2SH on-chain spend path uses the corrected shape. This difference is
//! documented as an open technical finding.
//!
//! ## Composite `Any(2)` spend semantics
//!
//! The redeem script shape for `Any(branch_a, branch_b)` is:
//! ```text
//! OP_IF <branch_a> OP_ELSE <branch_b> OP_ENDIF
//! ```
//!
//! When the script engine executes `OP_IF`, it pops the **top** stack element
//! as the branch selector. The satisfier for `Any` therefore places the branch
//! selector as its **last push** (immediately before the redeem script push),
//! so it sits on top of the stack when `OP_IF` runs:
//!
//! ```text
//! signature_script: <branch_satisfier_elems...> <selector> <redeem>
//! ```
//!
//! - `selector = [0x01]` (OP_1 / truthy) → executes `branch_a` (index 0).
//! - `selector = []`     (empty/OP_0 / falsy) → executes `branch_b` (index 1).
//!
//! ## Composite `All(leaves)` spend semantics
//!
//! The redeem script for `All([leaf_0, leaf_1, …, leaf_n])` is:
//! ```text
//! <leaf_0> OP_VERIFY <leaf_1> OP_VERIFY … <leaf_n>
//! ```
//!
//! The satisfier assembles each leaf's elements in order: leaf_0's satisfier
//! elements first, leaf_1's elements next, …, leaf_n's elements last.
//! Because the script is executed left-to-right and each leaf's elements are
//! consumed by its opcodes before the next leaf starts, the satisfier order
//! mirrors the script order.
//!
//! Status: **v1 — unaudited — testnet first.**

use kaspa_addresses::{Address, Prefix};
use kaspa_bip32::secp256k1::Keypair;
use kaspa_consensus_core::tx::TransactionId;
use kaspa_txscript::{
    opcodes::codes::{
        OpCheckLockTimeVerify, OpCheckMultiSig, OpCheckSig, OpElse, OpEndIf, OpIf, OpVerify,
    },
    script_builder::ScriptBuilder,
};
use kaspa_wrpc_client::KaspaRpcClient;

use kcp_common::{
    p2sh::{lock_to_p2sh_tx, schnorr_satisfier_sig, spend_p2sh_tx, spend_p2sh_tx_with_locktime},
    wallet::Wallet,
};

use crate::condition::SpendCondition;
use crate::error::{Error, Result};

// Re-export for convenience in example code.
pub use kcp_common::p2sh::redeem_script_hash;

/// Compile a P2SH-correct timelock redeem script for Kaspa.
///
/// Kaspa's `OpCheckLockTimeVerify` **pops** the deadline from the stack
/// (`pop_raw`), unlike Bitcoin's where CLTV merely peeks. This means the
/// standard pattern `<deadline> OP_CLTV OP_DROP <xonly> OP_CHECKSIG` produced
/// by `compile_condition` is incorrect for P2SH spending: `OP_DROP` ends up
/// removing the signature rather than the deadline.
///
/// This function emits `<deadline> OP_CLTV <xonly> OP_CHECKSIG` (no
/// `OP_DROP`), which is the correct shape for Kaspa P2SH CLTV spending.
///
/// `compile_condition` is left unchanged (it is correct for the pure evaluator
/// and its existing tests). This function is private to `onchain.rs`.
fn compile_timelock_p2sh_redeem(
    deadline: u64,
    controller_xonly: &[u8; 32],
) -> crate::error::Result<Vec<u8>> {
    let mut b = ScriptBuilder::new();
    b.add_i64(deadline as i64)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        .add_op(OpCheckLockTimeVerify)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        // No OP_DROP: Kaspa CLTV already pops the deadline.
        .add_data(controller_xonly)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
        .add_op(OpCheckSig)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    Ok(b.drain().to_vec())
}

/// Compile the redeem script for a `condition` using the P2SH-correct form.
///
/// For timelock conditions, the P2SH-correct shape omits `OP_DROP` (see
/// module-level docs on Kaspa's CLTV pop-semantics). Composites delegate to
/// [`compile_condition_p2sh`] so the locking redeem matches what
/// [`spend_any_vault`] / [`spend_all_vault`] derive (a composite with a
/// timelock branch must omit `OP_DROP` in that branch too — otherwise the lock
/// and spend resolve to different P2SH addresses and the value is unspendable).
fn p2sh_redeem_for(condition: &SpendCondition) -> Result<Vec<u8>> {
    match condition {
        SpendCondition::TimelockHeight {
            deadline,
            controller_xonly,
        }
        | SpendCondition::TimelockUnixSeconds {
            deadline,
            controller_xonly,
        } => compile_timelock_p2sh_redeem(*deadline, controller_xonly),
        _ => compile_condition_p2sh(condition),
    }
}

// ── P2SH-correct composite compiler ──────────────────────────────────────────

/// Compile a leaf condition to P2SH-correct bytes.
///
/// Uses [`compile_timelock_p2sh_redeem`] (no `OP_DROP`) for timelock leaves;
/// delegates to `compile_condition` for multisig leaves. Composite leaves are
/// not accepted here (callers must recurse via [`compile_condition_p2sh`]).
fn compile_leaf_p2sh(c: &SpendCondition) -> Result<Vec<u8>> {
    match c {
        SpendCondition::TimelockHeight {
            deadline,
            controller_xonly,
        }
        | SpendCondition::TimelockUnixSeconds {
            deadline,
            controller_xonly,
        } => compile_timelock_p2sh_redeem(*deadline, controller_xonly),

        SpendCondition::MultiSig {
            threshold,
            xonly_keys,
        } => {
            // Multisig shape is identical for pure evaluator and P2SH spend.
            let mut b = ScriptBuilder::new();
            b.add_i64(*threshold as i64)
                .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
            for pk in xonly_keys {
                b.add_data(pk.as_ref())
                    .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
            }
            b.add_i64(xonly_keys.len() as i64)
                .map_err(|e| Error::ScriptBuilder(format!("{e}")))?
                .add_op(OpCheckMultiSig)
                .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
            Ok(b.drain().to_vec())
        }

        SpendCondition::All { .. } | SpendCondition::Any { .. } => Err(Error::CompileUnsupported(
            "composite condition in leaf P2SH compilation path".into(),
        )),
    }
}

/// Compile `All(leaves)` to P2SH-correct bytes.
///
/// Output: `<leaf_0> OP_VERIFY <leaf_1> OP_VERIFY … <leaf_n>`
fn compile_all_leaves_p2sh(children: &[SpendCondition]) -> Result<Vec<u8>> {
    if children.is_empty() {
        return Err(Error::CompileUnsupported(
            "All with zero children is not valid".into(),
        ));
    }
    for (i, child) in children.iter().enumerate() {
        if matches!(
            child,
            SpendCondition::All { .. } | SpendCondition::Any { .. }
        ) {
            return Err(Error::CompileUnsupported(format!(
                "All: child at index {i} is composite; only leaf children are supported"
            )));
        }
    }

    let mut out = Vec::new();
    let last = children.len() - 1;

    let mut b_verify = ScriptBuilder::new();
    b_verify
        .add_op(OpVerify)
        .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
    let verify_bytes = b_verify.drain().to_vec();

    for (i, child) in children.iter().enumerate() {
        out.extend_from_slice(&compile_leaf_p2sh(child)?);
        if i < last {
            out.extend_from_slice(&verify_bytes);
        }
    }
    Ok(out)
}

/// Compile a branch (leaf or `All(leaves)`) to P2SH-correct bytes.
fn compile_branch_p2sh(c: &SpendCondition) -> Result<Vec<u8>> {
    match c {
        SpendCondition::All { children } => compile_all_leaves_p2sh(children),
        SpendCondition::Any { .. } => Err(Error::CompileUnsupported(
            "nested Any inside Any: v0 does not support Any branches containing Any".into(),
        )),
        leaf => compile_leaf_p2sh(leaf),
    }
}

/// Compile a [`SpendCondition`] to a P2SH-correct redeem script.
///
/// This is the correct compiler to use for **P2SH lock and spend** paths. It
/// differs from [`crate::script::compile_condition`] in one critical way:
/// timelock leaf conditions are compiled **without `OP_DROP`**, because
/// Kaspa's `OP_CHECKLOCKTIMEVERIFY` already pops the deadline from the stack
/// (see module-level docs). Using `OP_DROP` after CLTV would discard the
/// signature, causing the spend to fail.
///
/// ## Supported shapes
///
/// | Shape                     | Supported |
/// |---------------------------|-----------|
/// | Leaf (timelock / multisig)| Yes       |
/// | `All(leaves)`             | Yes       |
/// | `Any` of exactly 2 branches (each a leaf or `All(leaves)`) | Yes |
/// | `Any` of 1 or 3+ branches | `CompileUnsupported` error |
/// | `All` with composite children | `CompileUnsupported` error |
/// | Nested `Any` inside `Any` | `CompileUnsupported` error |
///
/// ## Errors
///
/// - [`Error::CompileUnsupported`] for unsupported composite shapes.
/// - [`Error::ScriptBuilder`] if the builder rejects the input.
pub fn compile_condition_p2sh(condition: &SpendCondition) -> Result<Vec<u8>> {
    match condition {
        SpendCondition::TimelockHeight { .. }
        | SpendCondition::TimelockUnixSeconds { .. }
        | SpendCondition::MultiSig { .. } => compile_leaf_p2sh(condition),

        SpendCondition::All { children } => compile_all_leaves_p2sh(children),

        SpendCondition::Any { children } => {
            if children.len() != 2 {
                return Err(Error::CompileUnsupported(format!(
                    "Any with {} branches: v0 P2SH compilation supports Any of exactly 2 branches",
                    children.len()
                )));
            }
            let a_bytes = compile_branch_p2sh(&children[0])?;
            let b_bytes = compile_branch_p2sh(&children[1])?;

            let mut b = ScriptBuilder::new();
            b.add_op(OpIf)
                .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
            let if_prefix = b.drain().to_vec();

            let mut b2 = ScriptBuilder::new();
            b2.add_op(OpElse)
                .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
            let else_bytes = b2.drain().to_vec();

            let mut b3 = ScriptBuilder::new();
            b3.add_op(OpEndIf)
                .map_err(|e| Error::ScriptBuilder(format!("{e}")))?;
            let endif_bytes = b3.drain().to_vec();

            let mut out = Vec::new();
            out.extend_from_slice(&if_prefix);
            out.extend_from_slice(&a_bytes);
            out.extend_from_slice(&else_bytes);
            out.extend_from_slice(&b_bytes);
            out.extend_from_slice(&endif_bytes);
            Ok(out)
        }
    }
}

// ── Composite Any(2) spend ────────────────────────────────────────────────────

/// Returns `true` if `condition` is a timelock leaf (CLTV-based).
///
/// Used internally to decide whether a spend path needs `lock_time` and
/// non-final `sequence` on the transaction.
fn is_timelock(condition: &SpendCondition) -> bool {
    matches!(
        condition,
        SpendCondition::TimelockHeight { .. } | SpendCondition::TimelockUnixSeconds { .. }
    )
}

/// Build the satisfier elements for a single leaf condition.
///
/// For a `MultiSig` leaf: returns `threshold` Schnorr signatures in key order.
/// For a timelock leaf: returns one Schnorr signature from `controller_keypair`
/// (which must correspond to the key in the leaf's `controller_xonly` field).
///
/// `sighash` is the Schnorr sighash over the spend transaction.
/// `keypairs` must contain at least `threshold` keypairs (for multisig) or
/// exactly one keypair (for timelock).
fn leaf_satisfier_elements(
    condition: &SpendCondition,
    sighash: &[u8; 32],
    keypairs: &[Keypair],
) -> Result<Vec<Vec<u8>>> {
    match condition {
        SpendCondition::MultiSig { threshold, .. } => {
            let k = *threshold as usize;
            if keypairs.len() < k {
                return Err(Error::Rpc(format!(
                    "MultiSig leaf needs {k} keypairs, got {}",
                    keypairs.len()
                )));
            }
            Ok(keypairs[..k]
                .iter()
                .map(|kp| schnorr_satisfier_sig(sighash, kp))
                .collect())
        }

        SpendCondition::TimelockHeight { .. } | SpendCondition::TimelockUnixSeconds { .. } => {
            if keypairs.is_empty() {
                return Err(Error::Rpc(
                    "Timelock leaf needs exactly 1 keypair, got 0".into(),
                ));
            }
            Ok(vec![schnorr_satisfier_sig(sighash, &keypairs[0])])
        }

        SpendCondition::All { .. } | SpendCondition::Any { .. } => Err(Error::CompileUnsupported(
            "leaf_satisfier_elements called with composite condition".into(),
        )),
    }
}

/// Build the satisfier elements for a branch (leaf or `All(leaves)`).
///
/// Returns the flat list of stack elements in **push order** — the order in
/// which they must appear in the P2SH `signature_script` (first element in the
/// returned Vec is pushed first, last is pushed last and therefore sits on top
/// when the redeem script executes).
///
/// ## Stack ordering for `All(leaves)`
///
/// The `All([leaf_0, leaf_1, …, leaf_n])` redeem script executes left-to-right:
/// `leaf_0` runs first and consumes the TOP of the stack. Therefore `leaf_0`'s
/// satisfier elements must be pushed LAST (so they sit on top). The push order
/// is the REVERSE of the leaf order:
///
/// ```text
/// push order: leaf_n satisfiers ... leaf_1 satisfiers leaf_0 satisfiers
/// stack top:  ← leaf_0 satisfiers (consumed first by the script)
/// ```
///
/// `keypairs` must still be supplied in leaf order (leaf_0's keypairs first)
/// for readability; this function reverses internally for the push order.
///
/// For an `All(leaves)` branch, `keypairs` must contain one contiguous group
/// per leaf, consumed left-to-right: the first `threshold_0` for leaf_0, the
/// next `threshold_1` for leaf_1, and so on. For a timelock leaf each group is
/// exactly 1 keypair; for a multisig leaf each group is `threshold` keypairs.
fn branch_satisfier_elements(
    branch: &SpendCondition,
    sighash: &[u8; 32],
    keypairs: &[Keypair],
) -> Result<Vec<Vec<u8>>> {
    match branch {
        SpendCondition::TimelockHeight { .. }
        | SpendCondition::TimelockUnixSeconds { .. }
        | SpendCondition::MultiSig { .. } => leaf_satisfier_elements(branch, sighash, keypairs),

        SpendCondition::All { children } => {
            // First, compute each leaf's satisfier elements in leaf order,
            // then reverse the per-leaf groups so that the last leaf's
            // elements are pushed first (deepest in stack) and the first
            // leaf's elements are pushed last (on top when the script runs).

            // Pass 1: compute groups in leaf order, tracking keypair offsets.
            let mut groups: Vec<Vec<Vec<u8>>> = Vec::new();
            let mut offset = 0usize;
            for child in children {
                let need = match child {
                    SpendCondition::MultiSig { threshold, .. } => *threshold as usize,
                    SpendCondition::TimelockHeight { .. }
                    | SpendCondition::TimelockUnixSeconds { .. } => 1,
                    SpendCondition::All { .. } | SpendCondition::Any { .. } => {
                        return Err(Error::CompileUnsupported(
                            "All branch contains a composite child".into(),
                        ))
                    }
                };
                let slice = keypairs.get(offset..offset + need).ok_or_else(|| {
                    Error::Rpc(format!(
                        "All branch needs {} keypairs starting at offset {}, only {} provided",
                        need,
                        offset,
                        keypairs.len()
                    ))
                })?;
                groups.push(leaf_satisfier_elements(child, sighash, slice)?);
                offset += need;
            }

            // Pass 2: emit in REVERSE group order (last leaf's elements are
            // pushed first; first leaf's elements sit on top of the stack).
            let mut elems = Vec::new();
            for group in groups.iter().rev() {
                elems.extend_from_slice(group);
            }
            Ok(elems)
        }

        SpendCondition::Any { .. } => Err(Error::CompileUnsupported(
            "nested Any inside Any branch: not supported".into(),
        )),
    }
}

/// Spend a vault locked under an `Any(2)` composite condition.
///
/// `condition` must be `SpendCondition::Any` with exactly 2 children (v0
/// limitation). The caller selects which branch to satisfy via `branch_index`:
///
/// - `branch_index = 0` → execute the `OP_IF` path (branch_a). The selector
///   pushed before the redeem script is `[0x01]` (truthy).
/// - `branch_index = 1` → execute the `OP_ELSE` path (branch_b). The selector
///   pushed before the redeem script is `[]` (empty / falsy).
///
/// `branch_keypairs` must satisfy the chosen branch's requirements:
/// - Multisig branch: `threshold` keypairs in key order.
/// - Timelock branch: exactly 1 keypair for the `controller_xonly` key.
/// - `All(leaves)` branch: keypairs for each leaf in order (see
///   [`branch_satisfier_elements`]).
///
/// `lock_time_if_timelock` must be provided when the chosen branch is a
/// timelock leaf or `All` whose first leaf is a timelock — it sets
/// `tx.lock_time` for `OP_CHECKLOCKTIMEVERIFY`. For purely multisig branches
/// it is ignored (pass `0`).
///
/// The spend is engine-preflighted before submission (see module docs).
///
/// # Errors
///
/// - [`Error::CompileUnsupported`] if `condition` is not `Any(2)`.
/// - [`Error::Rpc`] if the engine rejects the assembled spend or on node failure.
// Each parameter is a distinct, irreducible input.
#[allow(clippy::too_many_arguments)]
pub async fn spend_any_vault(
    client: &KaspaRpcClient,
    condition: &SpendCondition,
    branch_index: usize,
    vault_outpoint: (TransactionId, u32),
    branch_keypairs: &[Keypair],
    dest: &Address,
    prefix: Prefix,
    fee_sompi: u64,
    lock_time_if_timelock: u64,
) -> Result<String> {
    let children = match condition {
        SpendCondition::Any { children } if children.len() == 2 => children,
        SpendCondition::Any { children } => {
            return Err(Error::CompileUnsupported(format!(
                "spend_any_vault: Any with {} branches; v0 supports exactly 2",
                children.len()
            )))
        }
        _ => {
            return Err(Error::CompileUnsupported(
                "spend_any_vault: condition must be Any(2)".into(),
            ))
        }
    };

    if branch_index > 1 {
        return Err(Error::Rpc(format!(
            "spend_any_vault: branch_index {branch_index} out of range (0 or 1)"
        )));
    }

    let redeem = compile_condition_p2sh(condition)?;
    let chosen_branch = &children[branch_index];

    // OP_1 = `[0x01]` selects branch 0 (OP_IF side);
    // empty bytes `[]` selects branch 1 (OP_ELSE side).
    // The selector is the TOP element when OP_IF fires, so it is pushed LAST
    // in the satisfier list (before the redeem push).
    let selector: Vec<u8> = if branch_index == 0 {
        vec![0x01u8]
    } else {
        vec![]
    };

    // Does the chosen branch contain a CLTV leaf anywhere?
    let branch_needs_locktime = branch_needs_cltv(chosen_branch);

    if branch_needs_locktime {
        let sequence: u64 = 0; // non-final, required by CLTV
        spend_p2sh_tx_with_locktime(
            client,
            &redeem,
            vault_outpoint,
            dest,
            prefix,
            fee_sompi,
            false,
            lock_time_if_timelock,
            sequence,
            |sighash| {
                let mut elems = branch_satisfier_elements(chosen_branch, sighash, branch_keypairs)
                    .map_err(|e| kcp_common::error::Error::Rpc(format!("{e}")))?;
                elems.push(selector.clone());
                Ok(elems)
            },
        )
        .await
        .map_err(|e| Error::Rpc(format!("spend_any_vault(branch {branch_index}): {e}")))
    } else {
        spend_p2sh_tx(
            client,
            &redeem,
            vault_outpoint,
            dest,
            prefix,
            fee_sompi,
            false,
            |sighash| {
                let mut elems = branch_satisfier_elements(chosen_branch, sighash, branch_keypairs)
                    .map_err(|e| kcp_common::error::Error::Rpc(format!("{e}")))?;
                elems.push(selector.clone());
                Ok(elems)
            },
        )
        .await
        .map_err(|e| Error::Rpc(format!("spend_any_vault(branch {branch_index}): {e}")))
    }
}

/// Returns `true` if `condition` (or any of its leaf children in an `All`)
/// is a timelock leaf that requires CLTV — and therefore the spend transaction
/// must set `lock_time` and a non-final `sequence`.
fn branch_needs_cltv(condition: &SpendCondition) -> bool {
    match condition {
        SpendCondition::TimelockHeight { .. } | SpendCondition::TimelockUnixSeconds { .. } => true,
        SpendCondition::All { children } => children.iter().any(is_timelock),
        _ => false,
    }
}

// ── Composite All(leaves) spend ───────────────────────────────────────────────

/// Spend a vault locked under an `All(leaves)` composite condition.
///
/// `condition` must be `SpendCondition::All` whose children are all leaf
/// conditions (timelock or multisig). The v0 compiler only supports leaf
/// children in `All`.
///
/// `all_keypairs` must supply the keypairs for every leaf in left-to-right
/// order: the first group satisfies `children[0]`, the next group satisfies
/// `children[1]`, and so on. Each group is:
/// - 1 keypair for a timelock leaf.
/// - `threshold` keypairs (in key order) for a multisig leaf.
///
/// If any leaf is a timelock (`TimelockHeight` or `TimelockUnixSeconds`),
/// `lock_time` must be ≥ the largest deadline among all timelock leaves and
/// all timelocks must be of the same type (height or unix-seconds). Pass `0`
/// for purely multisig `All` conditions.
///
/// The spend is engine-preflighted before submission.
///
/// # Errors
///
/// - [`Error::CompileUnsupported`] if `condition` is not `All(leaves)`.
/// - [`Error::Rpc`] if the engine rejects the assembled spend or on node failure.
// Each parameter is a distinct, irreducible input.
#[allow(clippy::too_many_arguments)]
pub async fn spend_all_vault(
    client: &KaspaRpcClient,
    condition: &SpendCondition,
    vault_outpoint: (TransactionId, u32),
    all_keypairs: &[Keypair],
    dest: &Address,
    prefix: Prefix,
    fee_sompi: u64,
    lock_time: u64,
) -> Result<String> {
    let children = match condition {
        SpendCondition::All { children } => children,
        _ => {
            return Err(Error::CompileUnsupported(
                "spend_all_vault: condition must be All(leaves)".into(),
            ))
        }
    };

    let redeem = compile_condition_p2sh(condition)?;
    let any_cltv = children.iter().any(is_timelock);

    if any_cltv {
        let sequence: u64 = 0;
        spend_p2sh_tx_with_locktime(
            client,
            &redeem,
            vault_outpoint,
            dest,
            prefix,
            fee_sompi,
            false,
            lock_time,
            sequence,
            |sighash| {
                branch_satisfier_elements(condition, sighash, all_keypairs)
                    .map_err(|e| kcp_common::error::Error::Rpc(format!("{e}")))
            },
        )
        .await
        .map_err(|e| Error::Rpc(format!("spend_all_vault: {e}")))
    } else {
        spend_p2sh_tx(
            client,
            &redeem,
            vault_outpoint,
            dest,
            prefix,
            fee_sompi,
            false,
            |sighash| {
                branch_satisfier_elements(condition, sighash, all_keypairs)
                    .map_err(|e| kcp_common::error::Error::Rpc(format!("{e}")))
            },
        )
        .await
        .map_err(|e| Error::Rpc(format!("spend_all_vault: {e}")))
    }
}

/// Lock `value_sompi` under the compiled P2SH covenant for `condition`.
///
/// Compiles `condition` to a Kaspa script (using the P2SH-correct form for
/// timelock conditions — see module-level docs), wraps it in a P2SH locking
/// script, and funds that output from `wallet`. Returns the submitted
/// transaction id.
///
/// # Errors
///
/// Returns [`Error::CompileUnsupported`] if `condition` cannot be compiled (see
/// [`crate::script::compile_condition`]), or [`Error::Rpc`] on node failure.
pub async fn lock_vault_tx(
    client: &KaspaRpcClient,
    wallet: &Wallet,
    condition: &SpendCondition,
    value_sompi: u64,
) -> Result<String> {
    let redeem = p2sh_redeem_for(condition)?;
    lock_to_p2sh_tx(client, wallet, &redeem, value_sompi)
        .await
        .map_err(|e| Error::Rpc(format!("lock_vault_tx: {e}")))
}

/// Spend a multisig vault UTXO by providing `k` valid signatures.
///
/// `condition` must be a [`SpendCondition::MultiSig`] whose compiled script is
/// the one the vault was locked under. `signer_keypairs` must contain exactly
/// `condition.threshold` keypairs in the same relative order as their x-only
/// keys appear in `condition.xonly_keys` (the k-of-n signing subset must be
/// consecutive from the front of the key list, or use the first `k` keypairs
/// if signing all k keys in order).
///
/// The spend is engine-verified offline before submission. A wrong signature or
/// wrong key order will be rejected by the engine and the function will return
/// an error rather than submitting an invalid transaction.
///
/// # Signature order
///
/// Kaspa's `OP_CHECKMULTISIG` consumes signatures in the same order as their
/// corresponding public keys appear in the script. Pass `signer_keypairs` in
/// the same order as the keys are listed in `condition.xonly_keys`.
///
/// # Errors
///
/// - [`Error::CompileUnsupported`] if `condition` is not a `MultiSig`.
/// - [`Error::Rpc`] if the satisfier is rejected by the engine (wrong sig,
///   wrong order) or on any node failure.
// Parameters are each irreducible inputs to a multisig vault spend.
#[allow(clippy::too_many_arguments)]
pub async fn spend_multisig_vault(
    client: &KaspaRpcClient,
    condition: &SpendCondition,
    vault_outpoint: (TransactionId, u32),
    signer_keypairs: &[Keypair],
    dest: &Address,
    prefix: Prefix,
    fee_sompi: u64,
) -> Result<String> {
    // Reject non-MultiSig shapes explicitly, then compile the redeem with the
    // SAME compiler the lock path uses (`compile_condition_p2sh`, via
    // `p2sh_redeem_for`) — the P2SH address is the hash of these exact bytes,
    // so lock and spend must share one source of truth or value becomes
    // unspendable on any future divergence.
    if !matches!(condition, SpendCondition::MultiSig { .. }) {
        return Err(Error::CompileUnsupported(
            "spend_multisig_vault requires a MultiSig condition".to_string(),
        ));
    }
    let redeem = compile_condition_p2sh(condition)?;

    spend_p2sh_tx(
        client,
        &redeem,
        vault_outpoint,
        dest,
        prefix,
        fee_sompi,
        false, // CHECKMULTISIG validates identically with covenants on or off
        |sighash| {
            // Build one 65-byte sig per signer in key order.
            let sigs: Vec<Vec<u8>> = signer_keypairs
                .iter()
                .map(|kp| schnorr_satisfier_sig(sighash, kp))
                .collect();
            Ok(sigs)
        },
    )
    .await
    .map_err(|e| Error::Rpc(format!("spend_multisig_vault: {e}")))
}

/// Spend a timelock vault UTXO after the deadline has passed.
///
/// Works for both [`SpendCondition::TimelockHeight`] and
/// [`SpendCondition::TimelockUnixSeconds`]. The caller must supply a
/// `lock_time` value that satisfies `OP_CHECKLOCKTIMEVERIFY`:
///
/// - For `TimelockHeight`: `lock_time` must be `< 500_000_000_000` and `≥
///   condition.deadline`. Typically pass the current DAA score or higher.
/// - For `TimelockUnixSeconds`: `lock_time` must be `≥ 500_000_000_000` and
///   `≥ condition.deadline`. Pass the current unix timestamp or higher, but it
///   must remain above the 500_000_000_000 threshold.
///
/// The input sequence is set to 0 (non-final), which is required by
/// `OP_CHECKLOCKTIMEVERIFY`.
///
/// The spend is engine-verified offline before submission.
///
/// # Errors
///
/// - [`Error::CompileUnsupported`] if `condition` is not a timelock leaf.
/// - [`Error::Rpc`] if CLTV is not yet satisfied (lock_time < deadline) or
///   on any node failure.
// Parameters are each irreducible inputs to a timelock vault spend.
#[allow(clippy::too_many_arguments)]
pub async fn spend_timelock_vault(
    client: &KaspaRpcClient,
    condition: &SpendCondition,
    vault_outpoint: (TransactionId, u32),
    controller_keypair: &Keypair,
    dest: &Address,
    prefix: Prefix,
    fee_sompi: u64,
    lock_time: u64,
) -> Result<String> {
    // Use the P2SH-correct redeem (no OP_DROP for CLTV — see module docs).
    let redeem = p2sh_redeem_for(condition)?;
    // Sequence 0 = non-final; CLTV requires sequence != MAX_TX_IN_SEQUENCE_NUM.
    let sequence: u64 = 0;

    spend_p2sh_tx_with_locktime(
        client,
        &redeem,
        vault_outpoint,
        dest,
        prefix,
        fee_sompi,
        false,
        lock_time,
        sequence,
        |sighash| Ok(vec![schnorr_satisfier_sig(sighash, controller_keypair)]),
    )
    .await
    .map_err(|e| Error::Rpc(format!("spend_timelock_vault: {e}")))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::condition::SpendCondition;
    use crate::script::compile_condition;
    use kaspa_bip32::secp256k1::{Keypair, SECP256K1};
    use kaspa_consensus_core::{
        mass::SigopCount,
        subnets::SUBNETWORK_ID_NATIVE,
        tx::{
            ComputeCommit, ScriptPublicKey, ScriptVec, Transaction, TransactionId,
            TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
        },
    };
    use kcp_common::{
        p2sh::{
            build_p2sh_signature_script, p2sh_input_sighash, p2sh_lock_script,
            p2sh_redeem_sigop_count, schnorr_satisfier_sig, verify_p2sh_spend_offline,
        },
        tx::CARRIER_FEE_SOMPI,
    };

    /// Build a test keypair from a single repeated byte (deterministic, NOT secret).
    fn test_keypair(byte: u8) -> Keypair {
        Keypair::from_seckey_slice(SECP256K1, &[byte; 32]).unwrap()
    }

    /// A minimal destination `ScriptPublicKey` for test outputs (`OP_TRUE`).
    fn op_true_spk() -> ScriptPublicKey {
        ScriptPublicKey::new(0, ScriptVec::from_slice(&[0x51]))
    }

    /// Build a synthetic P2SH spend transaction and its corresponding UTXO entry.
    ///
    /// Returns `(tx, input_utxo_entry)` with input commitments set and a
    /// synthetic lock_time/sequence on the transaction and input respectively.
    fn build_spend_tx(
        redeem: &[u8],
        amount: u64,
        lock_time: u64,
        sequence: u64,
    ) -> (Transaction, UtxoEntry) {
        let p2sh_spk = p2sh_lock_script(redeem);
        let prev = TransactionOutpoint::new(TransactionId::from_slice(&[0xabu8; 32]), 0);
        let input = TransactionInput::new(prev, vec![], sequence, 0);
        let output = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, op_true_spk());
        let mut tx = Transaction::new(
            0,
            vec![input],
            vec![output],
            lock_time,
            SUBNETWORK_ID_NATIVE,
            0,
            vec![],
        );
        // set_input_commitments is private; replicate its effect via the public
        // helper path: the sighash computation inside p2sh.rs calls it before
        // computing hashes. For tests we use p2sh_input_sighash which requires
        // commitments to already be set. We set them via
        // kcp_common::p2sh's private path indirectly: re-use the same
        // ComputeCommit approach by calling set via the tx module.
        //
        // Compute the correct sigop count from the redeem script so the
        // engine's resource meter budget matches. For a 2-of-2 multisig the
        // count is 2; for a single-sig CHECKSIG it is 1; for a CLTV+CHECKSIG
        // redeem it is 1. Using SigopCount(1) for multisig would cause
        // "sig op count exceeds passed limit of 1".
        let sigops = p2sh_redeem_sigop_count(redeem);
        let commit: ComputeCommit = SigopCount(sigops).into();
        for inp in tx.inputs.iter_mut() {
            inp.compute_commit = commit;
        }
        let entry = UtxoEntry::new(amount, p2sh_spk, 0, false, None);
        (tx, entry)
    }

    // ── Multisig: 2-of-2 ─────────────────────────────────────────────────────

    /// Prove that a correctly assembled 2-of-2 multisig P2SH spend is accepted
    /// by the real rusty-kaspa script engine.
    ///
    /// Script: `<2> <pk1> <pk2> <2> OP_CHECKMULTISIG`
    /// Satisfier: `[sig1_for_pk1, sig2_for_pk2]` in key order, no dummy element.
    #[test]
    fn multisig_2of2_lock_spend_executes_on_engine() {
        let kp1 = test_keypair(0x11);
        let kp2 = test_keypair(0x22);
        let pk1 = kp1.x_only_public_key().0.serialize();
        let pk2 = kp2.x_only_public_key().0.serialize();

        let condition = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![pk1, pk2],
        };
        let redeem = compile_condition(&condition).expect("compile_condition failed");

        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, 0, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        // Signatures in key order: sig for pk1 first, sig for pk2 second.
        // No leading dummy element — Kaspa's CHECKMULTISIG does not consume one.
        let sig1 = schnorr_satisfier_sig(&sighash, &kp1);
        let sig2 = schnorr_satisfier_sig(&sighash, &kp2);
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig1, sig2], &redeem).unwrap();

        verify_p2sh_spend_offline(&tx, 0, &entry, false)
            .expect("engine must accept a valid 2-of-2 multisig P2SH spend");
    }

    /// Fund-safety invariant: the evaluator compiler (`compile_condition`) and
    /// the P2SH compiler (`compile_condition_p2sh`, used by BOTH the lock and
    /// spend paths) must emit byte-identical multisig redeem scripts. The P2SH
    /// address is the hash of these exact bytes — if the compilers ever
    /// diverge, historic locks become unspendable. This test fails CI on any
    /// such divergence.
    #[test]
    fn multisig_redeem_identical_across_compilers() {
        let pks: Vec<[u8; 32]> = (1u8..=3)
            .map(|b| test_keypair(b).x_only_public_key().0.serialize())
            .collect();
        for (threshold, n) in [(1u8, 1usize), (2, 2), (2, 3), (3, 3)] {
            let condition = SpendCondition::MultiSig {
                threshold,
                xonly_keys: pks[..n].to_vec(),
            };
            let evaluator = compile_condition(&condition).expect("compile_condition");
            let p2sh = compile_condition_p2sh(&condition).expect("compile_condition_p2sh");
            let lock = p2sh_redeem_for(&condition).expect("p2sh_redeem_for");
            assert_eq!(
                evaluator, p2sh,
                "{threshold}-of-{n}: evaluator vs P2SH multisig redeem diverged"
            );
            assert_eq!(
                p2sh, lock,
                "{threshold}-of-{n}: lock-path redeem diverged from spend-path redeem"
            );
        }
    }

    /// Prove that a 2-of-2 spend with one wrong signature is rejected by the engine.
    #[test]
    fn multisig_2of2_wrong_sig_rejected_by_engine() {
        let kp1 = test_keypair(0x11);
        let kp2 = test_keypair(0x22);
        let wrong = test_keypair(0x33); // does not correspond to any listed key
        let pk1 = kp1.x_only_public_key().0.serialize();
        let pk2 = kp2.x_only_public_key().0.serialize();

        let condition = SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![pk1, pk2],
        };
        let redeem = compile_condition(&condition).expect("compile_condition failed");

        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, 0, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig1 = schnorr_satisfier_sig(&sighash, &kp1);
        let sig_wrong = schnorr_satisfier_sig(&sighash, &wrong); // wrong key for slot 2
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig1, sig_wrong], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject a 2-of-2 spend where one signature is from the wrong key"
        );
    }

    // ── Timelock: height-based CLTV ───────────────────────────────────────────

    /// `LOCK_TIME_THRESHOLD` from rusty-kaspa — values below this are DAA
    /// heights; values at or above are unix-second timestamps.
    const LOCK_TIME_THRESHOLD: u64 = 500_000_000_000;

    /// Prove that a CLTV-timelock (height-based) spend with tx.lock_time >=
    /// deadline is accepted by the real engine.
    ///
    /// Deadline is in the past (deadline=1000, tx.lock_time=2000) and below
    /// the unix-seconds threshold so the engine treats it as a DAA-height CLTV.
    #[test]
    fn timelock_height_cltv_valid_locktime_accepted_by_engine() {
        let kp = test_keypair(0x55);
        let controller_xonly = kp.x_only_public_key().0.serialize();

        // Deadline well below the unix-seconds threshold (height-based CLTV).
        let deadline: u64 = 1_000;
        // tx.lock_time >= deadline AND same side of threshold as deadline.
        let tx_lock_time: u64 = 2_000; // >= 1_000, < LOCK_TIME_THRESHOLD
        assert!(
            tx_lock_time < LOCK_TIME_THRESHOLD,
            "tx_lock_time must be a height (< threshold)"
        );
        assert!(deadline < LOCK_TIME_THRESHOLD, "deadline must be a height");
        assert!(tx_lock_time >= deadline, "tx.lock_time must satisfy CLTV");

        // Use the P2SH-correct form (no OP_DROP — Kaspa CLTV pops the deadline).
        let redeem = super::compile_timelock_p2sh_redeem(deadline, &controller_xonly)
            .expect("compile_timelock_p2sh_redeem failed");

        // Sequence must be non-final for CLTV. Use 0 (not 0xffff...ffff).
        let sequence: u64 = 0;
        let amount = 100_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, sequence);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &kp);
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig], &redeem).unwrap();

        verify_p2sh_spend_offline(&tx, 0, &entry, false)
            .expect("engine must accept CLTV spend when tx.lock_time >= height deadline");
    }

    /// Prove that a CLTV spend where tx.lock_time < deadline is rejected.
    ///
    /// Uses the P2SH-correct redeem (`compile_timelock_p2sh_redeem`, no OP_DROP).
    #[test]
    fn timelock_height_cltv_locktime_too_low_rejected_by_engine() {
        let kp = test_keypair(0x55);
        let controller_xonly = kp.x_only_public_key().0.serialize();

        let deadline: u64 = 5_000;
        let tx_lock_time: u64 = 4_999; // < deadline — CLTV not satisfied

        // Use the P2SH-correct form (no OP_DROP — Kaspa CLTV pops the deadline).
        let redeem = super::compile_timelock_p2sh_redeem(deadline, &controller_xonly)
            .expect("compile_timelock_p2sh_redeem failed");

        let sequence: u64 = 0;
        let amount = 100_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, sequence);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &kp);
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject CLTV spend when tx.lock_time < deadline"
        );
    }

    // ── Timelock: unix-seconds CLTV ───────────────────────────────────────────

    /// Prove that a unix-seconds CLTV spend with tx.lock_time >= deadline is
    /// accepted by the real engine.
    ///
    /// Both deadline and tx.lock_time must be >= LOCK_TIME_THRESHOLD.
    #[test]
    fn timelock_unix_seconds_cltv_valid_locktime_accepted_by_engine() {
        let kp = test_keypair(0x66);
        let controller_xonly = kp.x_only_public_key().0.serialize();

        // A past unix-timestamp deadline: 1 hour before an arbitrary reference
        // point above the threshold. Both values must be >= LOCK_TIME_THRESHOLD.
        let deadline: u64 = LOCK_TIME_THRESHOLD + 3_600; // "now - 3600s" scenario
        let tx_lock_time: u64 = LOCK_TIME_THRESHOLD + 7_200; // >= deadline
        assert!(deadline >= LOCK_TIME_THRESHOLD);
        assert!(tx_lock_time >= deadline);

        // Use the P2SH-correct form (no OP_DROP — Kaspa CLTV pops the deadline).
        let redeem = super::compile_timelock_p2sh_redeem(deadline, &controller_xonly)
            .expect("compile_timelock_p2sh_redeem failed");

        let sequence: u64 = 0;
        let amount = 100_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, sequence);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &kp);
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig], &redeem).unwrap();

        verify_p2sh_spend_offline(&tx, 0, &entry, false)
            .expect("engine must accept unix-seconds CLTV spend when tx.lock_time >= deadline");
    }

    /// Prove that a unix-seconds CLTV spend with tx.lock_time < deadline is rejected.
    ///
    /// Uses the P2SH-correct redeem (`compile_timelock_p2sh_redeem`, no OP_DROP).
    #[test]
    fn timelock_unix_seconds_cltv_future_deadline_rejected_by_engine() {
        let kp = test_keypair(0x66);
        let controller_xonly = kp.x_only_public_key().0.serialize();

        // Deadline is in the future (tx.lock_time < deadline).
        let tx_lock_time: u64 = LOCK_TIME_THRESHOLD + 3_600;
        let deadline: u64 = LOCK_TIME_THRESHOLD + 7_200; // deadline > tx_lock_time
        assert!(tx_lock_time < deadline);

        // Use the P2SH-correct form (no OP_DROP — Kaspa CLTV pops the deadline).
        let redeem = super::compile_timelock_p2sh_redeem(deadline, &controller_xonly)
            .expect("compile_timelock_p2sh_redeem failed");

        let sequence: u64 = 0;
        let amount = 100_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, sequence);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &kp);
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject unix-seconds CLTV spend when tx.lock_time < deadline"
        );
    }

    /// Prove that a unix-seconds CLTV spend with a mismatched locktime type is
    /// rejected (deadline is unix-seconds but tx.lock_time is a height value).
    ///
    /// Uses the P2SH-correct redeem (`compile_timelock_p2sh_redeem`, no OP_DROP).
    #[test]
    fn timelock_unix_seconds_cltv_type_mismatch_rejected_by_engine() {
        let kp = test_keypair(0x66);
        let controller_xonly = kp.x_only_public_key().0.serialize();

        let deadline: u64 = LOCK_TIME_THRESHOLD + 3_600; // unix-seconds
                                                         // Mismatch: script has unix-seconds (>= threshold) but tx.lock_time
                                                         // is a DAA-height value (< threshold). The engine must reject.
        let tx_lock_time_height: u64 = 9_999;

        let redeem = super::compile_timelock_p2sh_redeem(deadline, &controller_xonly)
            .expect("compile_timelock_p2sh_redeem failed");

        let sequence: u64 = 0;
        let amount = 100_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time_height, sequence);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &kp);
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject a locktime-type mismatch (unix-seconds script vs height tx)"
        );
    }

    // ── Composite Any(timelockA, multisigB): offline engine tests ─────────────
    //
    // Canonical test fixture:
    //   branch 0 (OP_IF side):  TimelockUnixSeconds { deadline, key_tl }
    //   branch 1 (OP_ELSE side): MultiSig { 2-of-2: key_m1, key_m2 }
    //
    // Redeem layout:
    //   OP_IF
    //     <deadline> OP_CLTV <pk_tl> OP_CHECKSIG
    //   OP_ELSE
    //     <2> <pk_m1> <pk_m2> <2> OP_CHECKMULTISIG
    //   OP_ENDIF
    //
    // Satisfier layout for branch 0:
    //   <sig_tl> [0x01] <redeem>
    //
    // Satisfier layout for branch 1:
    //   <sig_m1> <sig_m2> [] <redeem>

    /// Build the canonical Any(timelock, multisig) condition used by the composite tests.
    fn any_tl_multisig_condition() -> SpendCondition {
        let kp_tl = test_keypair(0xAA);
        let kp_m1 = test_keypair(0xBB);
        let kp_m2 = test_keypair(0xCC);

        SpendCondition::Any {
            children: vec![
                SpendCondition::TimelockUnixSeconds {
                    deadline: LOCK_TIME_THRESHOLD + 3_600, // unix-seconds past deadline
                    controller_xonly: kp_tl.x_only_public_key().0.serialize(),
                },
                SpendCondition::MultiSig {
                    threshold: 2,
                    xonly_keys: vec![
                        kp_m1.x_only_public_key().0.serialize(),
                        kp_m2.x_only_public_key().0.serialize(),
                    ],
                },
            ],
        }
    }

    /// Prove that Any(timelock, multisig) via branch 0 (timelock) with a
    /// valid lock_time and signature is accepted by the engine.
    ///
    /// Satisfier: [sig_tl, selector=0x01, redeem]
    #[test]
    fn any_composite_branch0_timelock_valid_accepted_by_engine() {
        let kp_tl = test_keypair(0xAA);
        let condition = any_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        // tx.lock_time >= deadline; sequence = 0 (non-final)
        let deadline: u64 = LOCK_TIME_THRESHOLD + 3_600;
        let tx_lock_time = LOCK_TIME_THRESHOLD + 7_200; // > deadline
        assert!(tx_lock_time >= deadline, "tx_lock_time must satisfy CLTV");
        let sequence: u64 = 0;
        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, sequence);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_tl = schnorr_satisfier_sig(&sighash, &kp_tl);

        // satisfier elements: [sig_tl, selector=0x01], then redeem
        let selector = vec![0x01u8]; // truthy → OP_IF → branch 0
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_tl, selector], &redeem).unwrap();

        verify_p2sh_spend_offline(&tx, 0, &entry, false)
            .expect("engine must accept Any branch-0 timelock spend with valid lock_time and sig");
    }

    /// Prove that Any(timelock, multisig) via branch 1 (multisig) with 2
    /// valid signatures is accepted by the engine.
    ///
    /// Satisfier: [sig_m1, sig_m2, selector=[], redeem]
    #[test]
    fn any_composite_branch1_multisig_valid_accepted_by_engine() {
        let kp_m1 = test_keypair(0xBB);
        let kp_m2 = test_keypair(0xCC);
        let condition = any_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        // Multisig branch does not need CLTV; lock_time = 0, sequence = 0.
        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, 0, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_m1 = schnorr_satisfier_sig(&sighash, &kp_m1);
        let sig_m2 = schnorr_satisfier_sig(&sighash, &kp_m2);

        // satisfier elements: [sig_m1, sig_m2, selector=empty], then redeem
        let selector = vec![]; // falsy → OP_ELSE → branch 1
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_m1, sig_m2, selector], &redeem).unwrap();

        verify_p2sh_spend_offline(&tx, 0, &entry, false)
            .expect("engine must accept Any branch-1 multisig spend with 2 valid sigs");
    }

    /// Prove that branch-0 (timelock) with the wrong selector (OP_0 / empty)
    /// causes the engine to execute branch-1 code against timelock satisfier
    /// elements — and reject.
    #[test]
    fn any_composite_wrong_selector_rejected_by_engine() {
        let kp_tl = test_keypair(0xAA);
        let condition = any_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        let tx_lock_time = LOCK_TIME_THRESHOLD + 7_200;
        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_tl = schnorr_satisfier_sig(&sighash, &kp_tl);

        // Wrong: use branch-1 selector (empty) with branch-0 satisfier (1 sig).
        // Engine will execute multisig branch with a single sig — must reject.
        let wrong_selector = vec![]; // goes to OP_ELSE (multisig branch)
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_tl, wrong_selector], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject wrong-selector: branch-0 satisfier fed to branch-1 code"
        );
    }

    /// Prove that branch-1 (multisig) with only one sig (not 2-of-2) is rejected.
    #[test]
    fn any_composite_branch1_missing_sig_rejected_by_engine() {
        let kp_m1 = test_keypair(0xBB);
        let condition = any_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, 0, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_m1 = schnorr_satisfier_sig(&sighash, &kp_m1);

        // Only one sig for a 2-of-2 multisig: the remaining stack element will
        // be wrong when CHECKMULTISIG pops k=2 sigs.
        let selector = vec![];
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_m1, selector], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject branch-1 spend with only 1 of 2 required signatures"
        );
    }

    /// Prove that branch-0 (timelock) with lock_time below deadline is rejected.
    #[test]
    fn any_composite_branch0_locktime_too_low_rejected_by_engine() {
        let kp_tl = test_keypair(0xAA);
        let condition = any_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        let deadline = LOCK_TIME_THRESHOLD + 3_600;
        let tx_lock_time = LOCK_TIME_THRESHOLD + 1_000; // < deadline — CLTV will fail
        assert!(tx_lock_time < deadline);
        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_tl = schnorr_satisfier_sig(&sighash, &kp_tl);
        let selector = vec![0x01u8];
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_tl, selector], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject Any branch-0 timelock when tx.lock_time < deadline"
        );
    }

    // ── Composite All(timelock, multisig): offline engine tests ───────────────
    //
    // Fixture: All([TimelockUnixSeconds{deadline, key_tl}, MultiSig{1-of-1: key_m1}])
    //
    // Redeem layout:
    //   <deadline> OP_CLTV <pk_tl> OP_CHECKSIG OP_VERIFY
    //   <1> <pk_m1> <1> OP_CHECKMULTISIG
    //
    // Satisfier layout (both leaves must be satisfied):
    //   <sig_tl> <sig_m1> <redeem>

    fn all_tl_multisig_condition() -> SpendCondition {
        let kp_tl = test_keypair(0xDD);
        let kp_m1 = test_keypair(0xEE);

        SpendCondition::All {
            children: vec![
                SpendCondition::TimelockUnixSeconds {
                    deadline: LOCK_TIME_THRESHOLD + 3_600,
                    controller_xonly: kp_tl.x_only_public_key().0.serialize(),
                },
                SpendCondition::MultiSig {
                    threshold: 1,
                    xonly_keys: vec![kp_m1.x_only_public_key().0.serialize()],
                },
            ],
        }
    }

    /// Prove that All(timelock, multisig) with all satisfiers valid and a
    /// sufficient lock_time is accepted by the engine.
    ///
    /// Redeem script (leaf order): `<deadline> OP_CLTV <pk_tl> OP_CHECKSIG OP_VERIFY
    ///                              <1> <pk_m1> <1> OP_CHECKMULTISIG`
    ///
    /// Satisfier push order (REVERSE of leaf order, so leaf_0/tl is on top):
    ///   push sig_m1 first (deepest), push sig_tl last (on top)
    ///   → signature_script: [sig_m1, sig_tl, redeem]
    ///
    /// Execution trace:
    ///   stack starts: sig_m1 | sig_tl
    ///   <deadline> pushed       → sig_m1 | sig_tl | deadline
    ///   OP_CLTV pops deadline   → sig_m1 | sig_tl
    ///   <pk_tl> pushed          → sig_m1 | sig_tl | pk_tl
    ///   OP_CHECKSIG pops pk_tl, sig_tl → verifies → sig_m1 | true
    ///   OP_VERIFY pops true     → sig_m1
    ///   <1><pk_m1><1> pushed    → sig_m1 | 1 | pk_m1 | 1
    ///   OP_CHECKMULTISIG pops n=1, pk_m1, k=1, sig_m1 → verifies → true
    #[test]
    fn all_composite_all_satisfiers_valid_accepted_by_engine() {
        let kp_tl = test_keypair(0xDD);
        let kp_m1 = test_keypair(0xEE);
        let condition = all_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        let tx_lock_time = LOCK_TIME_THRESHOLD + 7_200; // >= deadline
        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_tl = schnorr_satisfier_sig(&sighash, &kp_tl);
        let sig_m1 = schnorr_satisfier_sig(&sighash, &kp_m1);

        // Push order: sig_m1 (leaf_1, deepest) then sig_tl (leaf_0, on top).
        // Reverse of leaf order because the redeem script consumes leaf_0 first.
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_m1, sig_tl], &redeem).unwrap();

        verify_p2sh_spend_offline(&tx, 0, &entry, false)
            .expect("engine must accept All(tl, multisig) when all satisfiers are valid");
    }

    /// Prove that All(timelock, multisig) with the multisig sig missing is rejected.
    #[test]
    fn all_composite_missing_multisig_satisfier_rejected_by_engine() {
        let kp_tl = test_keypair(0xDD);
        let condition = all_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        let tx_lock_time = LOCK_TIME_THRESHOLD + 7_200;
        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_tl = schnorr_satisfier_sig(&sighash, &kp_tl);

        // Only the timelock sig; multisig leaf has no satisfier.
        // The engine will try to CHECKSIG for the multisig with whatever is
        // on the stack after the timelock passes — and reject.
        tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig_tl], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject All(tl, multisig) when multisig satisfier is missing"
        );
    }

    /// Prove that All(timelock, multisig) with the timelock lock_time too low
    /// is rejected even when both signatures are valid.
    ///
    /// Uses the correct reverse-push order (sig_m1 first, sig_tl on top), so
    /// the sigs are correctly presented but CLTV itself rejects the spend.
    #[test]
    fn all_composite_timelock_not_satisfied_rejected_by_engine() {
        let kp_tl = test_keypair(0xDD);
        let kp_m1 = test_keypair(0xEE);
        let condition = all_tl_multisig_condition();

        let redeem =
            super::compile_condition_p2sh(&condition).expect("compile_condition_p2sh failed");

        let deadline = LOCK_TIME_THRESHOLD + 3_600;
        let tx_lock_time = LOCK_TIME_THRESHOLD + 1_000; // < deadline
        assert!(tx_lock_time < deadline);
        let amount = 200_000_000u64;
        let (mut tx, entry) = build_spend_tx(&redeem, amount, tx_lock_time, 0);

        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
        let sig_tl = schnorr_satisfier_sig(&sighash, &kp_tl);
        let sig_m1 = schnorr_satisfier_sig(&sighash, &kp_m1);

        // Correct push order (sig_m1 first, sig_tl on top) — the CLTV
        // deadline is not met, so the engine must reject despite correct sigs.
        tx.inputs[0].signature_script =
            build_p2sh_signature_script(&[sig_m1, sig_tl], &redeem).unwrap();

        assert!(
            verify_p2sh_spend_offline(&tx, 0, &entry, false).is_err(),
            "engine must reject All(tl, multisig) when CLTV deadline is not met"
        );
    }
}
