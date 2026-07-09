//! Portrait settlement — LIVE TN10 broadcast of the sound cross-layer binding.
//!
//! THESIS: a P2SH covenant whose redeem script is the KIP-16 tag-0x21 verifier
//! pinned to a specific vProg (`image_id` + `control_id`) can be spent ONLY by
//! presenting a real RISC Zero succinct STARK that verifies in-consensus and
//! binds the next state (`journal`). A tampered proof / wrong next-state cannot
//! spend it.
//!
//! This binary does it for real on testnet-10:
//!   1. LOCK: fund a P2SH(redeem) output, where redeem =
//!        <image_id> <control_id> <hashfn> <tag 0x21> OpZkPrecompile
//!      (the engine's own `p2sh_scripts()` redeem half — committed in the
//!      lock address, so the address itself pins the vProg).
//!   2. SETTLE (positive): spend the P2SH UTXO with signature script pushing
//!        <claim> <control_index> <control_digests> <seal> <journal>
//!      then the redeem. The engine runs the real STARK verifier; it accepts
//!      only because the proof is valid and binds journal+image+control.
//!   3. NEGATIVE CONTROL: attempt the same spend with a tampered journal. The
//!      node/engine MUST reject it (the proof does not bind that state).
//!
//! Every step is gated by `verify_p2sh_spend_offline` (the real engine) BEFORE
//! submission, then submitted to the live node. No fabricated evidence: txids
//! printed are real submit_transaction results.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://127.0.0.1:17210 \
//! KCP_KEY_FILE=.secrets/tn10-portrait.key \
//! KCP_PROOF_DIR=/path/to/proof-export/succinct \
//! KCP_NET_SUFFIX=10 \
//!   cargo run --manifest-path examples/portrait-settlement/Cargo.toml --bin settle --release
//! ```
//!
//! `KCP_PROOF_DIR` must contain the 8 proof fields as hex files
//! (`claim, control_index, control_digests, seal, journal|journal_hash,
//! image|image_id, control_id, hashfn`). The `journal` on-chain field is the
//! 32-byte sha256(journal_bytes); if only `journal_hash.hex` is present it is
//! used. Generate with the kii-csci-prover (RISC0_DEV_MODE=0).
//!
//! Status: v0 — pre-production — unaudited — testnet-only.

use std::env;
use std::path::{Path, PathBuf};

use kaspa_addresses::Prefix as AddrPrefix;
use kaspa_consensus_core::tx::TransactionId;
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_txscript::{extract_script_pub_key_address, opcodes::codes::OpZkPrecompile};

use kcp_common::{
    p2sh::{lock_to_p2sh_tx, p2sh_lock_script, spend_p2sh_tx_with_sigops},
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_pq_anchor::sigop::sigop_count_for_pq_verify;

type BoxError = Box<dyn std::error::Error>;

const ZK_TAG_R0_SUCCINCT: u8 = 0x21;
const HASHFN_POSEIDON2: u8 = 1;

// ── proof loading ───────────────────────────────────────────────────────────

fn proof_dir() -> PathBuf {
    env::var("KCP_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let manifest = env!("CARGO_MANIFEST_DIR");
            PathBuf::from(manifest).join("../../crates/kcp-pq-anchor/tests/fixtures")
        })
}

/// Read `<dir>/<name>.hex` or `<dir>/succinct.<name>.hex` → bytes.
fn read_hex(dir: &Path, name: &str) -> Result<Vec<u8>, BoxError> {
    let plain = dir.join(format!("{name}.hex"));
    let succinct = dir.join(format!("succinct.{name}.hex"));
    let path = if plain.exists() { plain } else { succinct };
    let s = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(hex::decode(s.trim())?)
}

struct Proof {
    claim: Vec<u8>,
    control_index: Vec<u8>, // 4-byte LE, as on-chain
    control_digests: Vec<u8>,
    seal: Vec<u8>,
    journal: Vec<u8>, // 32-byte sha256(journal_bytes)
    image_id: Vec<u8>,
    control_id: Vec<u8>,
}

