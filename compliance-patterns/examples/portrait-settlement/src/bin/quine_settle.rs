//! Kii Quine — self-reproducing silverscript covenant, TN10 rehearsal settlement.
//!
//! The quine covenant `reproduce(State[] prev_states) : (State)` (quine.sil,
//! a companion project) requires its successor output to carry THIS SAME covenant
//! (binding = cov, to = 1, mode = transition) with `gen = prev.gen + 1`. So the
//! covenant perpetuates itself across UTXOs — the script reproduces itself, and
//! the engine per-instance covenant_id (KIP-20) is inherited by every successor.
//!
//! `reproduce` takes NO caller args and NO signature (ABI inputs = []), so the
//! spend sigscript is fully captured per generation in `quine-capture.json`
//! (KCP_QUINE_CAPTURE) — there is nothing tx-dependent to splice. Each spend is
//! engine-preflighted against the REAL pinned v2.0.0 engine (90dbf07) with a
//! real CovenantsContext before submit (no funds risked on a malformed spend).
//!
//! Construction mirrors the proven `multi_step_lineage` example (genesis +
//! N appends, constant genesis covenant_id) minus the oracle signature.
//!
//! Modes (KCP_MODE):
//!   dryrun  (default) — offline v2.0.0-engine proof: a valid reproduce (gen
//!                       0→1) ACCEPTS; a spend whose output drops the covenant
//!                       REJECTS. No node, no funds.
//!   lock              — fund P2SH(gen0) from the wallet → generation-0 covenant
//!                       UTXO. Prints genesis_txid + per-instance covenant_id.
//!   reproduce         — chain gen N→N+1 for KCP_REPRODUCTIONS steps from
//!                       KCP_GENESIS_TXID, verifying covenant_id == genesis and
//!                       gen incrementing at each step. Writes KCP_OUT_JSON.
//!   negctl            — spend a gen-N covenant UTXO to a PLAIN output (no
//!                       covenant carried forward) → the covenant must REJECT.
//!
//! Status: v0 — pre-production — unaudited — testnet-only — MIT.

use std::env;

use kaspa_consensus_core::{
    constants::TX_VERSION_TOCCATA,
    hashing::covenant_id::covenant_id,
    hashing::sighash::SigHashReusedValuesUnsync,
    mass::ComputeBudget,
    sign::sign,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        CovenantBinding, PopulatedTransaction, ScriptPublicKey, SignableTransaction, Transaction,
        TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
        VerifiableTransaction,
    },
    Hash,
};
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_txscript::{
    caches::Cache, covenants::CovenantsContext, extract_script_pub_key_address,
    pay_to_address_script, pay_to_script_hash_script, EngineCtx, EngineFlags, TxScriptEngine,
};

use kcp_common::{
    tx::CARRIER_FEE_SOMPI,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};

use sha2::{Digest, Sha256};

type BoxError = Box<dyn std::error::Error>;

const GENESIS_VALUE_SOMPI: u64 = 100_000_000; // 1 TKAS into the covenant UTXO
const MIN_CHANGE_FOR_MASS_SOMPI: u64 = 12_000_000;

/// One generation's captured material.
struct GenCap {
    gen: u64,
    script: Vec<u8>,
    sigscript: Vec<u8>,
}

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn load_capture(path: &str) -> Result<Vec<GenCap>, BoxError> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let gens = v["generations"].as_array().ok_or("no generations[]")?;
    let mut out = Vec::with_capacity(gens.len());
    for g in gens {
        out.push(GenCap {
            gen: g["gen"].as_u64().ok_or("gen not u64")?,
            script: hex::decode(g["script_hex"].as_str().ok_or("no script_hex")?)?,
            sigscript: hex::decode(
                g["reproduce_sigscript_hex"]
                    .as_str()
                    .ok_or("no reproduce_sigscript_hex")?,
            )?,
        });
    }
    // Sanity: gen index == array position, gen embedded at script byte[2].
    for (i, g) in out.iter().enumerate() {
        if g.gen != i as u64 {
            return Err(format!("capture out of order at index {i} (gen={})", g.gen).into());
        }
        if g.script.get(2).copied() != Some(i as u8) {
            return Err(format!("gen {i}: script byte[2] != {i} (state not embedded?)").into());
        }
    }
    Ok(out)
}

