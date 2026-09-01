//! CSCI demo — Covenant-Settled Compliance Instrument on TN10.
//!
//! Demonstrates the full CSCI state-machine data flow:
//!   1. Genesis: compute and anchor the CSCI instrument's genesis state
//!   2. Settlement: compute a compliant transfer journal and anchor it on TN10
//!   3. Negative control: tamper the journal → different commitment (would be
//!      rejected by OpZkPrecompile if embedded in a full STARK settlement)
//!
//! On-chain evidence is via OP_RETURN carrier transactions (journal hash
//! records). The full ZK proof path (OpZkPrecompile binding) requires running
//! the kii-csci-prover workspace separately — the prover builds and the
//! journal schema is identical.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210  \
//! KCP_KEY_FILE=/path/to/wallet.key  \
//! cargo run --manifest-path examples/csci-demo/Cargo.toml
//! ```
//!
//! Fund the wallet (≥ 0.5 KAS) from a TN10 faucet before running.
//! Status: v0 — pre-production — unaudited — testnet-only.

use std::env;

use kcp_common::{
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_csci::{CsciState, CsciStateTransition};
use kcp_ktt_token::tx::{anchor_token_op_tx, DEFAULT_TOKEN_OP_VALUE_SOMPI};
use sha2::{Digest, Sha256};

type BoxError = Box<dyn std::error::Error>;

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Whether an anchor error is a transient confirmation race worth retrying.
fn is_transient(s: &str) -> bool {
    s.contains("not found")
        || s.contains("already spent")
        || s.contains("in the mempool")
        || s.contains("no UTXOs")
        || s.contains("fee threshold")
        || s.contains("timeout")
        || s.contains("timed out")
}

/// Retry anchor_token_op_tx up to 60×, sleeping 3s on transient errors.
async fn anchor(
    rpc: &kaspa_wrpc_client::KaspaRpcClient,
    wallet: &Wallet,
    payload: Vec<u8>,
    label: &str,
) -> Result<String, BoxError> {
    for attempt in 1..=60u32 {
        match anchor_token_op_tx(rpc, wallet, payload.clone(), DEFAULT_TOKEN_OP_VALUE_SOMPI).await {
            Ok(id) => return Ok(id),
            Err(e) if is_transient(&e.to_string()) && attempt < 60 => {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(e) => {
                return Err(format!("{label} attempt {attempt}: {e}").into())
            }
        }
    }
    Err(format!("{label} not confirmed after 60s").into())
}

/// Build the CSCI anchor payload for on-chain recording:
/// b"KCP-CSCI" (8B) || tag (1B) || data (up to 55B) → max 64B
fn csci_payload(tag: u8, data: &[u8]) -> Vec<u8> {
    let mut v = b"KCP-CSCI".to_vec();
    v.push(tag);
    v.extend_from_slice(data);
    v
}

const TAG_GENESIS:     u8 = 0x00;
const TAG_SETTLEMENT:  u8 = 0x01;
const TAG_NEG_CONTROL: u8 = 0xff;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| {
        eprintln!("KCP_NODE_URL not set; defaulting to ws://localhost:17210");
        "ws://localhost:17210".to_string()
    });
    let key_file = env::var("KCP_KEY_FILE")
        .map_err(|_| "KCP_KEY_FILE is required — path to a testnet wallet key file")?;
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let wallet = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)?;
    let cfg = NodeConfig::testnet(&node_url, net_suffix);
    let client = NodeClient::new(cfg);
    let rpc = client.rpc().await?;

    println!("CSCI demo — wallet address: {}", wallet.address);
    println!("Connected to: {}", node_url);

    // ── CSCI parameters ───────────────────────────────────────────────────────
    // In production: covenant_id is the KIP-20 covenant-ID of the genesis UTXO.
    // Here we use sha256("csci-demo-v0" || wallet_pubkey) as a deterministic ID.
    let owner_pk_bytes = wallet.keypair.x_only_public_key().0.serialize();
    let covenant_id: [u8; 32] = sha256(&[b"csci-demo-v0".as_ref(), owner_pk_bytes.as_ref()].concat());
    let rule_bytes = b"csci-v0-allow-partial-transfer";
    let rule_hash: [u8; 32] = sha256(rule_bytes);
    let genesis_amount: u64 = 1_000_000_000; // 10 KAS in sompi

    // Genesis state: the CSCI instrument starts here.
    let mut owner_pk = [0u8; 32];
    owner_pk.copy_from_slice(&owner_pk_bytes);
    let genesis = CsciState::new_genesis(
        owner_pk,
        genesis_amount,
        rule_hash,
        covenant_id,
    );
    let genesis_hash = genesis.state_hash();

    println!("\n── CSCI PARAMETERS ──────────────────────────────────────────────────────");
    println!("covenant_id : {}", hex::encode(covenant_id));
    println!("rule_hash   : {}", hex::encode(rule_hash));
    println!("genesis_hash: {}", hex::encode(genesis_hash));
    println!("genesis seq : {}", genesis.seq);

    // ── Step 1: Genesis anchor ────────────────────────────────────────────────
    // Payload: b"KCP-CSCI" 0x00 covenant_id(32) — 41 bytes total.
    println!("\n[1/3] Anchoring CSCI genesis on TN10...");
    let genesis_payload = csci_payload(TAG_GENESIS, &covenant_id);
    let txid_genesis = anchor(rpc.as_ref(), &wallet, genesis_payload, "CSCI genesis").await?;
    println!("      CSCI genesis anchor txid: {}", txid_genesis);

    // ── Step 2: Settlement journal ────────────────────────────────────────────
    // Compute a compliant transfer: from genesis owner → recipient, 500M sompi.
    // In production the compliance check is the RISC Zero vProg; here we call
    // CsciStateTransition::transfer() directly — same validation logic.
    let recipient_pk: [u8; 32] = sha256(b"csci-demo-recipient-v0");
    let transfer_amount: u64 = 500_000_000; // 5 KAS

    let transition = CsciStateTransition::transfer(
        &genesis,
        recipient_pk,
        transfer_amount,
        rule_hash,
    )?;
    let journal_bytes = transition.journal_bytes();
    let journal_hash  = transition.journal_hash();
    let new_state_hash = transition.new_state.state_hash();

    println!("\n── SETTLEMENT JOURNAL (104 bytes) ───────────────────────────────────────");
    println!("journal     : {}", hex::encode(&journal_bytes));
    println!("journal_hash: {}", hex::encode(journal_hash));
    println!("new_seq     : {}", transition.new_state.seq);
    println!("new_state_h : {}", hex::encode(new_state_hash));

    println!("\n[2/3] Anchoring CSCI settlement journal on TN10...");
    let settle_payload = csci_payload(TAG_SETTLEMENT, &journal_hash);
    let txid_settle = anchor(rpc.as_ref(), &wallet, settle_payload, "CSCI settlement").await?;
    println!("      CSCI settlement txid: {}", txid_settle);

    // ── Step 3: Negative control ──────────────────────────────────────────────
    // Tamper the journal: flip bit 0 of seq field (bytes 96..104).
    // The tampered journal produces a DIFFERENT journal_hash.
    // In full ZK mode: OpZkPrecompile would reject the proof because the
    // journal commitment doesn't match the proof's claimed journal_hash.
    let mut tampered = journal_bytes.clone();
    tampered[96] ^= 0x01; // flip LSB of seq
    let tampered_journal_hash: [u8; 32] = sha256(&tampered);

    println!("\n── NEGATIVE CONTROL ─────────────────────────────────────────────────────");
    println!("tampered_h  : {}", hex::encode(tampered_journal_hash));
    println!("original_h  : {}", hex::encode(journal_hash));
    println!("hashes_match: {} (should be false)", tampered_journal_hash == journal_hash);
    assert_ne!(tampered_journal_hash, journal_hash, "tampered journal must differ");

    println!("\n[3/3] Anchoring negative-control (tampered journal hash) on TN10...");
    let neg_payload = csci_payload(TAG_NEG_CONTROL, &tampered_journal_hash);
    let txid_neg = anchor(rpc.as_ref(), &wallet, neg_payload, "CSCI negative control").await?;
    println!("      CSCI neg-control txid: {}", txid_neg);

    // ── Provenance summary ────────────────────────────────────────────────────
    println!("\n══ CSCI PROVENANCE ══════════════════════════════════════════════════════");
    println!("covenant_id    : {}", hex::encode(covenant_id));
    println!("rule_hash      : {}", hex::encode(rule_hash));
    println!("genesis_hash   : {}", hex::encode(genesis_hash));
    println!("settlement_hash: {}", hex::encode(journal_hash));
    println!("neg_ctrl_hash  : {}", hex::encode(tampered_journal_hash));
    println!();
    println!("[KCP-CSCI-GEN-001]  genesis anchor   : {}", txid_genesis);
    println!("[KCP-CSCI-SETTLE-001] settlement       : {}", txid_settle);
    println!("[KCP-CSCI-NEG-001]  neg-control       : {}", txid_neg);
    println!();
    println!("✓ CSCI demo complete — all three TN10 anchors confirmed.");
    println!();
    println!("Next step: run kii-csci-prover to generate a real RISC Zero succinct STARK,");
    println!("then use build_csci_redeem() to embed the proof in a P2SH locking script.");
    println!("See docs/FLAGSHIP-DESIGN.md §3 for the full OpZkPrecompile settlement path.");

    Ok(())
}