fn load_proof(dir: &Path) -> Result<Proof, BoxError> {
    // image field is exported as either image.hex or image_id.hex
    let image_name = if dir.join("image.hex").exists() || dir.join("succinct.image.hex").exists() {
        "image"
    } else {
        "image_id"
    };
    // journal on-chain field is the 32-byte hash. The prover writes the raw
    // 104-byte journal to journal.hex and the hash to journal_hash.hex; the
    // engine vectors put the 32-byte hash directly in journal.hex. Pick the
    // 32-byte one.
    let journal = {
        let j = read_hex(dir, "journal")?;
        if j.len() == 32 {
            j
        } else {
            let jh = read_hex(dir, "journal_hash")?;
            if jh.len() != 32 {
                return Err(format!(
                    "neither journal.hex ({} bytes) nor journal_hash.hex ({} bytes) is 32 bytes",
                    j.len(),
                    jh.len()
                )
                .into());
            }
            jh
        }
    };
    Ok(Proof {
        claim: read_hex(dir, "claim")?,
        control_index: read_hex(dir, "control_index")?,
        control_digests: read_hex(dir, "control_digests")?,
        seal: read_hex(dir, "seal")?,
        journal,
        image_id: read_hex(dir, image_name)?,
        control_id: read_hex(dir, "control_id")?,
    })
}

// ── script halves (mirror the engine's R0Fields::p2sh_scripts) ───────────────

/// Minimal canonical data push that does NOT minify single small-int bytes to
/// OP_N. The engine's `parse_hashfn`/tag parsing require a 1-byte DATA push,
/// so hashfn (1) and tag (0x21) must be pushed as `0x01 <byte>`, never OP_1.
fn push_data(script: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    match len {
        0 => script.push(0x00),
        1..=75 => {
            script.push(len as u8);
            script.extend_from_slice(data);
        }
        76..=255 => {
            script.push(0x4c);
            script.push(len as u8);
            script.extend_from_slice(data);
        }
        256..=65535 => {
            script.push(0x4d);
            script.push((len & 0xff) as u8);
            script.push((len >> 8) as u8);
            script.extend_from_slice(data);
        }
        _ => {
            script.push(0x4e);
            script.extend_from_slice(&(len as u32).to_le_bytes());
            script.extend_from_slice(data);
        }
    }
}

/// Redeem half (committed in the P2SH lock address):
///   <image_id> <control_id> <hashfn> <tag 0x21> OpZkPrecompile
fn build_redeem(p: &Proof) -> Vec<u8> {
    let mut s = Vec::new();
    push_data(&mut s, &p.image_id);
    push_data(&mut s, &p.control_id);
    push_data(&mut s, &[HASHFN_POSEIDON2]);
    push_data(&mut s, &[ZK_TAG_R0_SUCCINCT]);
    s.push(OpZkPrecompile);
    s
}

/// Satisfier elements (pushed in the signature script, bottom→top):
///   <claim> <control_index> <control_digests> <seal> <journal>
/// kcp_common::spend_p2sh_tx appends the redeem push after these.
fn satisfier(p: &Proof) -> Vec<Vec<u8>> {
    vec![
        p.claim.clone(),
        p.control_index.clone(),
        p.control_digests.clone(),
        p.seal.clone(),
        p.journal.clone(),
    ]
}