/// Load the funded TN10 wallet from a JSON file ({mnemonic, address, ...}),
/// keeping the key only in that file + memory. Asserts the derived address
/// matches the file's `address`.
fn load_wallet() -> Result<Wallet, BoxError> {
    let path = env::var("KCP_WALLET_JSON").map_err(|_| "KCP_WALLET_JSON is required")?;
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    let expect = v["address"].as_str();

    // Preferred: a 32-byte private key passed in-memory via KCP_WALLET_KEY (the
    // kaspa-wasm receiveKey(0) the JS broadcaster signs with). The JSON wallet's
    // mnemonic uses kaspa's HD derivation, which differs from kcp-common's BIP44
    // path, so deriving from the mnemonic here would yield the wrong address.
    let wallet = if let Ok(key_hex) = env::var("KCP_WALLET_KEY") {
        Wallet::from_private_key_hex(key_hex.trim(), 0, Prefix::Testnet)
            .map_err(|e| format!("load KCP_WALLET_KEY: {e}"))?
    } else {
        let mnemonic = v["mnemonic"]
            .as_str()
            .ok_or("wallet json has no mnemonic")?;
        Wallet::from_phrase(mnemonic, "", 0, Prefix::Testnet)
            .map_err(|e| format!("derive wallet: {e}"))?
    };

    if let Some(expect) = expect {
        if wallet.address.to_string() != expect {
            return Err(format!(
                "wallet address mismatch: derived {} != file {} (set KCP_WALLET_KEY to the \
                 kaspa-wasm receiveKey(0) for this wallet)",
                wallet.address, expect
            )
            .into());
        }
    }
    Ok(wallet)
}

/// Run a covenant spend through the real v2.0.0 engine with a real
/// CovenantsContext. Returns consumed script units on accept.
fn covenant_engine_run(
    tx: &Transaction,
    idx: usize,
    entries: &[UtxoEntry],
) -> Result<u64, BoxError> {
    let populated = PopulatedTransaction::new(tx, entries.to_vec());
    let cov_ctx = CovenantsContext::from_tx(&populated)
        .map_err(|e| format!("CovenantsContext::from_tx: {e:?}"))?;
    let utxo = populated.utxo(idx).ok_or("no utxo")?;
    let sig_cache = Cache::new(0);
    let reused = SigHashReusedValuesUnsync::new();
    let ctx = EngineCtx::new(&sig_cache)
        .with_reused(&reused)
        .with_covenants_ctx(&cov_ctx);
    let flags = EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    };
    let mut vm =
        TxScriptEngine::from_transaction_input(&populated, &tx.inputs[idx], idx, utxo, ctx, flags);
    vm.execute()
        .map_err(|e| format!("covenant engine rejected: {e:?}"))?;
    Ok(vm.used_script_units().0)
}

/// Build a reproduce spend: spend the gen-N covenant UTXO (`in_spk`, value
/// `amount`, daa, covenant_id `cov_id`) using the captured gen-N sigscript,
/// continuing to `out` (P2SH(gen N+1) for a valid spend). The output binding
/// propagates the SAME genesis covenant_id (KIP-20 lineage). Engine-preflighted.
fn build_reproduce(
    in_spk: &ScriptPublicKey,
    sigscript: &[u8],
    out: TransactionOutput,
    outpoint: TransactionOutpoint,
    amount: u64,
    daa: u64,
    cov_id: Hash,
) -> Result<(Transaction, u64), BoxError> {
    let input = TransactionInput::new(outpoint, vec![], 0, 0);
    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![input],
        vec![out],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let input_entry = UtxoEntry::new(amount, in_spk.clone(), daa, false, Some(cov_id));
    // The sigscript is static (no sig depends on the tx), so a single measure
    // pass suffices: budget does not change the sigscript.
    tx.inputs[0].compute_commit = ComputeBudget(u16::MAX).into();
    tx.inputs[0].signature_script = sigscript.to_vec();
    let used = covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry))?;
    let budget = (used / 10_000 + 3).min(u16::MAX as u64) as u16;
    tx.inputs[0].compute_commit = ComputeBudget(budget).into();
    let used_final = covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry))?;
    Ok((tx, used_final))
}

