//! Multi-step LIVE covenant lineage on testnet-10 — genesis + N appends.
//!
//! Proves a covenant-id-bound lineage advances **multiple steps** on-chain:
//! each append spends the previous head and re-locks to the next state, so the
//! lineage genuinely chains (seq 0 → 1 → 2 …), not just a single transition.
//! Same construction as `reserve_covenant_live` (embedded silverc byte-capture
//! plus a spliced oracle sig, no silverscript-lang dep), extended to a
//! multi-step capture: `KCP_MS_CAPTURE_JSON` carries the N+1 state scripts and
//! the N append sigscript templates.
//!
//! ## Usage
//! ```text
//! KCP_NODE_URL=ws://localhost:17210 KCP_NET_SUFFIX=10 KCP_KEY_FILE=/path/funded.key \
//! KCP_MS_CAPTURE_JSON=/path/multistep-capture.json \
//!   cargo run -p kcp-common --example multi_step_lineage --features wrpc
//! ```
//! Locks ≤1 KAS. Testnet only. SYNTHETIC data. Status: **v0 — unaudited.**

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
    use kaspa_bip32::secp256k1::{Keypair, SECP256K1};
    use kaspa_consensus_core::{
        constants::TX_VERSION_TOCCATA,
        hashing::covenant_id::covenant_id,
        hashing::sighash::SigHashReusedValuesUnsync,
        mass::ComputeBudget,
        sign::sign,
        subnets::SUBNETWORK_ID_NATIVE,
        tx::{
            CovenantBinding, PopulatedTransaction, ScriptPublicKey, SignableTransaction,
            Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput,
            UtxoEntry, VerifiableTransaction,
        },
        Hash,
    };
    use kaspa_rpc_core::api::rpc::RpcApi;
    use kaspa_txscript::{
        caches::Cache, covenants::CovenantsContext, extract_script_pub_key_address,
        pay_to_address_script, EngineCtx, EngineFlags, TxScriptEngine,
    };
    use kcp_common::{
        p2sh::{p2sh_input_sighash, p2sh_lock_script, schnorr_satisfier_sig},
        tx::CARRIER_FEE_SOMPI,
        wallet::{Prefix, Wallet},
        wrpc::{NodeClient, NodeConfig},
    };
    use std::env;

    type BoxError = Box<dyn std::error::Error>;
    const GENESIS_VALUE_SOMPI: u64 = 10_000_000;
    const MIN_CHANGE_FOR_MASS_SOMPI: u64 = 12_000_000;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).expect("hex")
    }

    struct Step {
        prefix: Vec<u8>,
        suffix: Vec<u8>,
    }
    struct Cap {
        oracle_sk: Vec<u8>,
        state_scripts: Vec<Vec<u8>>, // N+1 scripts
        appends: Vec<Step>,          // N
    }

    fn load() -> Result<Cap, BoxError> {
        let path = env::var("KCP_MS_CAPTURE_JSON")
            .map_err(|_| "KCP_MS_CAPTURE_JSON is required — multi-step capture json")?;
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        let states = v["states"].as_array().ok_or("no states")?;
        let appends = v["appends"].as_array().ok_or("no appends")?;
        Ok(Cap {
            oracle_sk: h(v["oracle_sk_hex"].as_str().ok_or("no oracle_sk")?),
            state_scripts: states.iter().map(|s| h(s.as_str().unwrap())).collect(),
            appends: appends
                .iter()
                .map(|a| Step {
                    prefix: h(a["prefix_hex"].as_str().unwrap()),
                    suffix: h(a["suffix_hex"].as_str().unwrap()),
                })
                .collect(),
        })
    }

    fn engine_units(tx: &Transaction, entry: &UtxoEntry) -> Result<u64, BoxError> {
        let populated = PopulatedTransaction::new(tx, vec![entry.clone()]);
        let ctx = CovenantsContext::from_tx(&populated).map_err(|e| format!("from_tx: {e:?}"))?;
        let utxo = populated.utxo(0).ok_or("no utxo")?;
        let cache = Cache::new(0);
        let reused = SigHashReusedValuesUnsync::new();
        let mut vm = TxScriptEngine::from_transaction_input(
            &populated,
            &tx.inputs[0],
            0,
            utxo,
            EngineCtx::new(&cache)
                .with_reused(&reused)
                .with_covenants_ctx(&ctx),
            EngineFlags {
                covenants_enabled: true,
                ..Default::default()
            },
        );
        vm.execute()
            .map_err(|e| format!("engine rejected: {e:?}"))?;
        Ok(vm.used_script_units().0)
    }

    fn is_transient<E: std::fmt::Display>(e: &E) -> bool {
        let s = e.to_string();
        s.contains("not found") || s.contains("already spent") || s.contains("in the mempool")
    }

    #[tokio::main]
    pub async fn run() -> Result<(), BoxError> {
        let cap = load()?;
        let oracle = Keypair::from_seckey_slice(SECP256K1, &cap.oracle_sk)?;
        let n = cap.appends.len();
        let spks: Vec<ScriptPublicKey> = cap
            .state_scripts
            .iter()
            .map(|s| p2sh_lock_script(s))
            .collect();

        let node_url =
            env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://localhost:17210".to_string());
        let key_file = env::var("KCP_KEY_FILE").map_err(|_| "KCP_KEY_FILE is required")?;
        let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let rpc = NodeClient::new(NodeConfig::testnet(&node_url, net_suffix))
            .rpc()
            .await?;
        let node = NodeClient::new(NodeConfig::testnet(&node_url, net_suffix));
        let info = node.server_info().await?;
        if !info.network_id.contains("testnet") {
            return Err(format!("REFUSED: {} not testnet", info.network_id).into());
        }
        let wallet = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
            .map_err(|e| format!("wallet: {e}"))?;
        println!(
            "network={} synced={} steps={n}",
            info.network_id, info.is_synced
        );

        // ── GENESIS → P2SH(state0) ──────────────────────────────────────────────
        let entries = rpc
            .get_utxos_by_addresses(vec![wallet.address.clone()])
            .await
            .map_err(|e| format!("utxos: {e}"))?;
        let required = GENESIS_VALUE_SOMPI + CARRIER_FEE_SOMPI;
        let mut cands: Vec<_> = entries
            .into_iter()
            .filter(|e| e.utxo_entry.amount > required)
            .collect();
        cands.sort_by_key(|e| e.utxo_entry.amount);
        let fund = cands.into_iter().next().ok_or("no UTXO covers genesis")?;
        let fund_op = TransactionOutpoint::new(fund.outpoint.transaction_id, fund.outpoint.index);
        let change0 = fund.utxo_entry.amount.saturating_sub(required);
        let (cov_value, change) = if change0 >= MIN_CHANGE_FOR_MASS_SOMPI {
            (GENESIS_VALUE_SOMPI, change0)
        } else {
            (fund.utxo_entry.amount - CARRIER_FEE_SOMPI, 0)
        };
        let mut gen_out = TransactionOutput::new(cov_value, spks[0].clone());
        let cov_id: Hash = covenant_id(fund_op, std::iter::once((0u32, &gen_out)));
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
        CovenantsContext::from_tx(&PopulatedTransaction::new(&gtx, vec![fund_entry.clone()]))
            .map_err(|e| format!("genesis preflight: {e:?}"))?;
        let signed = sign(
            SignableTransaction::with_entries(gtx, vec![fund_entry]),
            wallet.keypair,
        );
        let genesis_txid: TransactionId = rpc
            .submit_transaction((&signed.tx).into(), false)
            .await
            .map_err(|e| format!("submit genesis: {e}"))?;
        println!("covenant_id: {cov_id}\ngenesis seq0: {genesis_txid}");

        // ── WALK: append step i spends P2SH(state_i) → P2SH(state_{i+1}) ─────────
        let mut head_op = TransactionOutpoint::new(genesis_txid, 0);
        let mut tx_ids = vec![genesis_txid.to_string()];
        for i in 0..n {
            let in_spk = spks[i].clone();
            let out_spk = spks[i + 1].clone();
            let p2sh_addr = extract_script_pub_key_address(&in_spk, Prefix::Testnet)
                .map_err(|e| format!("addr: {e}"))?;
            // wait for head UTXO, then build+preflight+submit the append
            let mut submitted = String::new();
            for attempt in 1..=40u32 {
                let u = rpc
                    .get_utxos_by_addresses(vec![p2sh_addr.clone()])
                    .await
                    .map_err(|e| format!("utxos: {e}"))?;
                let Some(head) = u.into_iter().find(|e| {
                    e.outpoint.transaction_id == head_op.transaction_id
                        && e.outpoint.index == head_op.index
                }) else {
                    if attempt < 40 {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                    return Err(format!("step {i}: head UTXO not confirmed").into());
                };
                let amount = head.utxo_entry.amount;
                let daa = head.utxo_entry.block_daa_score;
                let input_entry = UtxoEntry::new(amount, in_spk.clone(), daa, false, Some(cov_id));
                let mut out = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, out_spk.clone());
                out.covenant = Some(CovenantBinding {
                    authorizing_input: 0,
                    covenant_id: cov_id,
                });
                let mut tx = Transaction::new(
                    TX_VERSION_TOCCATA,
                    vec![TransactionInput::new(head_op, vec![], 0, 0)],
                    vec![out],
                    0,
                    SUBNETWORK_ID_NATIVE,
                    0,
                    vec![],
                );
                let sigscript = |tx: &Transaction| -> Vec<u8> {
                    let sh = p2sh_input_sighash(tx, std::slice::from_ref(&input_entry), 0);
                    let sig = schnorr_satisfier_sig(&sh, &oracle);
                    let mut s = Vec::with_capacity(
                        cap.appends[i].prefix.len() + 65 + cap.appends[i].suffix.len(),
                    );
                    s.extend_from_slice(&cap.appends[i].prefix);
                    s.extend_from_slice(&sig);
                    s.extend_from_slice(&cap.appends[i].suffix);
                    s
                };
                tx.inputs[0].compute_commit = ComputeBudget(u16::MAX).into();
                tx.inputs[0].signature_script = sigscript(&tx);
                let used = engine_units(&tx, &input_entry)?;
                tx.inputs[0].compute_commit =
                    ComputeBudget((used / 10_000 + 3).min(u16::MAX as u64) as u16).into();
                tx.inputs[0].signature_script = sigscript(&tx);
                engine_units(&tx, &input_entry)?; // final preflight
                match rpc.submit_transaction((&tx).into(), false).await {
                    Ok(id) => {
                        submitted = id.to_string();
                        break;
                    }
                    Err(e) if is_transient(&e) && attempt < 40 => {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => return Err(format!("submit append {i}: {e}").into()),
                }
            }
            if submitted.is_empty() {
                return Err(format!("step {i} not submitted").into());
            }
            println!("append seq{}→{}: {submitted}", i, i + 1);
            head_op =
                TransactionOutpoint::new(submitted.parse().map_err(|e| format!("parse: {e}"))?, 0);
            tx_ids.push(submitted);
        }

        println!("\n--- FACTS-ready: multi-step lineage ---");
        println!("  network: {}", info.network_id);
        println!("  covenant_id: {cov_id}");
        println!("  chain (seq 0..{n}): {}", tx_ids.join(" -> "));
        println!(
            "  head (seq {n}) scriptPubKey: {}",
            hex::encode(spks[n].script())
        );
        println!("  note: covenant lineage advanced {n} steps live; each append spent the prior head and was engine-preflighted (covenants_enabled). v0 unaudited synthetic.");
        Ok(())
    }
}