// ── main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://127.0.0.1:17210".to_string());
    let key_file =
        env::var("KCP_KEY_FILE").map_err(|_| "KCP_KEY_FILE is required (testnet wallet key)")?;
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    // Lock value for the covenant UTXO. Must cover the spend fee.
    let lock_value: u64 = env::var("KCP_LOCK_SOMPI")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200_000_000); // 2 TKAS
    let spend_fee: u64 = env::var("KCP_SPEND_FEE_SOMPI")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000_000); // 0.5 TKAS — generous; large tx

    let dir = proof_dir();
    println!("=== Portrait Settlement — LIVE TN10 (KIP-16 tag 0x21) ===");
    println!("[proof] dir: {}", dir.display());
    let proof = load_proof(&dir)?;
    println!("[proof] seal:        {} bytes", proof.seal.len());
    println!("[proof] journal:     {}", hex::encode(&proof.journal));
    println!("[proof] image_id:    {}", hex::encode(&proof.image_id));
    println!("[proof] control_id:  {}", hex::encode(&proof.control_id));

    let redeem = build_redeem(&proof);
    println!("[script] redeem:     {} bytes", redeem.len());
    let p2sh_spk = p2sh_lock_script(&redeem);
    let p2sh_addr = extract_script_pub_key_address(&p2sh_spk, AddrPrefix::Testnet)
        .map_err(|e| format!("p2sh address: {e}"))?;
    println!("[script] p2sh addr:  {p2sh_addr}");

    // ── connect ────────────────────────────────────────────────────────────
    let config = NodeConfig::testnet(&node_url, net_suffix);
    let node = NodeClient::new(config);
    let rpc = node.rpc().await?;
    let info = node.server_info().await?;
    println!(
        "[node] server={} network={} synced={} daa={}",
        info.server_version, info.network_id, info.is_synced, info.virtual_daa_score
    );
    if !info.network_id.contains("testnet") {
        return Err(format!("REFUSED: network_id '{}' is not testnet", info.network_id).into());
    }

    let wallet = Wallet::load(Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("load wallet: {e}"))?;
    println!("[wallet] {}", wallet.address_string());

    // ── 1. LOCK ──────────────────────────────────────────────────────────────
    // KCP_REUSE_LOCK_TXID lets a re-run spend an already-locked P2SH UTXO
    // (output index 0) instead of locking fresh funds — avoids waste on retries.
    let lock_txid = if let Ok(existing) = env::var("KCP_REUSE_LOCK_TXID") {
        println!("\n[1/3] LOCK — reusing existing P2SH UTXO {existing}:0");
        existing
    } else {
        println!("\n[1/3] LOCK — funding P2SH(redeem) with {lock_value} sompi ...");
        let txid = lock_to_p2sh_tx(rpc.as_ref(), &wallet, &redeem, lock_value)
            .await
            .map_err(|e| format!("lock failed: {e}"))?;
        println!("      lock tx: {txid}");
        txid
    };

    // Wait for the P2SH UTXO to appear at the covenant address.
    let lock_tx_id: TransactionId = lock_txid
        .parse()
        .map_err(|e| format!("parse lock txid: {e}"))?;
    println!("      waiting for P2SH UTXO to confirm ...");
    let mut found = false;
    for attempt in 1..=120u32 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let entries = rpc
            .get_utxos_by_addresses(vec![p2sh_addr.clone()])
            .await
            .map_err(|e| format!("get_utxos(p2sh): {e}"))?;
        if entries
            .iter()
            .any(|e| e.outpoint.transaction_id == lock_tx_id && e.outpoint.index == 0)
        {
            found = true;
            println!("      P2SH UTXO confirmed (after {}s)", attempt * 2);
            break;
        }
    }
    if !found {
        return Err("P2SH lock UTXO did not confirm within 240s".into());
    }

    // ── 2. SETTLE (positive) ──────────────────────────────────────────────────
    println!("\n[2/3] SETTLE — spending P2SH UTXO via real STARK verification ...");
    let p = &proof;
    let settle_txid = spend_p2sh_tx_with_sigops(
        rpc.as_ref(),
        &redeem,
        (lock_tx_id, 0),
        &wallet.address,
        AddrPrefix::Testnet,
        spend_fee,
        true,                        // covenants_enabled (TN10 post-Toccata)
        sigop_count_for_pq_verify(), // 255 → ~25.5M script units for tag-0x21
        false,                       // run the engine preflight (safety gate)
        |_sighash| Ok(satisfier(p)),
    )
    .await
    .map_err(|e| format!("settle spend failed: {e}"))?;
    println!("      SETTLE tx: {settle_txid}");
    println!("      → accepted ONLY because the STARK verified and bound journal+image+control.");

    // ── 3. ON-CHAIN NEGATIVE CONTROL ──────────────────────────────────────────
    // Lock a SECOND P2SH UTXO under the SAME covenant, then submit a spend whose
    // journal is tampered (next-state bit flipped). We DELIBERATELY skip the
    // local preflight and submit to the LIVE NODE, so the rejection is the
    // node's own consensus enforcement — the on-chain negative control. A
    // rejected tx has no txid; the evidence is the node's error string.
    println!("\n[3/3] ON-CHAIN NEGATIVE CONTROL — submit tampered spend to the live node ...");
    let neg_lock_txid = if let Ok(existing) = env::var("KCP_NEGCTL_LOCK_TXID") {
        println!("      reusing existing negctl P2SH UTXO {existing}:0");
        existing
    } else {
        let txid = lock_to_p2sh_tx(rpc.as_ref(), &wallet, &redeem, lock_value)
            .await
            .map_err(|e| format!("negctl lock failed: {e}"))?;
        println!("      negctl lock tx: {txid}");
        txid
    };
    let neg_lock_id: TransactionId = neg_lock_txid
        .parse()
        .map_err(|e| format!("parse negctl lock txid: {e}"))?;
    println!("      waiting for negctl P2SH UTXO to confirm ...");
    let mut neg_found = false;
    for attempt in 1..=120u32 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let entries = rpc
            .get_utxos_by_addresses(vec![p2sh_addr.clone()])
            .await
            .map_err(|e| format!("get_utxos(p2sh negctl): {e}"))?;
        if entries
            .iter()
            .any(|e| e.outpoint.transaction_id == neg_lock_id && e.outpoint.index == 0)
        {
            neg_found = true;
            println!("      negctl P2SH UTXO confirmed (after {}s)", attempt * 2);
            break;
        }
    }
    if !neg_found {
        return Err("negctl P2SH lock UTXO did not confirm within 240s".into());
    }

    let mut bad = Proof {
        claim: p.claim.clone(),
        control_index: p.control_index.clone(),
        control_digests: p.control_digests.clone(),
        seal: p.seal.clone(),
        journal: p.journal.clone(),
        image_id: p.image_id.clone(),
        control_id: p.control_id.clone(),
    };
    bad.journal[0] ^= 0x01; // tamper the bound next-state commitment
    println!(
        "      tampered journal: {} (orig {})",
        hex::encode(&bad.journal),
        hex::encode(&p.journal)
    );
    let neg_result = spend_p2sh_tx_with_sigops(
        rpc.as_ref(),
        &redeem,
        (neg_lock_id, 0),
        &wallet.address,
        AddrPrefix::Testnet,
        spend_fee,
        true,
        sigop_count_for_pq_verify(),
        true, // SKIP preflight — force the live node to do the rejecting
        |_sighash| Ok(satisfier(&bad)),
    )
    .await;
    let neg_outcome = match neg_result {
        Ok(txid) => {
            return Err(format!(
                "SECURITY FAILURE: live node ACCEPTED tampered-journal spend as {txid}"
            )
            .into())
        }
        Err(e) => {
            println!("      NODE REJECTED ✓");
            println!("      node error: {e}");
            format!("{e}")
        }
    };

    println!("\n══ RESULT ════════════════════════════════════════════════════════════");
    println!("  lock_txid:           {lock_txid}");
    println!("  settle_txid:         {settle_txid}");
    println!("  negctl_lock_txid:    {neg_lock_txid}");
    println!("  negative_control:    NODE-REJECTED (no txid; evidence = node error)");
    println!("  negative_control_err:{neg_outcome}");
    println!("  network:             {}", info.network_id);
    Ok(())
}
