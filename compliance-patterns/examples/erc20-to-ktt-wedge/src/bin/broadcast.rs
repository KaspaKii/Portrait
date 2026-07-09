//! REST broadcast fallback for the ERC20→KTT wedge demo.
//!
//! Submits the KTT issue/transfer/burn carrier txes directly to the
//! Foundation REST API (`KCP_REST_URL`) to bypass community relay nodes
//! that are not peered with the TN10 mining infrastructure.
//!
//! The carrier UTXO data is read from the REST API itself; signing is done
//! locally using the private key file. Each tx is submitted, then polled
//! (up to `KCP_POLL_S` seconds) until confirmed.
//!
//! ## Usage
//!
//! ```text
//! KCP_REST_URL=https://api-tn10.kaspa.org  \
//! KCP_KEY_FILE=/path/to/wallet.key          \
//! KCP_RECIPIENT_KEY_FILE=/path/to/recipient.key  \
//! cargo run --manifest-path examples/erc20-to-ktt-wedge/Cargo.toml --bin broadcast
//! ```
//!
//! Status: v0 — pre-production — unaudited — testnet-only.

use std::env;
use std::thread::sleep;
use std::time::Duration;

use kaspa_consensus_core::{
    sign::sign,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        ScriptPublicKey, ScriptVec, SignableTransaction, Transaction, TransactionInput,
        TransactionOutpoint, TransactionOutput, UtxoEntry,
    },
};
use kaspa_hashes::Hash;
use kaspa_txscript::pay_to_address_script;
use kcp_common::{
    canonical::canonical_hash,
    tx::CARRIER_FEE_SOMPI,
    wallet::{Prefix, Wallet},
};
use kcp_ktt_token::{
    payload::{OpClass, Payload},
    state::KttState,
    tx::{DEFAULT_TOKEN_OP_VALUE_SOMPI, MIN_CHANGE_SOMPI},
};
use kii_solidity_compat::erc20::Token;
use serde_json::{json, Value};

type BoxError = Box<dyn std::error::Error>;

const POLL_INTERVAL_S: u64 = 3;

fn canonical_hash_hex(v: &Value) -> Result<[u8; 32], BoxError> {
    canonical_hash(v).map_err(|e| e.to_string().into())
}

/// Derive a state commitment by hashing the encoded KttState.
fn state_commitment(state: &KttState) -> Result<[u8; 32], BoxError> {
    let encoded = state.encode();
    let hex_enc = hex::encode(&encoded);
    canonical_hash_hex(&json!({ "state": hex_enc }))
}

fn token_id_for(owner_hex: &str, amount: u64) -> Result<[u8; 32], BoxError> {
    canonical_hash_hex(&json!({
        "pattern": "kcp-ktt-token",
        "v": "0",
        "owner": owner_hex,
        "genesis_amount": amount,
    }))
}

/// Fetch UTXOs for `address` from the REST API; return the first entry.
fn fetch_utxo(rest_url: &str, address: &str) -> Result<Value, BoxError> {
    let url = format!("{rest_url}/addresses/{address}/utxos");
    let resp: Value = ureq::get(&url).call()?.into_json()?;
    let arr = resp.as_array().ok_or("utxos not an array")?;
    if arr.is_empty() {
        return Err(format!("no UTXOs for {address}").into());
    }
    // Pick the entry with the largest amount.
    let entry = arr
        .iter()
        .max_by_key(|e| {
            e["utxoEntry"]["amount"]
                .as_str()
                .unwrap_or("0")
                .parse::<u64>()
                .unwrap_or(0)
        })
        .ok_or("empty utxo list")?;
    Ok(entry.clone())
}

