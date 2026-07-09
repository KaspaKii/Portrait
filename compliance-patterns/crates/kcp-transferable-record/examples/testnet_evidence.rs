//! Testnet evidence runner for the transferable-record pattern.
//!
//! Demonstrates creation and transfer of a transferable record on the Kaspa
//! testnet. Produces two FACTS.yaml-ready output lines for the data room.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_KEY_FILE=/path/to/wallet.key   \
//! cargo run -p kcp-transferable-record --example testnet_evidence \
//!            --features wrpc
//! ```
//!
//! Optional:
//! - `KCP_NEXT_KEY_FILE` — path to a second wallet key file for the "next
//!   controller". If omitted, the next controller is derived at index 1 from
//!   the same mnemonic as the primary wallet.
//! - `KCP_TS` — ISO-8601 timestamp to embed in the genesis body (for
//!   reproducibility across runs). If omitted, the genesis body carries no
//!   timestamp field.
//!
//! ## Safety
//!
//! This example **refuses to run unless the node reports a testnet network**.
//! No hardcoded private keys. No faucet automation.
//!
//! Status: **v0 — unaudited — testnet first.**

use std::env;

use kcp_common::{
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_transferable_record::{
    lineage::TransferEvent,
    payload::Payload,
    record::{commitment, record_id},
    tx::{create_record_tx, transfer_record_tx},
};
use serde_json::{json, Value};

/// Minimum record UTXO value in sompi (1 KAS on testnet — small but meaningful).
const RECORD_VALUE_SOMPI: u64 = 100_000_000;

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
    let next_key_file = env::var("KCP_NEXT_KEY_FILE").ok();
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

    // ---- load wallets ----------------------------------------------------
    let primary = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("failed to load primary wallet: {e}"))?;
    println!("primary wallet:  {}", primary.address_string());

    let next_ctrl = match next_key_file {
        Some(ref path) => {
            let w = Wallet::load(std::path::Path::new(path), 0, Prefix::Testnet)
                .map_err(|e| format!("failed to load next-controller wallet: {e}"))?;
            println!(
                "next controller: {} (from KCP_NEXT_KEY_FILE)",
                w.address_string()
            );
            w
        }
        None => {
            // Derive index 1 from the same key file.
            let w = Wallet::load(std::path::Path::new(&key_file), 1, Prefix::Testnet)
                .map_err(|e| format!("failed to derive next-controller at index 1: {e}"))?;
            println!(
                "next controller: {} (index 1 from KCP_KEY_FILE)",
                w.address_string()
            );
            w
        }
    };

    // ---- build genesis body and record_id --------------------------------
    let mut genesis_body: Value = json!({"name": "kcp-evidence"});
    if let Some(ts) = ts_field {
        genesis_body["ts"] = Value::String(ts);
    }
    let rid = record_id(&genesis_body).map_err(|e| format!("record_id: {e}"))?;
    println!("record_id:       {}", hex::encode(rid));

    // ---- create record ---------------------------------------------------
    let create_commitment = commitment(&genesis_body).map_err(|e| format!("commitment: {e}"))?;
    let create_payload = Payload {
        record_id: rid,
        seq: 0,
        commitment: create_commitment,
    }
    .encode();

    println!("\n--- CREATE RECORD ---");
    let create_tx_id = create_record_tx(
        &rpc,
        &primary,
        &primary.address,
        create_payload,
        RECORD_VALUE_SOMPI,
    )
    .await
    .map_err(|e| format!("create_record_tx: {e}"))?;
    println!("create tx_id:    {create_tx_id}");
    println!("lookup:          tx {create_tx_id} on {}", info.network_id);

    // ---- transfer record -------------------------------------------------
    // Locate the record UTXO (output index 0 of the create tx).
    let record_txid: kaspa_consensus_core::tx::TransactionId = create_tx_id
        .parse()
        .map_err(|e| format!("failed to parse tx_id: {e}"))?;
    let record_outpoint = (record_txid, 0u32);

    let transfer_body = json!({"action": "transfer", "to": next_ctrl.address_string()});
    let transfer_commitment = commitment(&transfer_body).map_err(|e| format!("commitment: {e}"))?;
    let transfer_payload = Payload {
        record_id: rid,
        seq: 1,
        commitment: transfer_commitment,
    }
    .encode();

    println!("\n--- TRANSFER RECORD ---");
    // The record UTXO must be block-included and indexed before it can be
    // spent; poll briefly for confirmation instead of failing on the race.
    let mut transfer_tx_id = String::new();
    for attempt in 1..=30u32 {
        match transfer_record_tx(
            &rpc,
            &primary,
            record_outpoint,
            &next_ctrl.address,
            transfer_payload.clone(),
        )
        .await
        {
            Ok(id) => {
                transfer_tx_id = id;
                break;
            }
            Err(e) if e.to_string().contains("not found") && attempt < 30 => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => return Err(format!("transfer_record_tx: {e}").into()),
        }
    }
    if transfer_tx_id.is_empty() {
        return Err("record UTXO not confirmed after 60s".into());
    }
    println!("transfer tx_id:  {transfer_tx_id}");
    println!(
        "lookup:          tx {transfer_tx_id} on {}",
        info.network_id
    );

    // ---- lineage verification (off-chain) --------------------------------
    let genesis_ctrl_xonly: [u8; 32] = primary.keypair.x_only_public_key().0.serialize();
    let next_ctrl_xonly: [u8; 32] = next_ctrl.keypair.x_only_public_key().0.serialize();
    let events = vec![TransferEvent {
        seq: 1,
        record_id: rid,
        controller_xonly: next_ctrl_xonly,
        commitment: transfer_commitment,
    }];
    kcp_transferable_record::lineage::validate_chain(&genesis_ctrl_xonly, &events)
        .map_err(|e| format!("lineage validation failed: {e}"))?;
    println!("\nlineage:         validated (off-chain, TR-1/TR-2/TR-3)");

    // ---- FACTS.yaml-ready output -----------------------------------------
    let rid_hex = hex::encode(rid);
    println!("\n--- FACTS.yaml-ready ---");
    println!("  id: KCP-TR-001");
    println!(
        "  claim: \"kcp-transferable-record v0 pattern exercised on {}: \
         record_id={rid_hex}, create_tx={create_tx_id}, transfer_tx={transfer_tx_id}\"",
        info.network_id
    );
    println!("  source: examples/testnet_evidence.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!("  note: v0 — unaudited — lineage enforced off-chain only");

    Ok(())
}
