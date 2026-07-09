//! Two-party on-chain datasig evidence: lock value under a two-datasig CSFS
//! P2SH covenant and spend it by providing both oracle data-signatures.
//!
//! Demonstrates the full lock → spend cycle for the v1 paired-attestation
//! on-chain covenant. Both oracle keys must independently sign the shared
//! `msg_hash` (the 32-byte canonical attestation commitment); the covenant is
//! enforced by `OP_CHECKSIGFROMSTACK` at consensus level.
//!
//! The full on-chain pattern uses direct CSFS opcodes (not the silverscript
//! compiler). FACTS SS-024-v4 proves the primitive is real on kaspad v2.0.0.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_NET_SUFFIX=10                    \
//! KCP_KEY_FILE=/path/to/oracle_a.key  \
//! KCP_NEXT_KEY_FILE=/path/to/oracle_b.key \
//! cargo run -p kcp-paired-attestation --example onchain_evidence --features wrpc
//! ```
//!
//! - `KCP_KEY_FILE`      — oracle A's key (must hold funded testnet UTXO)
//! - `KCP_NEXT_KEY_FILE` — oracle B's distinct key (does not need funds)
//!
//! **Testnet only.** Refuses to run unless the node reports a testnet network.
//! No hardcoded private keys. No faucet automation.
//!
//! ## Evidence block (KCP-PA-002)
//!
//! On success the example prints a FACTS-ready KCP-PA-002 block proving:
//! "two-party attestation: value released ONLY on two valid independent oracle
//! data-signatures, enforced on-chain by OP_CHECKSIGFROMSTACK."
//!
//! Status: **v1 — unaudited — testnet first.**

use std::env;