/// Build, sign, and POST a carrier tx to the Foundation REST API.
/// Returns the txid hex string.
fn submit_carrier_tx(
    rest_url: &str,
    wallet: &Wallet,
    utxo_json: &Value,
    payload_bytes: Vec<u8>,
) -> Result<String, BoxError> {
    // Parse the UTXO fields from the REST API response.
    let outpoint_txid_hex = utxo_json["outpoint"]["transactionId"]
        .as_str()
        .ok_or("missing transactionId")?;
    let outpoint_index = utxo_json["outpoint"]["index"]
        .as_u64()
        .ok_or("missing index")? as u32;
    let input_amount = utxo_json["utxoEntry"]["amount"]
        .as_str()
        .ok_or("missing amount")?
        .parse::<u64>()?;
    let spk_hex = utxo_json["utxoEntry"]["scriptPublicKey"]["scriptPublicKey"]
        .as_str()
        .ok_or("missing scriptPublicKey")?;
    let block_daa_score = utxo_json["utxoEntry"]["blockDaaScore"]
        .as_str()
        .ok_or("missing blockDaaScore")?
        .parse::<u64>()?;
    let is_coinbase = utxo_json["utxoEntry"]["isCoinbase"]
        .as_bool()
        .unwrap_or(false);

    // Parse the txid into a kaspa_hashes::Hash.
    let txid_bytes = hex::decode(outpoint_txid_hex).map_err(|e| format!("txid hex: {e}"))?;
    let txid = Hash::from_slice(&txid_bytes);

    // Parse the script public key.
    // The REST API returns the raw script bytes as hex (version=0 P2PK script).
    let spk_bytes = hex::decode(spk_hex).map_err(|e| format!("spk hex: {e}"))?;
    let spk = ScriptPublicKey::new(0, ScriptVec::from_slice(&spk_bytes));

    let required = DEFAULT_TOKEN_OP_VALUE_SOMPI + CARRIER_FEE_SOMPI;
    if input_amount < required {
        return Err(format!(
            "UTXO amount {input_amount} < required {required} sompi"
        )
        .into());
    }

    let change_amount = input_amount - DEFAULT_TOKEN_OP_VALUE_SOMPI - CARRIER_FEE_SOMPI;
    let outpoint = TransactionOutpoint::new(txid, outpoint_index);
    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let token_output = TransactionOutput::new(
        DEFAULT_TOKEN_OP_VALUE_SOMPI,
        pay_to_address_script(&wallet.address),
    );
    let mut outputs = vec![token_output];
    if change_amount >= MIN_CHANGE_SOMPI {
        outputs.push(TransactionOutput::new(
            change_amount,
            pay_to_address_script(&wallet.address),
        ));
    }

    let tx = Transaction::new(
        0,
        vec![input],
        outputs,
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        payload_bytes,
    );

    let utxo_entry = UtxoEntry::new(
        input_amount,
        spk,
        block_daa_score,
        is_coinbase,
        None, // covenant_id: not present in REST API response
    );
    let signable = SignableTransaction::with_entries(tx, vec![utxo_entry]);
    let signed = sign(signable, wallet.keypair);

    // Serialize to REST JSON format.
    let signed_tx = &signed.tx;
    let inputs_json: Vec<Value> = signed_tx
        .inputs
        .iter()
        .map(|inp| {
            json!({
                "previousOutpoint": {
                    "transactionId": inp.previous_outpoint.transaction_id.to_string(),
                    "index": inp.previous_outpoint.index,
                },
                "signatureScript": hex::encode(&inp.signature_script),
                "sequence": inp.sequence,
                "sigOpCount": inp.compute_commit.sig_op_count().unwrap_or_default(),
            })
        })
        .collect();

    let outputs_json: Vec<Value> = signed_tx
        .outputs
        .iter()
        .map(|out| {
            json!({
                "amount": out.value,
                "scriptPublicKey": {
                    "version": out.script_public_key.version(),
                    "scriptPublicKey": hex::encode(out.script_public_key.script()),
                },
            })
        })
        .collect();

    let subnetwork_id_hex =
        hex::encode(AsRef::<[u8]>::as_ref(&signed_tx.subnetwork_id));

    let body = json!({
        "transaction": {
            "version": signed_tx.version,
            "inputs": inputs_json,
            "outputs": outputs_json,
            "lockTime": signed_tx.lock_time,
            "subnetworkId": subnetwork_id_hex,
            "gas": signed_tx.gas,
            "payload": hex::encode(&signed_tx.payload),
        },
        "allowOrphan": false,
    });

    let url = format!("{rest_url}/transactions");
    let result = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());

    let resp: Value = match result {
        Ok(r) => r.into_json()?,
        Err(ureq::Error::Status(code, r)) => {
            let body_text = r.into_string().unwrap_or_default();
            return Err(format!("REST API returned {code}: {body_text}").into());
        }
        Err(e) => return Err(e.into()),
    };

    if let Some(txid) = resp.as_str().or_else(|| resp["transactionId"].as_str()) {
        return Ok(txid.to_string());
    }
    if let Some(detail) = resp["detail"].as_str() {
        return Err(format!("REST API rejected tx: {detail}").into());
    }
    Ok(resp.to_string())
}

