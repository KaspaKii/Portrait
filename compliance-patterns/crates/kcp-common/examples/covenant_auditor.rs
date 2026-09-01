//! Auditor-side reader for a LIVE covenant lineage on testnet-10.
//!
//! Given only public information — the covenant_id, and the disclosed state
//! scripts (the same `live-capture.json` the publisher used) — an auditor
//! independently verifies the on-chain lineage head **without trusting the
//! publisher**:
//!   1. fetch the UTXO at `P2SH(state1)` (the claimed lineage head);
//!   2. confirm the node-attested `covenant_id` on that UTXO matches the
//!      disclosed covenant_id (consensus bound it; the auditor reads it back);
//!   3. confirm the UTXO's scriptPubKey equals `P2SH(state1_script)` — i.e. the
//!      live head encodes exactly the disclosed next-state, byte-for-byte;
//!   4. confirm the genesis UTXO `P2SH(state0)` with that covenant_id is GONE
//!      (spent by the append) — the lineage has advanced, not stalled.
//!
//! This is the pilot's auditor story made concrete against a real chain: the
//! consensus engine already guaranteed the *transition* was valid (it accepted
//! the append); the auditor confirms the *state on chain* is exactly what was
//! disclosed.
//!
//! ## Usage
//! ```text
//! KCP_NODE_URL=ws://localhost:17210 KCP_NET_SUFFIX=10 \
//! KCP_CAPTURE_JSON=.../CAVEATS/08-reserve-covenant/live-capture.json \
//! KCP_COVENANT_ID=7ba54cfa...7169 \
//!   cargo run -p kcp-common --example covenant_auditor --features wrpc
//! ```
//! Read-only: no keys, no transactions. Status: **v0 — unaudited.**

#[cfg(not(feature = "wrpc"))]
fn main() {
    eprintln!("requires --features wrpc");
}

#[cfg(feature = "wrpc")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    imp::run()
}

#[cfg(feature = "wrpc")]
mod imp {
    use kaspa_consensus_core::Hash;
    use kaspa_rpc_core::api::rpc::RpcApi;
    use kaspa_txscript::extract_script_pub_key_address;
    use kcp_common::{
        p2sh::p2sh_lock_script,
        wallet::Prefix,
        wrpc::{NodeClient, NodeConfig},
    };
    use std::env;

    type BoxError = Box<dyn std::error::Error>;

    pub fn run() -> Result<(), BoxError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        rt.block_on(run_async())
    }

    async fn run_async() -> Result<(), BoxError> {
        let node_url =
            env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://localhost:17210".to_string());
        let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let cap_path = env::var("KCP_CAPTURE_JSON")?;
        let cov_hex = env::var("KCP_COVENANT_ID")?;
        let cov_id = Hash::from_slice(&hex::decode(cov_hex.trim())?);

        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cap_path)?)?;
        let state0_script = hex::decode(
            v["state0_script_hex"]
                .as_str()
                .ok_or("no state0_script_hex")?,
        )?;
        let state1_script = hex::decode(
            v["state1_script_hex"]
                .as_str()
                .ok_or("no state1_script_hex")?,
        )?;
        let spk0 = p2sh_lock_script(&state0_script);
        let spk1 = p2sh_lock_script(&state1_script);

        let node = NodeClient::new(NodeConfig::testnet(&node_url, net_suffix));
        let rpc = node.rpc().await?;
        let info = node.server_info().await?;
        println!(
            "auditor connected: network={} synced={}",
            info.network_id, info.is_synced
        );
        println!("verifying covenant lineage {cov_id}\n");

        let addr0 = extract_script_pub_key_address(&spk0, Prefix::Testnet)?;
        let addr1 = extract_script_pub_key_address(&spk1, Prefix::Testnet)?;

        // [1+2+3] the claimed head: P2SH(state1) UTXO with the disclosed covenant_id,
        // whose spk equals P2SH(state1_script).
        let head = rpc
            .get_utxos_by_addresses(vec![addr1.clone()])
            .await?
            .into_iter()
            .find(|e| e.utxo_entry.covenant_id == Some(cov_id));

        let mut ok = true;
        match &head {
            Some(e) => {
                let spk_match = e.utxo_entry.script_public_key == spk1;
                println!(
                    "[head] FOUND at {}:{}  value={} sompi",
                    e.outpoint.transaction_id, e.outpoint.index, e.utxo_entry.amount
                );
                println!(
                    "  node-attested covenant_id == disclosed : {}",
                    e.utxo_entry.covenant_id == Some(cov_id)
                );
                println!("  scriptPubKey == P2SH(disclosed state1) : {spk_match}  (the live head encodes exactly the disclosed next-state)");
                ok &= spk_match;
            }
            None => {
                println!("[head] NOT FOUND — no UTXO at {addr1} carries covenant_id {cov_id}");
                ok = false;
            }
        }

        // [4] genesis must be consumed (lineage advanced).
        let genesis_live = rpc
            .get_utxos_by_addresses(vec![addr0.clone()])
            .await?
            .into_iter()
            .any(|e| e.utxo_entry.covenant_id == Some(cov_id));
        println!(
            "[genesis] P2SH(state0) UTXO with this covenant_id spent (lineage advanced): {}",
            !genesis_live
        );
        ok &= !genesis_live;

        println!(
            "\n=> AUDITOR VERDICT: {}",
            if ok {
                "VERIFIED — live on-chain head matches the disclosed state, lineage advanced"
            } else {
                "FAILED — on-chain state does not match the disclosure"
            }
        );
        if !ok {
            return Err("auditor verification failed".into());
        }
        Ok(())
    }
}
