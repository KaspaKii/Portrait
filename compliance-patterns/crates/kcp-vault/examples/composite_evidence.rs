//! On-chain evidence: lock value under a composite `Any(timelock, multisig)` P2SH
//! covenant and spend it via the chosen branch — consensus-enforced branch
//! selection.
//!
//! Demonstrates the full lock→spend cycle for an `Any(2)` composite vault:
//!
//! - Branch 0: `TimelockUnixSeconds { past_deadline, key0 }` — spendable by
//!   `key0` after the deadline has passed.
//! - Branch 1: `MultiSig { 2-of-2: key0, key1 }` — spendable by both keys at
//!   any time.
//!
//! This example spends via **branch 1 (multisig)** — both signatures — back to
//! the source wallet, demonstrating consensus-enforced branch selection at the
//! script level.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_NET_SUFFIX=10                    \
//! KCP_KEY_FILE=/path/to/wallet.key     \
//! KCP_NEXT_KEY_FILE=/path/to/wallet2.key  \  # optional; falls back to index 1 of key0
//! cargo run -p kcp-vault --example composite_evidence --features wrpc
//! ```
//!
//! **Testnet only.** Refuses to run unless the node reports a testnet network.
//! No hardcoded private keys.
//!
//! ## Evidence block (KCP-VT-003)
//!
//! On success the example prints a FACTS-ready KCP-VT-003 block proving:
//! "composite Any(timelock, multisig) vault: value released by satisfying the
//! chosen branch on-chain — consensus-enforced branch selection."
//!
//! Status: **v1 — unaudited — testnet first.**

use std::env;

