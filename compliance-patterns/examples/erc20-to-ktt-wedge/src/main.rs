//! ERC20 → KTT wedge demo — Kii Rosetta on-ramp.
//!
//! Shows the full path: familiar ERC20-shaped API (kii-solidity-compat) →
//! KTT state-machine transitions → carrier-anchored evidence on TN10.
//!
//! Each step below maps to an ERC20 concept:
//!   `Token::new`       → `ERC20(name, symbol)` constructor
//!   `initial_mint`     → `_mint(to, amount)` in the constructor
//!   `transfer`         → `transfer(to, amount)` — UTXO-spend shape
//!   `burn`             → `_burn(from, amount)`
//!
//! What is absent (by design — see MIGRATING-FROM-SOLIDITY.md §3):
//!   `balanceOf`        → sum KTT UTXOs from kaspad
//!   `approve`          → model as a covenant spend condition
//!   `transferFrom`     → no allowance object in the UTXO model
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210  \
//! KCP_KEY_FILE=/path/to/wallet.key  \
//! cargo run --manifest-path examples/erc20-to-ktt-wedge/Cargo.toml
//! ```
//!
//! Optional:
//!   KCP_NET_SUFFIX — testnet suffix (default: 10).
//!
//! The key file must contain a 64-char hex private key or a BIP-39 mnemonic.
//! Fund the issuer address from a TN10 faucet before running (≥ 0.5 KAS).
//!
//! ## Safety
//!
//! Refuses to run against a non-testnet node.
//! No hardcoded private keys. No faucet automation.
//!
//! Status: v0 — pre-production — unaudited — testnet-only.

use std::sync::Arc;
use std::env;

use kaspa_wrpc_client::KaspaRpcClient;
use kcp_common::{
    canonical::canonical_hash,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_ktt_token::{
    payload::{OpClass, Payload},
    state::KttState,
    tx::{anchor_token_op_tx, DEFAULT_TOKEN_OP_VALUE_SOMPI},
};
use kii_solidity_compat::erc20::Token;
use serde_json::json;

type BoxError = Box<dyn std::error::Error>;

/// Whether an anchor error is a transient confirmation race worth retrying.
fn is_transient_spend_error<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("not found")
        || s.contains("already spent")
        || s.contains("in the mempool")
        || s.contains("no UTXOs")
        || s.contains("fee threshold")
        || s.contains("timeout")
        || s.contains("timed out")
}

/// Canonical state commitment: hash(encode(state)) used as the on-chain anchor.
fn state_commitment(state: &KttState) -> Result<[u8; 32], BoxError> {
    let encoded = state.encode();
    let hex = hex::encode(&encoded);
    let h = canonical_hash(&json!({ "state": hex }))?;
    Ok(h)
}

/// Deterministic token_id for this genesis parameter set.
fn token_id_for(owner_hex: &str, amount: u64) -> Result<[u8; 32], BoxError> {
    let h = canonical_hash(&json!({
        "pattern": "kcp-ktt-token",
        "v": "0",
        "owner": owner_hex,
        "genesis_amount": amount,
    }))?;
    Ok(h)
}