/// Poll until the tx appears in a confirmed block (max `max_polls × POLL_INTERVAL_S`).
fn wait_for_confirmation(rest_url: &str, txid: &str, max_polls: u32) -> Result<(), BoxError> {
    for i in 1..=max_polls {
        sleep(Duration::from_secs(POLL_INTERVAL_S));
        let url = format!("{rest_url}/transactions/{txid}?inputs=false&outputs=false");
        match ureq::get(&url).call() {
            Ok(resp) => {
                let data: Value = resp.into_json()?;
                if let Some(bh) = data["accepting_block_hash"].as_str() {
                    if bh != "null" && !bh.is_empty() {
                        println!("  confirmed in block {bh}");
                        return Ok(());
                    }
                }
                // also check block_hash (present even in non-accepting blocks)
                if data["block_hash"].as_str().is_some_and(|h| !h.is_empty() && h != "null") {
                    println!("  in block (poll {i}/{max_polls})");
                    return Ok(());
                }
                if i % 10 == 0 {
                    println!("  waiting for confirmation ({i}/{max_polls})…");
                }
            }
            Err(ureq::Error::Status(404, _)) => {
                if i % 10 == 0 {
                    println!("  tx not yet indexed ({i}/{max_polls})…");
                }
            }
            Err(e) => return Err(format!("poll error: {e}").into()),
        }
    }
    Err(format!("tx {txid} not confirmed after {} polls", max_polls).into())
}