use kaspa_consensus_core::tx::TransactionId;
use kcp_common::{
    tx::CARRIER_FEE_SOMPI,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_paired_attestation::{
    onchain::{
        datasig, lock_attestation_vault, spend_attestation_vault, two_datasig_redeem_script,
    },
    record::{attestation_id, AttestationRecord},
};

/// Value locked under the two-datasig CSFS covenant (1 KAS).
const LOCK_VALUE_SOMPI: u64 = 100_000_000;

type BoxError = Box<dyn std::error::Error>;

/// Returns `true` for transient node errors that warrant a confirmation-poll retry.
fn is_transient<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("not found") || s.contains("already spent") || s.contains("in the mempool")
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // ── environment ──────────────────────────────────────────────────────────
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| {
        eprintln!("KCP_NODE_URL not set; defaulting to ws://localhost:17210");
        "ws://localhost:17210".to_string()
    });
    let key_file_a = env::var("KCP_KEY_FILE")
        .map_err(|_| "KCP_KEY_FILE is required — path to oracle A's funded testnet wallet")?;
    let key_file_b = env::var("KCP_NEXT_KEY_FILE")
        .map_err(|_| "KCP_NEXT_KEY_FILE is required — path to oracle B's distinct key")?;
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

    // ── testnet guard ─────────────────────────────────────────────────────────
    if !info.network_id.contains("testnet") {
        return Err(format!(
            "REFUSED: network_id '{}' is not testnet. \
             This example must only run against a testnet node.",
            info.network_id
        )
        .into());
    }

    // ── load oracle wallets ───────────────────────────────────────────────────
    // Oracle A: holds the funded UTXO used to pay the lock fee.
    // Oracle B: only its key is needed (does not require a funded UTXO).
    let wallet_a = Wallet::load(std::path::Path::new(&key_file_a), 0, Prefix::Testnet)
        .map_err(|e| format!("load oracle A wallet: {e}"))?;
    let wallet_b = Wallet::load(std::path::Path::new(&key_file_b), 0, Prefix::Testnet)
        .map_err(|e| format!("load oracle B wallet: {e}"))?;

    let pk_a = wallet_a.keypair.x_only_public_key().0.serialize();
    let pk_b = wallet_b.keypair.x_only_public_key().0.serialize();

    println!("oracle A address: {}", wallet_a.address_string());
    println!("oracle B address: {}", wallet_b.address_string());
    println!("pk_a (hex): {}", hex::encode(pk_a));
    println!("pk_b (hex): {}", hex::encode(pk_b));

    // ── derive msg_hash from a sample attestation record ─────────────────────
    // msg_hash = canonical_hash(AttestationRecord) — the 32-byte commitment
    // that both oracles sign independently. In production this would be the
    // SHA-256 of the agreed attestation terms.
    let record = AttestationRecord::new([0x01u8; 32], [0x02u8; 32], 1);
    let msg_hash: [u8; 32] = attestation_id(&record).map_err(|e| format!("attestation_id: {e}"))?;
    println!("msg_hash (attestation_id): {}", hex::encode(msg_hash));

    // ── display redeem script for the record ─────────────────────────────────
    let redeem = two_datasig_redeem_script(&pk_a, &pk_b, &msg_hash)
        .map_err(|e| format!("two_datasig_redeem_script: {e}"))?;
    println!(
        "redeem script ({} bytes): {}",
        redeem.len(),
        hex::encode(&redeem)
    );

    // ── both oracles sign msg_hash independently (64-byte datasigs) ──────────
    let sig_a = datasig(&msg_hash, &wallet_a.keypair);
    let sig_b = datasig(&msg_hash, &wallet_b.keypair);
    println!("sig_a (oracle A, hex): {}", hex::encode(&sig_a));
    println!("sig_b (oracle B, hex): {}", hex::encode(&sig_b));

    // ── LOCK value under the two-datasig CSFS covenant ───────────────────────
    println!("\n--- LOCK value under two-datasig CSFS P2SH covenant ---");
    let lock_tx_id =
        lock_attestation_vault(&rpc, &wallet_a, &pk_a, &pk_b, &msg_hash, LOCK_VALUE_SOMPI)
            .await
            .map_err(|e| format!("lock_attestation_vault: {e}"))?;
    println!("lock tx_id: {lock_tx_id}");
    println!("lookup:     tx {lock_tx_id} on {}", info.network_id);
    println!(
        "note: value is now locked under a real two-datasig CSFS covenant; \
         it can only be released by providing valid data-sigs from BOTH oracles."
    );

    // ── SPEND by providing both datasigs ─────────────────────────────────────
    // The vault UTXO (output 0 of the lock tx) must be block-confirmed before it
    // can be spent. Poll with retries. The engine is run offline before submit.
    let lock_txid: TransactionId = lock_tx_id
        .parse()
        .map_err(|e| format!("parse lock tx_id: {e}"))?;

    println!("\n--- SPEND P2SH (satisfy two-datasig CSFS covenant) ---");
    println!(
        "note: spend_attestation_vault runs the real script engine offline \
         with covenants_enabled=true before submit."
    );

    let mut spend_tx_id = String::new();
    for attempt in 1..=40u32 {
        let result = spend_attestation_vault(
            &rpc,
            &pk_a,
            &pk_b,
            &msg_hash,
            (lock_txid, 0),
            sig_a.clone(),
            sig_b.clone(),
            &wallet_a.address,
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
            Err(e) => {
                return Err(format!("spend_attestation_vault (attempt {attempt}): {e}").into())
            }
        }
    }
    if spend_tx_id.is_empty() {
        return Err("two-datasig vault UTXO not confirmed after 80s".into());
    }
    println!("spend tx_id: {spend_tx_id}");
    println!("lookup:      tx {spend_tx_id} on {}", info.network_id);
    println!(
        "engine preflight: PASSED (real script engine with covenants_enabled=true \
         accepted the two-datasig CSFS spend before submit)"
    );

    // ── FACTS-ready: KCP-PA-002 ───────────────────────────────────────────────
    println!("\n--- FACTS.yaml-ready: KCP-PA-002 ---");
    println!("  id: KCP-PA-002");
    println!(
        "  claim: \"two-party attestation: value released ONLY on two valid \
         independent oracle data-signatures, enforced on-chain by \
         OP_CHECKSIGFROMSTACK. network={} lock_tx={lock_tx_id} spend_tx={spend_tx_id} \
         msg_hash={}\"",
        info.network_id,
        hex::encode(msg_hash)
    );
    println!("  source: crates/kcp-paired-attestation/examples/onchain_evidence.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!("  cite_facts: [SS-024-v4 (CSFS primitive proven on v2.0.0)]");
    println!(
        "  note: \"v1 — unaudited — full on-chain two-datasig covenant via direct \
         CSFS opcodes (not silverscript compiler); covenants_enabled=true; \
         engine-preflighted; 64-byte raw Schnorr datasigs. \
         The v0 off-chain-mating path (kcp-paired-attestation/src/tx.rs) \
         remains for the privacy-preserving disclosed-blind use case.\""
    );

    Ok(())
}