/// Retry `anchor_token_op_tx` up to 30×, sleeping 2s on transient errors.
async fn anchor_with_retry(
    rpc: &Arc<KaspaRpcClient>,
    wallet: &Wallet,
    payload: Vec<u8>,
    label: &str,
) -> Result<String, BoxError> {
    for attempt in 1..=60u32 {
        match anchor_token_op_tx(rpc.as_ref(), wallet, payload.clone(), DEFAULT_TOKEN_OP_VALUE_SOMPI).await {
            Ok(id) => return Ok(id),
            Err(e) if is_transient_spend_error(&e) && attempt < 60 => {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(e) => {
                return Err(format!("anchor_token_op_tx ({label}) attempt {attempt}: {e}").into())
            }
        }
    }
    Err(format!("{label} carrier UTXO not confirmed after 60s").into())
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // ── env ────────────────────────────────────────────────────────────────────
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| {
        eprintln!("KCP_NODE_URL not set; defaulting to ws://localhost:17210");
        "ws://localhost:17210".to_string()
    });
    let key_file = env::var("KCP_KEY_FILE")
        .map_err(|_| "KCP_KEY_FILE is required — path to a testnet wallet key file")?;
    let recipient_key_file = env::var("KCP_RECIPIENT_KEY_FILE").unwrap_or_else(|_| key_file.clone());
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    // ── connect ────────────────────────────────────────────────────────────────
    let config = NodeConfig::testnet(&node_url, net_suffix);
    let node = NodeClient::new(config);
    let rpc = node.rpc().await?;

    let info = node.server_info().await?;
    println!(
        "connected: server={} network={} synced={} daa={}",
        info.server_version, info.network_id, info.is_synced, info.virtual_daa_score
    );

    if !info.network_id.contains("testnet") {
        return Err(format!(
            "REFUSED: network_id '{}' is not testnet. This demo only runs against TN10.",
            info.network_id
        )
        .into());
    }

    // ── load wallet ────────────────────────────────────────────────────────────
    let issuer = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("failed to load wallet: {e}"))?;
    let recipient = Wallet::load(std::path::Path::new(&recipient_key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("failed to load recipient wallet: {e}"))?;
    let issuer_xonly: [u8; 32] = issuer.keypair.x_only_public_key().0.serialize();
    let recipient_xonly: [u8; 32] = recipient.keypair.x_only_public_key().0.serialize();
    let issuer_owner_hex = hex::encode(issuer_xonly);

    println!("issuer:    {}", issuer.address_string());
    println!("recipient: {}", recipient.address_string());

    // ── [1] Token::new — ERC20(name, symbol) constructor ──────────────────────
    //
    // Unlike ERC20 this returns a descriptor; there is no on-chain deployment.
    // The minter_key is the issuer's 32-byte x-only Schnorr public key.
    let token = Token::new("HelloKTT", "HKT", 8, issuer_xonly);
    println!("\n=== HelloKTT (HKT) — ERC20 → KTT wedge demo ===");
    println!("    decimals:  {}", token.decimals);

    // ── [2] initial_mint — ERC20 `_mint(to, amount)` ──────────────────────────
    //
    // Returns (holder_state, minter_state). The holder_state carries the issued
    // tokens (is_minter=false). The minter_state persists signing authority
    // (is_minter=true, amount=0). Unlike ERC20 there is no shared `totalSupply`;
    // supply is covenant-enforced by the holder UTXO.
    const GENESIS_AMOUNT: u64 = 1_000_000;
    let (holder_state, _minter_state) = token.initial_mint(issuer_xonly, GENESIS_AMOUNT)?;
    println!("\n--- [1] initial_mint ({GENESIS_AMOUNT} HKT) ---");
    println!("    holder amount:    {}", holder_state.amount);
    println!("    holder is_minter: {}", holder_state.is_minter);

    let tok_id = token_id_for(&issuer_owner_hex, GENESIS_AMOUNT)?;
    let issue_commitment = state_commitment(&holder_state)?;
    let issue_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Issue,
        state_commitment: issue_commitment,
    }
    .encode();
    println!("    token_id:         {}", hex::encode(tok_id));

    let issue_tx_id = anchor_with_retry(&rpc, &issuer, issue_payload, "issue").await?;
    println!("    issue tx_id:      {issue_tx_id}");
    println!("    lookup:           tx {issue_tx_id} on {}", info.network_id);

    // ── [3] transfer — ERC20 `transfer(to, amount)` ───────────────────────────
    //
    // Returns (to_state, change_state). Unlike ERC20, you pass the UTXO
    // being spent (`holder_state`), not `msg.sender`. The change_state carries
    // the issuer's remaining tokens.
    const TRANSFER_AMOUNT: u64 = 400_000;
    let (to_state, change_state) = token.transfer(&holder_state, recipient_xonly, TRANSFER_AMOUNT)?;
    println!("\n--- [2] transfer ({TRANSFER_AMOUNT} HKT to recipient) ---");
    println!(
        "    to:     {} HKT  change: {} HKT",
        to_state.amount, change_state.amount
    );

    let transfer_commitment = state_commitment(&change_state)?;
    let transfer_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Transfer,
        state_commitment: transfer_commitment,
    }
    .encode();

    let transfer_tx_id =
        anchor_with_retry(&rpc, &issuer, transfer_payload, "transfer").await?;
    println!("    transfer tx_id:   {transfer_tx_id}");
    println!(
        "    lookup:           tx {transfer_tx_id} on {}",
        info.network_id
    );

    // ── [4] burn — ERC20 `_burn(from, amount)` ────────────────────────────────
    //
    // Returns the remaining UTXO state after burning `burn_amount` from
    // `change_state`. Unlike ERC20 there is no totalSupply to decrement;
    // the burned KTT UTXO is simply spent to nothing.
    const BURN_AMOUNT: u64 = 100_000;
    let remaining = token.burn(&change_state, BURN_AMOUNT)?;
    println!("\n--- [3] burn ({BURN_AMOUNT} HKT) ---");
    println!(
        "    before: {} HKT  burned: {}  remaining: {}",
        change_state.amount, BURN_AMOUNT, remaining.amount
    );

    let burn_commitment = state_commitment(&remaining)?;
    let burn_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Burn,
        state_commitment: burn_commitment,
    }
    .encode();

    let burn_tx_id = anchor_with_retry(&rpc, &issuer, burn_payload, "burn").await?;
    println!("    burn tx_id:       {burn_tx_id}");
    println!("    lookup:           tx {burn_tx_id} on {}", info.network_id);

    // ── FACTS-ready summary ────────────────────────────────────────────────────
    let tok_id_hex = hex::encode(tok_id);
    println!("\n--- FACTS-ready ---");
    println!(
        "  id: KCP-ERC20-WEDGE-001\n  \
         claim: \"kii-solidity-compat ERC20→KTT wedge on {network}: \
         token_id={tok_id_hex}, issue_tx={issue_tx_id}, \
         transfer_tx={transfer_tx_id}, burn_tx={burn_tx_id}\"\n  \
         source: examples/erc20-to-ktt-wedge/src/main.rs\n  \
         verified_at: [FACT-NEEDED: fill in today's date]\n  \
         note: v0 — unaudited — ERC20-shaped kii-solidity-compat API over \
         kcp-ktt-token; carrier-anchored on testnet; on-chain covenant \
         binding is the next step (KCC20 validateOutputStateWithTemplate)",
        network = info.network_id
    );

    Ok(())
}