/// A covenant continuation output: P2SH(state) carrying the lineage binding.
fn covenant_output(value: u64, script: &[u8], cov_id: Hash) -> TransactionOutput {
    let mut out = TransactionOutput::new(value, pay_to_script_hash_script(script));
    out.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id,
    });
    out
}

fn is_transient<E: std::fmt::Display>(e: &E) -> bool {
    let s = e.to_string();
    s.contains("not found") || s.contains("already spent") || s.contains("in the mempool")
}

// ── dryrun: offline v2.0.0-engine proof ───────────────────────────────────────
fn dry_run(caps: &[GenCap], wallet: &Wallet) -> Result<(), BoxError> {
    println!("=== QUINE DRY RUN — offline v2.0.0-engine proof (no node) ===");
    let spk0 = pay_to_script_hash_script(&caps[0].script);

    // Synthetic genesis: covenant_id = covenant_id(funding, [P2SH(state0)]).
    let funding = TransactionOutpoint::new(TransactionId::from_bytes([0xab; 32]), 0);
    let gen_out = covenant_output(
        GENESIS_VALUE_SOMPI,
        &caps[0].script,
        Hash::from_bytes([0; 32]),
    );
    let cov_id = covenant_id(funding, std::iter::once((0u32, &gen_out)));
    println!("derived covenant_id (engine, per-instance): {cov_id}");

    let synth = TransactionOutpoint::new(TransactionId::from_bytes([0xcd; 32]), 0);

    // POSITIVE: valid reproduce gen 0→1, output P2SH(state1) carrying the covenant.
    let out_ok = covenant_output(
        GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI,
        &caps[1].script,
        cov_id,
    );
    let (_tx, used) = build_reproduce(
        &spk0,
        &caps[0].sigscript,
        out_ok,
        synth,
        GENESIS_VALUE_SOMPI,
        0,
        cov_id,
    )?;
    println!("[1] valid reproduce (gen 0→1, carries covenant): ACCEPT, used_script_units={used}");

    // NEGATIVE: spend gen0 to a PLAIN output (no covenant binding) → must REJECT.
    let plain = TransactionOutput::new(
        GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI,
        pay_to_address_script(&wallet.address),
    );
    match build_reproduce(
        &spk0,
        &caps[0].sigscript,
        plain,
        synth,
        GENESIS_VALUE_SOMPI,
        0,
        cov_id,
    ) {
        Err(e) => println!("[2] spend that drops the covenant (plain output): REJECT ✓\n      {e}"),
        Ok(_) => return Err("NEG wrongly ACCEPTED (covenant not carried forward)".into()),
    }

    // NEGATIVE: carry a covenant but with gen NOT incremented (output P2SH(state0)).
    let stale = covenant_output(
        GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI,
        &caps[0].script,
        cov_id,
    );
    match build_reproduce(
        &spk0,
        &caps[0].sigscript,
        stale,
        synth,
        GENESIS_VALUE_SOMPI,
        0,
        cov_id,
    ) {
        Err(e) => {
            println!("[3] reproduce with gen NOT incremented (output=state0): REJECT ✓\n      {e}")
        }
        Ok(_) => return Err("NEG wrongly ACCEPTED (gen not incremented)".into()),
    }

    println!(
        "DRY RUN PASSED — reproduce is engine-valid on v2.0.0; non-reproducing spends rejected."
    );
    Ok(())
}

async fn connect() -> Result<(std::sync::Arc<dyn RpcApi>, String), BoxError> {
    let node_url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://127.0.0.1:17210".to_string());
    let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let node = NodeClient::new(NodeConfig::testnet(&node_url, net_suffix));
    let info = node.server_info().await?;
    if !info.network_id.contains("testnet") {
        return Err(format!("REFUSED: '{}' is not testnet", info.network_id).into());
    }
    println!(
        "connected: server={} network={} synced={}",
        info.server_version, info.network_id, info.is_synced
    );
    if !info.is_synced {
        return Err("REFUSED: node is not synced".into());
    }
    let rpc = node.rpc().await?;
    Ok((rpc, info.network_id))
}