use kaspa_consensus_core::tx::TransactionId;
use kcp_common::{
    tx::CARRIER_FEE_SOMPI,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_vault::{
    condition::SpendCondition,
    onchain::{compile_condition_p2sh, lock_vault_tx, spend_any_vault},
};

/// Value locked under the composite P2SH redeem script (1 KAS).
const LOCK_VALUE_SOMPI: u64 = 100_000_000;

/// `LOCK_TIME_THRESHOLD` from rusty-kaspa: values below this are DAA heights;
/// values at or above are unix-second timestamps.
const LOCK_TIME_THRESHOLD: u64 = 500_000_000_000;

type BoxError = Box<dyn std::error::Error>;

/// Transient errors that warrant a confirmation-poll retry.
fn is_transient<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("not found") || s.contains("already spent") || s.contains("in the mempool")
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // ── environment ──────────────────────────────────────────────────────────
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://localhost:17210".to_string());
    let key_file = env::var("KCP_KEY_FILE").map_err(|_| "KCP_KEY_FILE is required")?;
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    // ── node connection ───────────────────────────────────────────────────────
    let node = NodeClient::new(NodeConfig::testnet(&node_url, net_suffix));
    let rpc = node.rpc().await?;
    let info = node.server_info().await?;
    println!(
        "connected: server={} network={} synced={} daa={}",
        info.server_version, info.network_id, info.is_synced, info.virtual_daa_score
    );
    if !info.network_id.contains("testnet") {
        return Err(format!("REFUSED: network_id '{}' is not testnet", info.network_id).into());
    }

    // ── load two DISTINCT signers ────────────────────────────────────────────
    // wallet0 (funded) locks the vault and holds the timelock key.
    // wallet1 is the second 2-of-2 multisig signer.
    let wallet0 = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("load wallet index 0: {e}"))?;
    let wallet1 = match env::var("KCP_NEXT_KEY_FILE").ok() {
        Some(path) => Wallet::load(std::path::Path::new(&path), 0, Prefix::Testnet)
            .map_err(|e| format!("load next-key wallet: {e}"))?,
        None => Wallet::load(std::path::Path::new(&key_file), 1, Prefix::Testnet)
            .map_err(|e| format!("load wallet index 1: {e}"))?,
    };

    let pk0 = wallet0.keypair.x_only_public_key().0.serialize();
    let pk1 = wallet1.keypair.x_only_public_key().0.serialize();

    println!("wallet[0]:  {}", wallet0.address_string());
    println!("wallet[1]:  {}", wallet1.address_string());
    println!("pk[0] (hex): {}", hex::encode(pk0));
    println!("pk[1] (hex): {}", hex::encode(pk1));

    // ── build Any(timelock, multisig) composite condition ────────────────────
    // Branch 0 (OP_IF): TimelockUnixSeconds in the past — key0 can spend.
    // Branch 1 (OP_ELSE): 2-of-2 MultiSig — both keys must sign.
    // We spend via branch 1 (multisig) to demonstrate branch-selection.
    let past_deadline: u64 = LOCK_TIME_THRESHOLD + 3_600; // unix-seconds; well in the past
    let condition = SpendCondition::Any {
        children: vec![
            SpendCondition::TimelockUnixSeconds {
                deadline: past_deadline,
                controller_xonly: pk0,
            },
            SpendCondition::MultiSig {
                threshold: 2,
                xonly_keys: vec![pk0, pk1],
            },
        ],
    };
    condition
        .validate()
        .map_err(|e| format!("condition invalid: {e}"))?;

    println!("\n--- CONDITION ---");
    println!("{}", serde_json::to_string_pretty(&condition)?);

    // Print the compiled redeem opcode layout for transparency.
    let redeem =
        compile_condition_p2sh(&condition).map_err(|e| format!("compile_condition_p2sh: {e}"))?;
    println!(
        "\nredeem script ({} bytes): {}",
        redeem.len(),
        hex::encode(&redeem)
    );
    println!(
        "layout: OP_IF <deadline> OP_CLTV <pk0> OP_CHECKSIG OP_ELSE \
         <2> <pk0> <pk1> <2> OP_CHECKMULTISIG OP_ENDIF"
    );

    // ── LOCK ─────────────────────────────────────────────────────────────────
    println!("\n--- LOCK value under P2SH composite Any(timelock, multisig) covenant ---");
    let lock_tx_id = lock_vault_tx(&rpc, &wallet0, &condition, LOCK_VALUE_SOMPI)
        .await
        .map_err(|e| format!("lock_vault_tx: {e}"))?;
    println!("lock tx_id: {lock_tx_id}");
    println!("lookup:     tx {lock_tx_id} on {}", info.network_id);
    println!(
        "note: value is locked under a real P2SH composite covenant; \
         it can only be released by satisfying the timelock branch or \
         providing 2 valid Schnorr signatures."
    );

    // ── SPEND via branch 1 (multisig) ────────────────────────────────────────
    let lock_txid: TransactionId = lock_tx_id
        .parse()
        .map_err(|e| format!("parse tx_id: {e}"))?;

    println!("\n--- SPEND P2SH composite via branch 1 (2-of-2 multisig) ---");
    println!(
        "selector: [] (empty/OP_0 = falsy → OP_ELSE → multisig branch)\n\
         satisfier order: [sig_pk0, sig_pk1, selector=[], redeem]\n\
         note: spend_any_vault runs the real script engine offline before submit."
    );

    let mut spend_tx_id = String::new();
    for attempt in 1..=40u32 {
        let result = spend_any_vault(
            &rpc,
            &condition,
            1, // branch_index = 1 → OP_ELSE → multisig branch
            (lock_txid, 0),
            &[wallet0.keypair, wallet1.keypair],
            &wallet0.address,
            Prefix::Testnet,
            CARRIER_FEE_SOMPI,
            0, // lock_time_if_timelock: not needed for multisig branch
        )
        .await;
        match result {
            Ok(id) => {
                spend_tx_id = id;
                break;
            }
            Err(e) if is_transient(&e) && attempt < 40 => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => return Err(format!("spend_any_vault attempt {attempt}: {e}").into()),
        }
    }
    if spend_tx_id.is_empty() {
        return Err("composite P2SH UTXO not confirmed after 80s".into());
    }
    println!("spend tx_id: {spend_tx_id}");
    println!("lookup:      tx {spend_tx_id} on {}", info.network_id);
    println!(
        "engine preflight: PASSED (real script engine accepted the Any branch-1 multisig \
         spend before submit — consensus-enforced branch selection)"
    );

    // ── FACTS-ready: KCP-VT-003 ───────────────────────────────────────────────
    println!("\n--- FACTS.yaml-ready: KCP-VT-003 ---");
    println!("  id: KCP-VT-003");
    println!(
        "  claim: \"composite Any(timelock, multisig) vault: value released by satisfying \
         the chosen branch on-chain — consensus-enforced branch selection. \
         network={} lock_tx={lock_tx_id} spend_tx={spend_tx_id}\"",
        info.network_id
    );
    println!("  source: crates/kcp-vault/examples/composite_evidence.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!(
        "  note: v1 — unaudited — Any(TimelockUnixSeconds, MultiSig 2-of-2) P2SH \
         lock+spend via multisig branch; engine-preflighted; selector=[] (OP_0/falsy) \
         routes to OP_ELSE branch; satisfier=[sig0,sig1,selector,redeem]. \
         All(leaves) on-chain spend also implemented (spend_all_vault, engine-proved \
         by offline tests). Composite redeem: OP_IF <tl> OP_ELSE <multisig> OP_ENDIF."
    );

    Ok(())
}