fn main() -> Result<(), BoxError> {
    let rest_url = env::var("KCP_REST_URL")
        .unwrap_or_else(|_| "https://api-tn10.kaspa.org".to_string());
    let key_file = env::var("KCP_KEY_FILE")
        .map_err(|_| "KCP_KEY_FILE is required — path to a testnet wallet key file")?;
    let recipient_key_file =
        env::var("KCP_RECIPIENT_KEY_FILE").unwrap_or_else(|_| key_file.clone());

    let issuer =
        Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
            .map_err(|e| format!("failed to load wallet: {e}"))?;
    let recipient =
        Wallet::load(std::path::Path::new(&recipient_key_file), 0, Prefix::Testnet)
            .map_err(|e| format!("failed to load recipient wallet: {e}"))?;

    let issuer_xonly: [u8; 32] = issuer.keypair.x_only_public_key().0.serialize();
    let recipient_xonly: [u8; 32] = recipient.keypair.x_only_public_key().0.serialize();
    let issuer_owner_hex = hex::encode(issuer_xonly);

    println!("REST broadcast — ERC20→KTT wedge (via {})", rest_url);
    println!("issuer:    {}", issuer.address_string());
    println!("recipient: {}", recipient.address_string());

    let token = Token::new("HelloKTT", "HKT", 8, issuer_xonly);
    const GENESIS_AMOUNT: u64 = 1_000_000;
    const TRANSFER_AMOUNT: u64 = 400_000;
    const BURN_AMOUNT: u64 = 100_000;

    let (holder_state, _minter_state) = token.initial_mint(issuer_xonly, GENESIS_AMOUNT)?;
    let tok_id = token_id_for(&issuer_owner_hex, GENESIS_AMOUNT)?;

    // ── [1] issue ────────────────────────────────────────────────────────────
    println!("\n--- [1] initial_mint ({GENESIS_AMOUNT} HKT) ---");
    let issue_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Issue,
        state_commitment: state_commitment(&holder_state)?,
    }
    .encode();

    let utxo_json = fetch_utxo(&rest_url, &issuer.address_string())?;
    println!(
        "  carrier UTXO: {} idx {}  amount: {} sompi",
        utxo_json["outpoint"]["transactionId"].as_str().unwrap_or("?"),
        utxo_json["outpoint"]["index"].as_u64().unwrap_or(0),
        utxo_json["utxoEntry"]["amount"].as_str().unwrap_or("?"),
    );

    let issue_tx_id = submit_carrier_tx(&rest_url, &issuer, &utxo_json, issue_payload)?;
    println!("  issue tx_id: {issue_tx_id}");
    wait_for_confirmation(&rest_url, &issue_tx_id, 100)?;

    // ── [2] transfer ─────────────────────────────────────────────────────────
    println!("\n--- [2] transfer ({TRANSFER_AMOUNT} HKT to recipient) ---");
    let (to_state, change_state) =
        token.transfer(&holder_state, recipient_xonly, TRANSFER_AMOUNT)?;
    println!(
        "  to: {} HKT  change: {} HKT",
        to_state.amount, change_state.amount
    );
    let transfer_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Transfer,
        state_commitment: state_commitment(&change_state)?,
    }
    .encode();

    // After issue confirms, the change UTXO is the fresh issuer UTXO.
    let utxo_json2 = fetch_utxo(&rest_url, &issuer.address_string())?;
    let transfer_tx_id =
        submit_carrier_tx(&rest_url, &issuer, &utxo_json2, transfer_payload)?;
    println!("  transfer tx_id: {transfer_tx_id}");
    wait_for_confirmation(&rest_url, &transfer_tx_id, 100)?;

    // ── [3] burn ─────────────────────────────────────────────────────────────
    println!("\n--- [3] burn ({BURN_AMOUNT} HKT) ---");
    let remaining = token.burn(&change_state, BURN_AMOUNT)?;
    println!(
        "  before: {} HKT  burned: {}  remaining: {}",
        change_state.amount, BURN_AMOUNT, remaining.amount
    );
    let burn_payload = Payload {
        token_id: tok_id,
        op_class: OpClass::Burn,
        state_commitment: state_commitment(&remaining)?,
    }
    .encode();

    let utxo_json3 = fetch_utxo(&rest_url, &issuer.address_string())?;
    let burn_tx_id = submit_carrier_tx(&rest_url, &issuer, &utxo_json3, burn_payload)?;
    println!("  burn tx_id: {burn_tx_id}");
    wait_for_confirmation(&rest_url, &burn_tx_id, 100)?;

    // ── FACTS-ready summary ──────────────────────────────────────────────────
    let tok_id_hex = hex::encode(tok_id);
    println!("\n--- FACTS-ready ---");
    println!(
        "  id: KCP-ERC20-WEDGE-001\n  \
         claim: \"kii-solidity-compat ERC20→KTT wedge on testnet-10: \
         token_id={tok_id_hex}, issue_tx={issue_tx_id}, \
         transfer_tx={transfer_tx_id}, burn_tx={burn_tx_id}\"\n  \
         source: examples/erc20-to-ktt-wedge/src/bin/broadcast.rs\n  \
         verified_at: [FACT-NEEDED: fill in today's date]\n  \
         note: v0 — unaudited — ERC20-shaped kii-solidity-compat API over \
         kcp-ktt-token; carrier-anchored on testnet via Foundation REST API"
    );

    Ok(())
}
