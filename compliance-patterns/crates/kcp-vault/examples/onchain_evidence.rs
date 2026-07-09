//! On-chain evidence: lock value under a real P2SH multisig covenant and spend
//! it by satisfying the script (consensus-enforced, not just digest-anchored).
//!
//! Demonstrates the full lock→spend cycle for a 2-of-2 multisig vault using
//! wallet keys at BIP-44 indices 0 and 1 from the same `KCP_KEY_FILE`. The
//! vault value (1 KAS) is locked under `compile_condition(MultiSig{2,[pk0,pk1]})`,
//! then spent back to the wallet with both signatures.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_NET_SUFFIX=10                    \
//! KCP_KEY_FILE=/path/to/wallet.key     \
//! cargo run -p kcp-vault --example onchain_evidence --features wrpc
//! ```
//!
//! **Testnet only.** Refuses to run unless the node reports a testnet network.
//! No hardcoded private keys.
//!
//! ## Evidence block (KCP-VT-002)
//!
//! On success the example prints a FACTS-ready KCP-VT-002 block proving:
//! "vault value locked under a real multisig covenant script (P2SH) and spent
//! by satisfying it — consensus-enforced, not just digest-anchored."
//!
//! Status: **v1 — unaudited — testnet first.**

use std::env;

use kcp_common::{
    tx::CARRIER_FEE_SOMPI,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_vault::{
    condition::SpendCondition,
    onchain::{lock_vault_tx, spend_multisig_vault},
};

use kaspa_consensus_core::tx::TransactionId;

/// Value locked under the multisig P2SH redeem script (1 KAS).
const LOCK_VALUE_SOMPI: u64 = 100_000_000;

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

    // ── load two DISTINCT multisig signers ───────────────────────────────────
    // wallet0 (funded) locks the vault; wallet1 is the second 2-of-2 signer.
    // A raw-hex key file ignores the derivation index (so index 0 and 1 are
    // identical) — use KCP_NEXT_KEY_FILE for a distinct second key when set,
    // otherwise derive index 1 (valid for a mnemonic key file).
    let wallet0 = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("load wallet index 0: {e}"))?;
    let wallet1 = match std::env::var("KCP_NEXT_KEY_FILE").ok() {
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

    // ── build 2-of-2 multisig condition ──────────────────────────────────────
    let condition = SpendCondition::MultiSig {
        threshold: 2,
        xonly_keys: vec![pk0, pk1],
    };
    condition
        .validate()
        .map_err(|e| format!("condition invalid: {e}"))?;

    println!("\n--- CONDITION ---");
    println!("{}", serde_json::to_string_pretty(&condition)?);

    // ── LOCK ─────────────────────────────────────────────────────────────────
    // lock_vault_tx compiles the condition to a real script, wraps it in P2SH,
    // and funds the output from wallet[0].
    println!("\n--- LOCK value under P2SH multisig covenant ---");
    let lock_tx_id = lock_vault_tx(&rpc, &wallet0, &condition, LOCK_VALUE_SOMPI)
        .await
        .map_err(|e| format!("lock_vault_tx: {e}"))?;
    println!("lock tx_id: {lock_tx_id}");
    println!("lookup:     tx {lock_tx_id} on {}", info.network_id);
    println!(
        "note:       value is now locked under a real P2SH covenant script; \
         it can only be released by providing 2 valid Schnorr signatures."
    );

    // ── SPEND ─────────────────────────────────────────────────────────────────
    // The P2SH UTXO (output 0 of the lock tx) must be block-confirmed before
    // it can be spent. Poll with retries.
    let lock_txid: TransactionId = lock_tx_id
        .parse()
        .map_err(|e| format!("parse tx_id: {e}"))?;

    println!("\n--- SPEND P2SH (satisfy 2-of-2 multisig) ---");
    println!(
        "note: spend_multisig_vault runs the real script engine offline before submit; \
         the engine must accept before any RPC call is made."
    );

    let mut spend_tx_id = String::new();
    for attempt in 1..=40u32 {
        let result = spend_multisig_vault(
            &rpc,
            &condition,
            (lock_txid, 0),
            &[wallet0.keypair, wallet1.keypair],
            &wallet0.address,
            Prefix::Testnet,
            CARRIER_FEE_SOMPI,
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
            Err(e) => return Err(format!("spend_multisig_vault (attempt {attempt}): {e}").into()),
        }
    }
    if spend_tx_id.is_empty() {
        return Err("P2SH multisig UTXO not confirmed after 80s".into());
    }
    println!("spend tx_id: {spend_tx_id}");
    println!("lookup:      tx {spend_tx_id} on {}", info.network_id);
    println!("engine preflight: PASSED (real script engine accepted the 2-of-2 multisig spend before submit)");

    // ── FACTS-ready: KCP-VT-002 ───────────────────────────────────────────────
    println!("\n--- FACTS.yaml-ready: KCP-VT-002 ---");
    println!("  id: KCP-VT-002");
    println!(
        "  claim: \"vault value locked under a real multisig covenant script (P2SH) and spent \
         by satisfying it — consensus-enforced, not just digest-anchored. \
         network={} lock_tx={lock_tx_id} spend_tx={spend_tx_id}\"",
        info.network_id
    );
    println!("  source: crates/kcp-vault/examples/onchain_evidence.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!(
        "  note: v1 — unaudited — 2-of-2 multisig P2SH lock+spend; \
         engine-preflighted; no dummy element (Kaspa CHECKMULTISIG does not require one). \
         Timelock CLTV on-chain also implemented (spend_timelock_vault). \
         Composite Any/All on-chain spend = next step."
    );

    Ok(())
}