// ── lock: fund the gen-0 covenant UTXO ────────────────────────────────────────
async fn lock(caps: &[GenCap], wallet: &Wallet) -> Result<(), BoxError> {
    let (rpc, _net) = connect().await?;
    let spk0 = pay_to_script_hash_script(&caps[0].script);

    let entries = rpc
        .get_utxos_by_addresses(vec![wallet.address.clone()])
        .await
        .map_err(|e| format!("get_utxos: {e}"))?;
    let required = GENESIS_VALUE_SOMPI + CARRIER_FEE_SOMPI;
    let mut cands: Vec<_> = entries
        .into_iter()
        .filter(|e| e.utxo_entry.amount > required)
        .collect();
    cands.sort_by_key(|e| e.utxo_entry.amount);
    let fund = cands
        .into_iter()
        .next()
        .ok_or("no UTXO covers genesis value+fee")?;
    let fund_op = TransactionOutpoint::new(fund.outpoint.transaction_id, fund.outpoint.index);
    let change0 = fund.utxo_entry.amount.saturating_sub(required);
    let (cov_value, change) = if change0 >= MIN_CHANGE_FOR_MASS_SOMPI {
        (GENESIS_VALUE_SOMPI, change0)
    } else {
        (fund.utxo_entry.amount - CARRIER_FEE_SOMPI, 0)
    };

    let mut gen_out = TransactionOutput::new(cov_value, spk0.clone());
    let cov_id = covenant_id(fund_op, std::iter::once((0u32, &gen_out)));
    gen_out.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id,
    });
    let mut gouts = vec![gen_out];
    if change > 0 {
        gouts.push(TransactionOutput::new(
            change,
            pay_to_address_script(&wallet.address),
        ));
    }
    let gtx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(fund_op, vec![], 0, 0)],
        gouts,
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let fund_entry = UtxoEntry::new(
        fund.utxo_entry.amount,
        fund.utxo_entry.script_public_key.clone(),
        fund.utxo_entry.block_daa_score,
        fund.utxo_entry.is_coinbase,
        fund.utxo_entry.covenant_id,
    );
    // Preflight the genesis funding tx (build the CovenantsContext).
    CovenantsContext::from_tx(&PopulatedTransaction::new(&gtx, vec![fund_entry.clone()]))
        .map_err(|e| format!("genesis preflight: {e:?}"))?;
    let signed = sign(
        SignableTransaction::with_entries(gtx, vec![fund_entry]),
        wallet.keypair,
    );
    let genesis_txid = rpc
        .submit_transaction((&signed.tx).into(), false)
        .await
        .map_err(|e| format!("submit genesis: {e}"))?;
    println!("\n══ GENESIS LOCKED (generation 0) ══════════════════════════════════");
    println!("  genesis_txid:              {genesis_txid}");
    println!("  per_instance_covenant_id:  {cov_id}");
    println!("  covenant_value_sompi:      {cov_value}");
    println!(
        "  gen0_script_sha256:        {}",
        sha256_hex(&caps[0].script)
    );
    println!("  (next: KCP_MODE=reproduce KCP_GENESIS_TXID={genesis_txid})");
    Ok(())
}

