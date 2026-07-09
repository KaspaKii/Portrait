//! Print the UTXO set and total spendable balance for a testnet address.
//!
//! Usage:
//!   KCP_NODE_URL=ws://127.0.0.1:17210 \
//!   KCP_ADDRESS=kaspatest:q... \
//!   cargo run -p kcp-common --example balance --features wrpc
//!
//! Optional: `KCP_NET_SUFFIX` (default 10). Read-only; queries the node's
//! UTXO index for the given address and sums the amounts.

#[cfg(feature = "wrpc")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use kaspa_addresses::Address;
    use kaspa_rpc_core::api::rpc::RpcApi;
    use kcp_common::wrpc::{NodeClient, NodeConfig};
    use std::env;

    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://127.0.0.1:17210".to_string());
    let address_str = env::var("KCP_ADDRESS").map_err(|_| "KCP_ADDRESS is required")?;
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let address: Address =
        Address::try_from(address_str.as_str()).map_err(|e| format!("bad address: {e}"))?;

    let node = NodeClient::new(NodeConfig::testnet(&node_url, net_suffix));
    let rpc = node.rpc().await?;

    let utxos = rpc
        .get_utxos_by_addresses(vec![address.clone()])
        .await
        .map_err(|e| format!("get_utxos_by_addresses: {e}"))?;

    let total: u64 = utxos.iter().map(|e| e.utxo_entry.amount).sum();
    println!("address    : {address}");
    println!("utxo count : {}", utxos.len());
    println!(
        "balance    : {} sompi  ({:.8} TKAS)",
        total,
        total as f64 / 1e8
    );
    for (i, e) in utxos.iter().enumerate() {
        println!(
            "  [{i}] {} sompi  daa={}  coinbase={}",
            e.utxo_entry.amount, e.utxo_entry.block_daa_score, e.utxo_entry.is_coinbase
        );
    }
    Ok(())
}

#[cfg(not(feature = "wrpc"))]
fn main() {
    eprintln!(
        "balance requires the `wrpc` feature:\n  \
         cargo run -p kcp-common --example balance --features wrpc"
    );
}
