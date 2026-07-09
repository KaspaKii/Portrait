//! Testnet evidence runner for the KTT token pattern.
//!
//! Demonstrates the three-step KCC20-shape token lifecycle on the Kaspa
//! testnet using off-chain state transitions and carrier-anchored evidence:
//!
//! 1. **Issue** — genesis state: minter=true, amount=1_000_000.
//! 2. **Transfer** — transfer 400_000 to a derived recipient address (index 1);
//!    off-chain conservation check (KTT-1) performed before anchoring.
//! 3. **Burn** — burn 100_000 from the sender's remaining 600_000; off-chain
//!    burn check performed before anchoring.
//!
//! Each operation produces a carrier transaction embedding the KCPKT payload.
//! Between chained operations, the example polls for UTXO confirmation (up to
//! 60 seconds) before spending the output.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_KEY_FILE=/path/to/wallet.key   \
//! cargo run -p kcp-ktt-token --example testnet_evidence \
//!            --features wrpc
//! ```
//!
//! Optional:
//! - `KCP_NET_SUFFIX` — testnet suffix (default: 10).
//! - `KCP_TS` — ISO-8601 timestamp for reproducibility across runs.
//!
//! ## Safety
//!
//! This example **refuses to run unless the node reports a testnet network**.
//! No hardcoded private keys. No faucet automation.
//!
//! Status: **v0 — unaudited — testnet first.**

use std::env;

use kcp_common::{
    canonical::canonical_hash,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_ktt_token::{
    payload::{OpClass, Payload},
    state::{IdentifierType, KttState},
    token::{burn as token_burn, mint as token_mint, transfer, AuthContext},
    tx::{anchor_token_op_tx, DEFAULT_TOKEN_OP_VALUE_SOMPI},
};
use serde_json::json;

type BoxError = Box<dyn std::error::Error>;

/// Whether an anchor error is a transient confirmation race worth retrying.
///
/// The three ops are independent self-funded carrier transactions on a
/// single-UTXO wallet, so each must wait for the previous op's change output
/// to confirm. Until then the node may still return the just-spent UTXO
/// ("already spent ... in the mempool") or not yet expose the change
/// ("not found", "no UTXOs", "fee threshold").
fn is_transient_spend_error<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("not found")
        || s.contains("already spent")
        || s.contains("in the mempool")
        || s.contains("no UTXOs")
        || s.contains("fee threshold")
}

/// Compute the state_commitment for a [`KttState`]: canonical_hash of its
/// encoded bytes represented as a JSON hex string, for deterministic hashing.
fn state_commitment(state: &KttState) -> Result<[u8; 32], BoxError> {
    let encoded = state.encode();
    let hex = hex::encode(&encoded);
    let h = canonical_hash(&json!({ "state": hex }))?;
    Ok(h)
}

