//! Testnet evidence runner for the vault pattern.
//!
//! Demonstrates compiling a spending condition to a real Kaspa script,
//! computing its digest, and anchoring the digest on-chain in a carrier
//! transaction on the Kaspa testnet. Also evaluates the condition offline
//! in pre- and post-deadline states to illustrate the pure evaluator.
//!
//! ## Usage
//!
//! ```text
//! KCP_NODE_URL=ws://localhost:17210   \
//! KCP_KEY_FILE=/path/to/wallet.key   \
//! cargo run -p kcp-vault --example testnet_evidence \
//!            --features wrpc
//! ```
//!
//! Optional:
//! - `KCP_TS` — ISO-8601 timestamp for reproducibility across runs.
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
    canonical::canonical_hash,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};
use kcp_vault::{
    condition::SpendCondition,
    evaluator::{evaluate, EvalContext},
    payload::Payload,
    script::{compile_condition, vault_script_digest},
    tx::{anchor_vault_tx, DEFAULT_VAULT_VALUE_SOMPI},
};

type BoxError = Box<dyn std::error::Error>;

/// A documented fixed test xonly key (all-zero except byte 0 = 0x02).
///
/// This is a non-secret key used only in the testnet evidence example to
/// form a 2-of-2 multisig alongside the wallet key. It has no spending
/// authority over real funds.
const FIXED_TEST_XONLY: [u8; 32] = {
    let mut k = [0u8; 32];
    k[0] = 0x02;
    k
};

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // ---- read environment ------------------------------------------------
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| {
        eprintln!("KCP_NODE_URL not set; defaulting to ws://localhost:17210");
        "ws://localhost:17210".to_string()
    });
    let key_file = env::var("KCP_KEY_FILE")
        .map_err(|_| "KCP_KEY_FILE is required — path to a testnet wallet key file")?;
    let _ts_field = env::var("KCP_TS").ok();

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
    println!("wallet:           {}", wallet.address_string());

    // ---- current Unix timestamp ------------------------------------------
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before Unix epoch")
        .as_secs();

    // ---- build the spending condition ------------------------------------
    //
    // Any(
    //   TimelockUnixSeconds { deadline = now+3600, controller_xonly = wallet_key },
    //   MultiSig { threshold=2, keys=[wallet_key, FIXED_TEST_XONLY] }
    // )
    //
    // If Any(2) were unsupported, the timelock leaf would be used instead.
    // In v0 Any(exactly 2) IS supported, so the composite is used here.

    let wallet_xonly = wallet.keypair.x_only_public_key().0.serialize();
    let deadline = now_secs + 3600;

    let timelock_branch = SpendCondition::TimelockUnixSeconds {
        deadline,
        controller_xonly: wallet_xonly,
    };

    let multisig_branch = SpendCondition::MultiSig {
        threshold: 2,
        xonly_keys: vec![wallet_xonly, FIXED_TEST_XONLY],
    };

    let condition = SpendCondition::Any {
        children: vec![timelock_branch, multisig_branch],
    };
    condition
        .validate()
        .map_err(|e| format!("condition invalid: {e}"))?;

    println!("\n--- CONDITION ---");
    println!("{}", serde_json::to_string_pretty(&condition)?);

    // ---- compile the script -----------------------------------------------
    let script_bytes =
        compile_condition(&condition).map_err(|e| format!("compile_condition: {e}"))?;
    let digest = vault_script_digest(&script_bytes);
    println!("\n--- SCRIPT ---");
    println!("script bytes:     {} bytes", script_bytes.len());
    println!("script (hex):     {}", hex::encode(&script_bytes));
    println!("script_digest:    {}", hex::encode(digest));

    // ---- compute vault_id ------------------------------------------------
    let vault_id = canonical_hash(&condition).map_err(|e| format!("canonical_hash: {e}"))?;
    println!("vault_id:         {}", hex::encode(vault_id));

    // ---- build and encode payload ----------------------------------------
    let payload = Payload {
        vault_id,
        script_digest: digest,
    };
    let payload_bytes = payload.encode();
    println!("payload (hex):    {}", hex::encode(&payload_bytes));

    // ---- offline evaluation — pre-deadline (neither branch passes) -------
    println!("\n--- OFFLINE EVALUATION ---");
    let ctx_pre = EvalContext {
        daa_score: info.virtual_daa_score,
        unix_seconds: now_secs - 1, // one second before deadline
        signers_present: vec![],    // no signers
    };
    let pre_result = evaluate(&condition, &ctx_pre);
    println!(
        "pre-deadline (unix_seconds={}, no signers): {}",
        ctx_pre.unix_seconds, pre_result
    );
    assert!(!pre_result, "expected pre-deadline eval to be false");

    // Post-deadline with wallet key present — timelock branch passes.
    let ctx_post_tl = EvalContext {
        daa_score: info.virtual_daa_score,
        unix_seconds: deadline, // exactly at deadline
        signers_present: vec![],
    };
    let post_tl_result = evaluate(&condition, &ctx_post_tl);
    println!(
        "post-deadline (unix_seconds={}, no signers): {}",
        ctx_post_tl.unix_seconds, post_tl_result
    );
    assert!(post_tl_result, "expected post-deadline timelock to pass");

    // Multisig branch — both keys present (before deadline).
    let ctx_multisig = EvalContext {
        daa_score: info.virtual_daa_score,
        unix_seconds: now_secs - 1,
        signers_present: vec![wallet_xonly, FIXED_TEST_XONLY],
    };
    let multisig_result = evaluate(&condition, &ctx_multisig);
    println!(
        "pre-deadline, both multisig keys present: {}",
        multisig_result
    );
    assert!(multisig_result, "expected 2-of-2 multisig to pass");

    // ---- anchor on-chain --------------------------------------------------
    println!("\n--- ANCHOR VAULT (evidence tx) ---");
    let tx_id = anchor_vault_tx(&rpc, &wallet, payload_bytes, DEFAULT_VAULT_VALUE_SOMPI)
        .await
        .map_err(|e| format!("anchor_vault_tx: {e}"))?;
    println!("tx_id:            {tx_id}");
    println!("lookup:           tx {tx_id} on {}", info.network_id);

    // ---- FACTS.yaml-ready output -----------------------------------------
    let vault_id_hex = hex::encode(vault_id);
    let digest_hex = hex::encode(digest);
    println!("\n--- FACTS.yaml-ready ---");
    println!("  id: KCP-VT-001");
    println!(
        "  claim: \"kcp-vault v0 pattern exercised on {}: \
         vault_id={vault_id_hex}, script_digest={digest_hex}, tx={tx_id}\"",
        info.network_id
    );
    println!("  source: examples/testnet_evidence.rs");
    println!("  verified_at: [FACT-NEEDED: fill in today's date]");
    println!(
        "  note: v0 — script compiled + digest anchored; \
         value not yet locked under script (P2SH not yet implemented)"
    );

    Ok(())
}
