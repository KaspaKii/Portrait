//! Testnet evidence runner for the sealed-lineage pattern.
//!
//! Demonstrates creation and a single append of a sealed-lineage UTXO chain
//! on the Kaspa testnet, followed by off-chain chain validation. Produces
//! FACTS.yaml-ready output for the data room.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_KEY_FILE=/path/to/wallet.key   \
//! cargo run -p kcp-sealed-lineage --example testnet_evidence \
//!            --features wrpc
//! ```
//!
//! Optional:
//! - `KCP_TS` — ISO-8601 timestamp to embed in the genesis body (for
//!   reproducibility across runs). If omitted, no timestamp field is added.
//!
//! ## Safety
//!
//! This example **refuses to run unless the node reports a testnet network**.
//! No hardcoded private keys. No faucet automation.
//!
//! Status: **v0 — unaudited — testnet first.**

use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use kcp_common::{
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_sealed_lineage::{
    invariants::{validate_chain, APPEND, GENESIS},
    payload::Payload,
    record::{commitment, lineage_id},
    tx::{append_lineage_tx, create_lineage_tx, DEFAULT_LINEAGE_VALUE_SOMPI},
};
use serde_json::{json, Value};

type BoxError = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // ---- read environment ------------------------------------------------
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| {
        eprintln!("KCP_NODE_URL not set; defaulting to ws://localhost:17210");
        "ws://localhost:17210".to_string()
    });
    let key_file = env::var("KCP_KEY_FILE")
        .map_err(|_| "KCP_KEY_FILE is required — path to a testnet wallet key file")?;
    let ts_field = env::var("KCP_TS").ok();

    // ---- connect ---------------------------------------------------------
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let config = NodeConfig::testnet(&node_url, net_suffix);
    let node = NodeClient::new(config);
    let rpc = node.rpc().await?;

    let info = node.server_info().await?;
    println!(
        "connected: server={} network={} synced={} daa={}",
        info.server_version, info.network_id, info.is_synced, info.virtual_daa_score
    );

    // ---- testnet guard ---------------------------------------------------
    if !info.network_id.contains("testnet") {
        return Err(format!(
            "REFUSED: network_id '{}' is not testnet. \
             This example must only run against a testnet node.",
            info.network_id
        )
        .into());
    }

    // ---- load wallet -----------------------------------------------------
    let wallet = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("failed to load wallet: {e}"))?;
    println!("publisher wallet: {}", wallet.address_string());

    // ---- current Unix timestamp for t_bucket ----------------------------
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before Unix epoch")
        .as_secs();

    // ---- build genesis body and lineage_id ------------------------------
    let mut genesis_body: Value = json!({"name": "kcp-sl-evidence"});
    if let Some(ts) = ts_field {
        genesis_body["ts"] = Value::String(ts);
    }
    let lid = lineage_id(&genesis_body).map_err(|e| format!("lineage_id: {e}"))?;
    println!("lineage_id:       {}", hex::encode(lid));

    // ---- genesis commitment and payload ----------------------------------
    let genesis_blind = [0x01u8; 32]; // deterministic for example; use random in production
    let genesis_commitment =
        commitment(&genesis_body, &genesis_blind).map_err(|e| format!("commitment: {e}"))?;
    let genesis_payload = Payload {
        lineage_id: lid,
        seq: 0,
        event_class: GENESIS,
        t_bucket: now_secs,
        commitment: genesis_commitment,
    }
    .encode();

    println!("\n--- CREATE LINEAGE (genesis) ---");
    let genesis_tx_id =
        create_lineage_tx(&rpc, &wallet, genesis_payload, DEFAULT_LINEAGE_VALUE_SOMPI)
            .await
            .map_err(|e| format!("create_lineage_tx: {e}"))?;
    println!("genesis tx_id:    {genesis_tx_id}");
    println!(
        "lookup:           tx {genesis_tx_id} on {}",
        info.network_id
    );

    // ---- append event ----------------------------------------------------
    let lineage_txid: kaspa_consensus_core::tx::TransactionId = genesis_tx_id
        .parse()
        .map_err(|e| format!("failed to parse genesis tx_id: {e}"))?;
    let lineage_outpoint = (lineage_txid, 0u32);

    let append_body = json!({"action": "append"});
    let append_blind = [0x02u8; 32]; // deterministic for example; use random in production
    let append_commitment =
        commitment(&append_body, &append_blind).map_err(|e| format!("commitment: {e}"))?;
    let append_payload = Payload {
        lineage_id: lid,
        seq: 1,
        event_class: APPEND,
        t_bucket: now_secs,
        commitment: append_commitment,
    }
    .encode();

    println!("\n--- APPEND TO LINEAGE ---");
    // The lineage UTXO must be block-included and indexed before it can be
    // spent; poll briefly for confirmation instead of failing on the race.
    let mut append_tx_id = String::new();
    for attempt in 1..=30u32 {
        match append_lineage_tx(&rpc, &wallet, lineage_outpoint, append_payload.clone()).await {
            Ok(id) => {
                append_tx_id = id;
                break;
            }
            Err(e) if e.to_string().contains("not found") && attempt < 30 => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => return Err(format!("append_lineage_tx: {e}").into()),
        }
    }
    if append_tx_id.is_empty() {
        return Err("lineage UTXO not confirmed after 60s".into());
    }
    println!("append tx_id:     {append_tx_id}");
    println!("lookup:           tx {append_tx_id} on {}", info.network_id);

    // ---- off-chain chain validation (L-1 through L-4) -------------------
    let chain = vec![
        Payload {
            lineage_id: lid,
            seq: 0,
            event_class: GENESIS,
            t_bucket: now_secs,
            commitment: genesis_commitment,
        },
        Payload {
            lineage_id: lid,
            seq: 1,
            event_class: APPEND,
            t_bucket: now_secs,
            commitment: append_commitment,
        },
    ];
    validate_chain(&chain).map_err(|e| format!("chain validation failed: {e}"))?;
    println!("\nlineage:          validated (off-chain, L-1/L-2/L-3/L-4)");

    // ---- FACTS.yaml-ready output -----------------------------------------
    let lid_hex = hex::encode(lid);
    println!("\n--- FACTS.yaml-ready ---");
    println!("  id: KCP-SL-001");
    println!(
        "  claim: \"kcp-sealed-lineage v0 pattern exercised on {}: \
         lineage_id={lid_hex}, genesis_tx={genesis_tx_id}, append_tx={append_tx_id}\"",
        info.network_id
    );
    println!("  source: examples/testnet_evidence.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!("  note: v0 — unaudited — invariants enforced off-chain only");

    Ok(())
}