/// Locate the covenant UTXO at P2SH(script) matching `op`, polling for confirmation.
async fn wait_utxo(
    rpc: &dyn RpcApi,
    script: &[u8],
    op: TransactionOutpoint,
) -> Result<(u64, u64, Option<Hash>), BoxError> {
    let spk = pay_to_script_hash_script(script);
    let addr =
        extract_script_pub_key_address(&spk, Prefix::Testnet).map_err(|e| format!("addr: {e}"))?;
    for attempt in 1..=90u32 {
        let utxos = rpc
            .get_utxos_by_addresses(vec![addr.clone()])
            .await
            .map_err(|e| format!("get_utxos: {e}"))?;
        if let Some(u) = utxos.into_iter().find(|e| {
            e.outpoint.transaction_id == op.transaction_id && e.outpoint.index == op.index
        }) {
            return Ok((
                u.utxo_entry.amount,
                u.utxo_entry.block_daa_score,
                u.utxo_entry.covenant_id,
            ));
        }
        if attempt < 90 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    Err("covenant UTXO not confirmed in time".into())
}

// ── reproduce: chain gen N→N+1 for KCP_REPRODUCTIONS steps, verifying each ─────
async fn reproduce(caps: &[GenCap], _wallet: &Wallet) -> Result<(), BoxError> {
    let genesis_hex = env::var("KCP_GENESIS_TXID").map_err(|_| "KCP_GENESIS_TXID is required")?;
    let genesis_tid: TransactionId = genesis_hex
        .parse()
        .map_err(|e| format!("parse txid: {e}"))?;
    let reps: u64 = env::var("KCP_REPRODUCTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let start: u64 = env::var("KCP_START_GEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let out_json = env::var("KCP_OUT_JSON")
        .unwrap_or_else(|_| "rehearsal-records.json".to_string());
    if (start + reps + 1) as usize > caps.len() {
        return Err(format!(
            "need {} generations in the capture, have {}",
            start + reps + 1,
            caps.len()
        )
        .into());
    }

    let (rpc, network) = connect().await?;
    println!("reproduce: genesis={genesis_hex} start_gen={start} reproductions={reps}");

    let mut head_op = TransactionOutpoint::new(genesis_tid, 0);
    let mut ref_cov_id: Option<Hash> = None;
    let mut records: Vec<String> = Vec::new();

    for step in 0..reps {
        let cur = (start + step) as usize;
        let nxt = cur + 1;

        // Locate + read the head (gen-cur) UTXO; its covenant_id is ground truth.
        let (amount, daa, cov_opt) = wait_utxo(rpc.as_ref(), &caps[cur].script, head_op).await?;
        let cov_id = cov_opt.ok_or("head UTXO has no covenant_id binding")?;
        match ref_cov_id {
            None => {
                ref_cov_id = Some(cov_id);
                println!("genesis per-instance covenant_id: {cov_id}");
            }
            Some(r) if r != cov_id => {
                return Err(format!(
                    "QUINE BROKEN at gen {cur}: covenant_id {cov_id} != genesis {r}"
                )
                .into());
            }
            _ => {}
        }
        println!(
            "[gen {cur}] head UTXO confirmed: amount={amount} covenant_id={cov_id} \
             (verified == genesis) script_sha256={}",
            sha256_hex(&caps[cur].script)
        );

        // Build reproduce gen cur→nxt: output P2SH(gen nxt) carrying the same covid.
        let out = covenant_output(amount - CARRIER_FEE_SOMPI, &caps[nxt].script, cov_id);
        let in_spk = pay_to_script_hash_script(&caps[cur].script);
        let (tx, used) = build_reproduce(
            &in_spk,
            &caps[cur].sigscript,
            out,
            head_op,
            amount,
            daa,
            cov_id,
        )?;
        println!("[gen {cur}→{nxt}] reproduce preflight: ACCEPT, used_script_units={used}");

        // Submit (retry transient mempool races).
        let mut txid = String::new();
        for attempt in 1..=40u32 {
            match rpc.submit_transaction((&tx).into(), false).await {
                Ok(id) => {
                    txid = id.to_string();
                    break;
                }
                Err(e) if is_transient(&e) && attempt < 40 => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => return Err(format!("submit reproduce gen {cur}→{nxt}: {e}").into()),
            }
        }
        if txid.is_empty() {
            return Err(format!("reproduce gen {cur}→{nxt} not submitted").into());
        }
        println!("[gen {cur}→{nxt}] reproduce_txid: {txid}");
        records.push(format!(
            "    {{ \"step\": {step}, \"from_gen\": {cur}, \"to_gen\": {nxt}, \
             \"reproduce_txid\": \"{txid}\", \"covenant_id\": \"{cov_id}\", \
             \"successor_script_sha256\": \"{}\", \"successor_spk_hex\": \"{}\" }}",
            sha256_hex(&caps[nxt].script),
            hex::encode(pay_to_script_hash_script(&caps[nxt].script).script()),
        ));
        head_op = TransactionOutpoint::new(txid.parse().map_err(|e| format!("parse: {e}"))?, 0);
    }

    // Final: confirm the last successor carries the SAME covenant_id and the
    // expected gen — that is the quine reproduced and verified end-state.
    let final_gen = (start + reps) as usize;
    let (famount, _fdaa, fcov) = wait_utxo(rpc.as_ref(), &caps[final_gen].script, head_op).await?;
    let fcov = fcov.ok_or("final successor UTXO has no covenant_id")?;
    let r = ref_cov_id.ok_or("no reference covenant_id")?;
    if fcov != r {
        return Err(format!(
            "QUINE BROKEN: final gen {final_gen} covenant_id {fcov} != genesis {r}"
        )
        .into());
    }
    println!(
        "\n══ VERIFIED: final gen {final_gen} UTXO covenant_id == genesis ✓ ({fcov}), amount={famount}"
    );

    let json = format!(
        "{{\n  \"_description\": \"Kii Quine TN10 reproduction chain — each step verified covenant_id == genesis and gen incremented.\",\n  \"network\": \"{network}\",\n  \"genesis_txid\": \"{genesis_hex}\",\n  \"genesis_covenant_id\": \"{r}\",\n  \"start_gen\": {start},\n  \"reproductions\": {reps},\n  \"final_gen\": {final_gen},\n  \"final_gen_covenant_id\": \"{fcov}\",\n  \"steps\": [\n{}\n  ]\n}}\n",
        records.join(",\n")
    );
    std::fs::write(&out_json, &json)?;
    println!("WROTE {out_json}");
    Ok(())
}

// ── negctl: spend a gen-N UTXO to a plain output; the covenant must reject ─────
async fn negctl(caps: &[GenCap], wallet: &Wallet) -> Result<(), BoxError> {
    let prev_hex = env::var("KCP_PREV_TXID").map_err(|_| "KCP_PREV_TXID is required")?;
    let prev_tid: TransactionId = prev_hex.parse().map_err(|e| format!("parse txid: {e}"))?;
    let from_gen: usize = env::var("KCP_FROM_GEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let (rpc, _net) = connect().await?;

    let head_op = TransactionOutpoint::new(prev_tid, 0);
    let (amount, daa, cov_opt) = wait_utxo(rpc.as_ref(), &caps[from_gen].script, head_op).await?;
    let cov_id = cov_opt.ok_or("gen UTXO has no covenant_id binding")?;
    let in_spk = pay_to_script_hash_script(&caps[from_gen].script);
    println!("negctl: spending gen {from_gen} UTXO {prev_hex}:0 (covenant_id {cov_id}) to a PLAIN output");

    // PLAIN output: no covenant binding. The reproduce covenant requires the
    // successor to carry the covenant → this must be rejected.
    let plain = TransactionOutput::new(
        amount - CARRIER_FEE_SOMPI,
        pay_to_address_script(&wallet.address),
    );
    let input = TransactionInput::new(head_op, vec![], 0, 0);
    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![input],
        vec![plain],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let input_entry = UtxoEntry::new(amount, in_spk.clone(), daa, false, Some(cov_id));
    // Realistic budget so the rejection is the covenant require, not a mass error.
    tx.inputs[0].compute_commit = ComputeBudget(20).into();
    tx.inputs[0].signature_script = caps[from_gen].sigscript.clone();

    // The local engine should already reject (the covenant is not carried forward).
    match covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry)) {
        Err(e) => println!("local v2.0.0 engine REJECTED (expected): {e}"),
        Ok(_) => {
            return Err("SECURITY FAILURE: local engine ACCEPTED a non-reproducing spend".into())
        }
    }

    // Submit so the LIVE NODE does the rejecting on-chain (records the node error).
    println!("\n══ NEGATIVE CONTROL ═══════════════════════════════════════════════");
    println!("  spent_gen:        {from_gen}");
    println!("  prev_txid:        {prev_hex}");
    println!("  covenant_id:      {cov_id}");
    match rpc.submit_transaction((&tx).into(), false).await {
        Ok(id) => {
            Err(format!("SECURITY FAILURE: node ACCEPTED a non-reproducing spend as {id}").into())
        }
        Err(e) => {
            println!("  rejected_txid:    null (NODE REJECTED ✓)");
            println!("  node_error:       {e}");
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let cap_path = env::var("KCP_QUINE_CAPTURE")
        .map_err(|_| "KCP_QUINE_CAPTURE is required — path to quine-capture.json")?;
    let caps = load_capture(&cap_path)?;
    let wallet = load_wallet()?;
    let mode = env::var("KCP_MODE").unwrap_or_else(|_| "dryrun".to_string());
    println!(
        "quine_settle mode={mode} wallet={} generations={}",
        wallet.address,
        caps.len()
    );

    match mode.as_str() {
        "dryrun" => dry_run(&caps, &wallet),
        "lock" => lock(&caps, &wallet).await,
        "reproduce" => reproduce(&caps, &wallet).await,
        "negctl" => negctl(&caps, &wallet).await,
        other => Err(format!("unknown KCP_MODE '{other}'").into()),
    }
}
