//! P2SH lock→spend round-trip on the Kaspa testnet.
//!
//! Proves the covenant spend-path plumbing end-to-end on a live network:
//! lock value under a redeem script (`<wallet x-only pubkey> OP_CHECKSIG`),
//! then spend it back by satisfying that script. Both transactions are real
//! and land on-chain. This is the foundation the vault and paired-attestation
//! patterns build their consensus-enforced versions on.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_NET_SUFFIX=10                    \
//! KCP_KEY_FILE=/path/to/wallet.key     \
//! cargo run -p kcp-common --example p2sh_roundtrip --features wrpc
//! ```
//!
//! Refuses to run unless the node reports a testnet network. No hardcoded keys.
//!
//! Status: **v0 — unaudited — testnet first.**

use std::env;

use kcp_common::{
    p2sh::{lock_to_p2sh_tx, schnorr_satisfier_sig, spend_p2sh_tx},
    tx::CARRIER_FEE_SOMPI,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};

use kaspa_consensus_core::tx::TransactionId;
use kaspa_txscript::{opcodes::codes::OpCheckSig, script_builder::ScriptBuilder};

/// Value locked under the P2SH redeem script (1 KAS).
const LOCK_VALUE_SOMPI: u64 = 100_000_000;

type BoxError = Box<dyn std::error::Error>;

fn is_transient<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("not found") || s.contains("already spent") || s.contains("in the mempool")
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://localhost:17210".to_string());
    let key_file = env::var("KCP_KEY_FILE").map_err(|_| "KCP_KEY_FILE is required")?;
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

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

    let wallet = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("load wallet: {e}"))?;
    println!("wallet:           {}", wallet.address_string());

    // Redeem script: <wallet x-only pubkey> OP_CHECKSIG — a P2SH-wrapped
    // single-sig. The simplest covenant that exercises the full lock→spend path.
    let xonly = wallet.keypair.x_only_public_key().0.serialize();
    let redeem = ScriptBuilder::new()
        .add_data(&xonly)?
        .add_op(OpCheckSig)?
        .drain()
        .to_vec();
    println!(
        "redeem script:    {} bytes (<xonly> OP_CHECKSIG)",
        redeem.len()
    );

    // ── LOCK ────────────────────────────────────────────────────────────────
    println!("\n--- LOCK value under P2SH ---");
    let lock_tx_id = lock_to_p2sh_tx(&rpc, &wallet, &redeem, LOCK_VALUE_SOMPI)
        .await
        .map_err(|e| format!("lock_to_p2sh_tx: {e}"))?;
    println!("lock tx_id:       {lock_tx_id}");
    println!("lookup:           tx {lock_tx_id} on {}", info.network_id);

    // ── SPEND ─────────────────────────────────────────────────────────────────
    // The P2SH UTXO (output 0 of the lock tx) must be block-included before it
    // can be spent; poll until the spend succeeds. spend_p2sh_tx runs the real
    // script engine over the assembled spend before submitting.
    let lock_txid: TransactionId = lock_tx_id
        .parse()
        .map_err(|e| format!("parse tx_id: {e}"))?;
    println!("\n--- SPEND P2SH (satisfy <sig> against the redeem script) ---");
    let mut spend_tx_id = String::new();
    for attempt in 1..=40u32 {
        let result = spend_p2sh_tx(
            &rpc,
            &redeem,
            (lock_txid, 0),
            &wallet.address,
            Prefix::Testnet,
            CARRIER_FEE_SOMPI,
            false, // CHECKSIG validates identically with covenants on/off
            |sighash| Ok(vec![schnorr_satisfier_sig(sighash, &wallet.keypair)]),
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
            Err(e) => return Err(format!("spend_p2sh_tx: {e}").into()),
        }
    }
    if spend_tx_id.is_empty() {
        return Err("p2sh UTXO not confirmed after 80s".into());
    }
    println!("spend tx_id:      {spend_tx_id}");
    println!("lookup:           tx {spend_tx_id} on {}", info.network_id);
    println!("\nengine preflight: PASSED (real script engine accepted the spend before submit)");

    // ── FACTS-ready ───────────────────────────────────────────────────────────
    println!("\n--- FACTS.yaml-ready ---");
    println!("  id: KCP-P2SH-001");
    println!(
        "  claim: \"P2SH covenant spend-path proven on {}: redeem=<xonly> OP_CHECKSIG, \
         lock_tx={lock_tx_id}, spend_tx={spend_tx_id} (value locked under a redeem script \
         and spent by satisfying it; engine-preflighted)\"",
        info.network_id
    );
    println!("  source: crates/kcp-common/examples/p2sh_roundtrip.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!("  note: v0 — unaudited — real on-chain P2SH lock+spend; foundation for vault/PLA full on-chain enforcement");

    Ok(())
}
