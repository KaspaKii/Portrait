//! Read-only node status + covenant-activation probe.
//!
//! Connects to a Kaspa testnet node and reports server version, network,
//! sync state, and virtual DAA score — then reports whether the **Toccata**
//! hardfork (which activates SilverScript covenant introspection) is live at
//! the current DAA.
//!
//! Ground truth (rusty-kaspa v2.0.0, the workspace pin):
//! - testnet-10 Toccata activation DAA = `467_579_632`
//!   (`consensus/core/src/config/params.rs` testnet-10 block).
//! - `covenants_enabled = toccata_activation.is_active(block_daa_score)`
//!   (`consensus/src/processes/transaction_validator/tx_validation_in_utxo_context.rs`).
//!   So covenant-bound transactions are consensus-valid exactly when the
//!   virtual DAA score has reached the activation score.
//!
//! ## Usage
//! ```text
//! KCP_NODE_URL=ws://localhost:17210 \
//! cargo run -p kcp-common --example node_status --features wrpc
//! ```
//! Optional: `KCP_NET_SUFFIX` (default 10).
//!
//! ## Safety
//! Read-only. No keys, no transactions, no funds touched. This example only
//! queries `server_info`.
//!
//! Status: **v0 — unaudited — testnet diagnostic.**

/// testnet-10 Toccata (covenant) activation DAA score. Source: rusty-kaspa
/// v2.0.0 `consensus/core/src/config/params.rs` (testnet-10 params block).
#[cfg(feature = "wrpc")]
const TESTNET10_TOCCATA_DAA: u64 = 467_579_632;

#[cfg(feature = "wrpc")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use kcp_common::wrpc::{NodeClient, NodeConfig};
    use std::env;

    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://localhost:17210".to_string());
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let config = NodeConfig::testnet(&node_url, net_suffix);
    let node = NodeClient::new(config);
    let _rpc = node.rpc().await?;
    let info = node.server_info().await?;

    println!("server_version : {}", info.server_version);
    println!("network_id     : {}", info.network_id);
    println!("is_synced      : {}", info.is_synced);
    println!("virtual_daa    : {}", info.virtual_daa_score);
    println!("toccata_daa    : {TESTNET10_TOCCATA_DAA}  (testnet-10 covenant activation)");

    if !info.network_id.contains("testnet-10") {
        println!(
            "covenants      : UNKNOWN — activation constant is for testnet-10, \
             but node reports '{}'",
            info.network_id
        );
        return Ok(());
    }

    if info.virtual_daa_score >= TESTNET10_TOCCATA_DAA {
        let past = info.virtual_daa_score - TESTNET10_TOCCATA_DAA;
        println!("covenants      : ACTIVE  (past activation by {past} DAA)");
    } else {
        let togo = TESTNET10_TOCCATA_DAA - info.virtual_daa_score;
        // ~10 blocks/sec on testnet-10 → rough wall-clock to activation.
        let approx_hours = (togo as f64) / 10.0 / 3600.0;
        println!(
            "covenants      : NOT YET ACTIVE  ({togo} DAA to go, ~{approx_hours:.1}h at 10 BPS)"
        );
    }
    Ok(())
}

#[cfg(not(feature = "wrpc"))]
fn main() {
    eprintln!(
        "node_status requires the `wrpc` feature:\n  \
         cargo run -p kcp-common --example node_status --features wrpc"
    );
}