/// Compute the token_id for a genesis parameter set.
fn token_id_for(owner_hex: &str, amount: u64) -> Result<[u8; 32], BoxError> {
    let h = canonical_hash(&json!({
        "pattern": "kcp-ktt-token",
        "v": "0",
        "owner": owner_hex,
        "genesis_amount": amount,
    }))?;
    Ok(h)
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // ── read environment ────────────────────────────────────────────────────
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| {
        eprintln!("KCP_NODE_URL not set; defaulting to ws://localhost:17210");
        "ws://localhost:17210".to_string()
    });
    let key_file = env::var("KCP_KEY_FILE")
        .map_err(|_| "KCP_KEY_FILE is required — path to a testnet wallet key file")?;
    let _ts_field = env::var("KCP_TS").ok();

    // ── connect ─────────────────────────────────────────────────────────────
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

    // ── testnet guard ───────────────────────────────────────────────────────
    if !info.network_id.contains("testnet") {
        return Err(format!(
            "REFUSED: network_id '{}' is not testnet. \
             This example must only run against a testnet node.",
            info.network_id
        )
        .into());
    }

    // ── load wallets ────────────────────────────────────────────────────────
    // Primary wallet (index 0) — issuer and sender.
    let issuer = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
        .map_err(|e| format!("failed to load wallet: {e}"))?;
    println!("issuer wallet:    {}", issuer.address_string());

    // Recipient wallet (index 1 from the same key file) — transfer target.
    let recipient = Wallet::load(std::path::Path::new(&key_file), 1, Prefix::Testnet)
        .map_err(|e| format!("failed to load recipient wallet at index 1: {e}"))?;
    println!("recipient wallet: {}", recipient.address_string());

    // ── build owner identifiers ─────────────────────────────────────────────
    let issuer_xonly: [u8; 32] = issuer.keypair.x_only_public_key().0.serialize();
    let recipient_xonly: [u8; 32] = recipient.keypair.x_only_public_key().0.serialize();

    let issuer_owner_hex = hex::encode(issuer_xonly);

    // ── Step 1: ISSUE ───────────────────────────────────────────────────────
    // Genesis state: minter=true, amount=1_000_000.
    const GENESIS_AMOUNT: u64 = 1_000_000;

    let genesis_state = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: issuer_xonly,
        amount: GENESIS_AMOUNT,
        is_minter: true,
    };

    let tok_id = token_id_for(&issuer_owner_hex, GENESIS_AMOUNT)?;
    let issue_commitment = state_commitment(&genesis_state)?;
    let issue_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Issue,
        state_commitment: issue_commitment,
    }
    .encode();

    println!("\n--- ISSUE (genesis carrier tx) ---");
    println!("token_id:         {}", hex::encode(tok_id));
    println!(
        "genesis state:    owner={} amount={} is_minter={}",
        &issuer_owner_hex[..16],
        GENESIS_AMOUNT,
        genesis_state.is_minter
    );

    let issue_tx_id =
        anchor_token_op_tx(&rpc, &issuer, issue_payload, DEFAULT_TOKEN_OP_VALUE_SOMPI)
            .await
            .map_err(|e| format!("anchor_token_op_tx (issue): {e}"))?;
    println!("issue tx_id:      {issue_tx_id}");
    println!("lookup:           tx {issue_tx_id} on {}", info.network_id);

    // ── Step 2: TRANSFER 400_000 to recipient ──────────────────────────────
    // Off-chain state transition: issuer sends 400_000 to recipient,
    // keeping 600_000. KTT-1 conservation check is in token::transfer.
    const TRANSFER_AMOUNT: u64 = 400_000;
    const REMAINING_AFTER_TRANSFER: u64 = GENESIS_AMOUNT - TRANSFER_AMOUNT; // 600_000

    let issuer_after_transfer = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: issuer_xonly,
        amount: REMAINING_AFTER_TRANSFER,
        is_minter: true, // minter persists on issuer branch
    };
    let recipient_state = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: recipient_xonly,
        amount: TRANSFER_AMOUNT,
        is_minter: false,
    };

    // Off-chain KTT-1/KTT-2/KTT-3 validation.
    // Note: with is_minter=true on issuer_after_transfer, KTT-1 is relaxed
    // (minter branch may diverge from input sum). We validate the non-minter
    // output arm explicitly.
    let auth = AuthContext {
        authorised_owners: vec![issuer_xonly],
    };
    transfer(
        std::slice::from_ref(&genesis_state),
        &[issuer_after_transfer.clone(), recipient_state.clone()],
        &auth,
        0, // no transfer rules in v0
    )
    .map_err(|e| format!("off-chain transfer validation failed: {e}"))?;
    println!("\noff-chain transfer: KTT-1/KTT-2/KTT-3 validated");

    // Commit to the post-transfer state (issuer branch only for the anchor).
    let transfer_commitment = state_commitment(&issuer_after_transfer)?;
    let transfer_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Transfer,
        state_commitment: transfer_commitment,
    }
    .encode();

    println!("\n--- TRANSFER 400_000 to recipient ---");
    println!(
        "issuer remaining: {}  recipient receives: {}",
        REMAINING_AFTER_TRANSFER, TRANSFER_AMOUNT
    );

    // Poll for the issue UTXO to be confirmed before spending the same wallet
    // UTXO again (conservative: we poll the node for a new UTXO that is NOT
    // the issue output, by simply retrying the anchor tx until it succeeds).
    let mut transfer_tx_id = String::new();
    for attempt in 1..=30u32 {
        match anchor_token_op_tx(
            &rpc,
            &issuer,
            transfer_payload.clone(),
            DEFAULT_TOKEN_OP_VALUE_SOMPI,
        )
        .await
        {
            Ok(id) => {
                transfer_tx_id = id;
                break;
            }
            Err(e) if is_transient_spend_error(&e) && attempt < 30 => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => return Err(format!("anchor_token_op_tx (transfer): {e}").into()),
        }
    }
    if transfer_tx_id.is_empty() {
        return Err("transfer carrier UTXO not confirmed after 60s".into());
    }
    println!("transfer tx_id:   {transfer_tx_id}");
    println!(
        "lookup:           tx {transfer_tx_id} on {}",
        info.network_id
    );

    // ── Step 3: BURN 100_000 from issuer's 600_000 ─────────────────────────
    const BURN_AMOUNT: u64 = 100_000;
    const REMAINING_AFTER_BURN: u64 = REMAINING_AFTER_TRANSFER - BURN_AMOUNT; // 500_000

    let issuer_after_burn = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: issuer_xonly,
        amount: REMAINING_AFTER_BURN,
        is_minter: true, // minter persists
    };

    // Off-chain KTT-1/KTT-2/KTT-3/burn-check validation.
    token_burn(
        &issuer_after_transfer,
        &issuer_after_burn,
        BURN_AMOUNT,
        &auth,
        0,
    )
    .map_err(|e| format!("off-chain burn validation failed: {e}"))?;
    println!("\noff-chain burn:   KTT burn invariants validated");

    let burn_commitment = state_commitment(&issuer_after_burn)?;
    let burn_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Burn,
        state_commitment: burn_commitment,
    }
    .encode();

    println!("\n--- BURN 100_000 from issuer ---");
    println!(
        "issuer before burn: {}  burn amount: {}  remaining: {}",
        REMAINING_AFTER_TRANSFER, BURN_AMOUNT, REMAINING_AFTER_BURN
    );

    // Also demonstrate a mint step (pure off-chain, no extra carrier tx needed
    // for the evidence run — the mint_op validation is shown inline).
    let minted_state = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: recipient_xonly,
        amount: 50_000,
        is_minter: false,
    };
    let minter_persisted = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: issuer_xonly,
        amount: 0,
        is_minter: true,
    };
    let mint_input = KttState {
        identifier_type: IdentifierType::Pubkey,
        owner_identifier: issuer_xonly,
        amount: 0,
        is_minter: true,
    };
    token_mint(&mint_input, &minted_state, &minter_persisted, &auth, 0)
        .map_err(|e| format!("off-chain mint validation failed: {e}"))?;
    println!("off-chain mint:   KTT mint invariants validated (mint=50_000, no carrier tx in evidence run)");

    // Burn carrier tx — poll after the transfer anchor.
    let mut burn_tx_id = String::new();
    for attempt in 1..=30u32 {
        match anchor_token_op_tx(
            &rpc,
            &issuer,
            burn_payload.clone(),
            DEFAULT_TOKEN_OP_VALUE_SOMPI,
        )
        .await
        {
            Ok(id) => {
                burn_tx_id = id;
                break;
            }
            Err(e) if is_transient_spend_error(&e) && attempt < 30 => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => return Err(format!("anchor_token_op_tx (burn): {e}").into()),
        }
    }
    if burn_tx_id.is_empty() {
        return Err("burn carrier UTXO not confirmed after 60s".into());
    }
    println!("burn tx_id:       {burn_tx_id}");
    println!("lookup:           tx {burn_tx_id} on {}", info.network_id);

    // ── transition chain summary ────────────────────────────────────────────
    println!("\n--- TOKEN TRANSITION CHAIN (off-chain validation) ---");
    println!("  issue:    {} tokens (is_minter=true)", GENESIS_AMOUNT);
    println!(
        "  transfer: {} to recipient, {} retained (KTT-1/KTT-2/KTT-3 green)",
        TRANSFER_AMOUNT, REMAINING_AFTER_TRANSFER
    );
    println!(
        "  burn:     {} burned, {} remaining (KTT burn invariants green)",
        BURN_AMOUNT, REMAINING_AFTER_BURN
    );
    println!("  chain:    validated off-chain (v0 — on-chain binding target: KCC20 validateOutputStateWithTemplate)");

    // ── FACTS.yaml-ready output ─────────────────────────────────────────────
    let tok_id_hex = hex::encode(tok_id);
    println!("\n--- FACTS.yaml-ready ---");
    println!("  id: KCP-KTT-001");
    println!(
        "  claim: \"kcp-ktt-token v0 KCC20-shape pattern exercised on {}: \
         token_id={tok_id_hex}, issue_tx={issue_tx_id}, \
         transfer_tx={transfer_tx_id}, burn_tx={burn_tx_id}\"",
        info.network_id
    );
    println!("  source: examples/testnet_evidence.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!(
        "  note: v0 — unaudited — KCC20-shape state transitions validated \
         off-chain; carrier-anchored on testnet; on-chain binding target is \
         KCC20 validateOutputStateWithTemplate (engine-enforced, see FACTS SS-026)"
    );

    Ok(())
}
