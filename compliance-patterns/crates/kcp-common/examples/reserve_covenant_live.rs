//! LIVE covenant-id-bound deployment of the anchor-only reserve covenant on
//! testnet-10 — the engine-proven → on-chain-live jump for the state-continuity
//! tier (see repo `docs/NEXT-STEPS-covenant-live-deploy.md`).
//!
//! Builds and submits TWO real transactions:
//!   1. **genesis** — an ordinary wallet UTXO funds a covenant output
//!      `P2SH(state0)` whose `CovenantBinding.covenant_id` is the consensus
//!      derivation `covenant_id(funding_outpoint, [output0])`.
//!   2. **append** — spends the genesis covenant UTXO and continues the covenant
//!      to `P2SH(state1)` (seq 0→1), authorised by the oracle's signature over
//!      the spend. The covenant script (`validateOutputState` + `checkSig`)
//!      enforces RA-1..RA-4 at consensus.
//!
//! No silverscript-lang dependency: the covenant scripts and the append
//! covenant-decl sigscript are EMBEDDED bytes captured once from silverc
//! (`CAVEATS/08-reserve-covenant/live-capture.json`). The only tx-dependent
//! piece — the oracle signature — is spliced into the captured sigscript at the
//! 65-byte region proven to be the sole variable part by diffing two dummy-sig
//! captures. The append is engine-preflighted with a real
//! `CovenantsContext::from_tx` (covenants_enabled=true) before submit.
//!
//! ## Usage
//! `KCP_CAPTURE_JSON` (required) is the silverc byte-capture for the covenant to
//! deploy; this runner is covenant-agnostic, so the same binary drives any of
//! the pattern covenants by pointing it at a different capture.
//! ```text
//! # OFFLINE proof (no node, no funds) — the hard gate before any live submit:
//! KCP_DRY_RUN=1 KCP_CAPTURE_JSON=/path/live-capture.json \
//!   cargo run -p kcp-common --example reserve_covenant_live --features wrpc
//!
//! # LIVE (locks ≤1 KAS):
//! KCP_NODE_URL=ws://localhost:17210 KCP_NET_SUFFIX=10 KCP_KEY_FILE=/path/funded.key \
//! KCP_CAPTURE_JSON=/path/live-capture.json \
//!   cargo run -p kcp-common --example reserve_covenant_live --features wrpc
//! ```
//! Refuses non-testnet networks. SYNTHETIC reserve data only.
//!
//! Status: **v0 — unaudited — testnet first.**

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
    use std::env;

    use kcp_common::{
        p2sh::{p2sh_input_sighash, p2sh_lock_script, schnorr_satisfier_sig},
        tx::CARRIER_FEE_SOMPI,
        wallet::{Prefix, Wallet},
        wrpc::{NodeClient, NodeConfig},
    };

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

    type BoxError = Box<dyn std::error::Error>;

    const GENESIS_VALUE_SOMPI: u64 = 10_000_000; // 0.1 KAS locked into the covenant UTXO (anchor-only)

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).expect("hex")
    }

    struct Capture {
        oracle_sk: Vec<u8>,
        state0_script: Vec<u8>,
        state1_script: Vec<u8>,
        append_prefix: Vec<u8>, // next-state pushes + the push-65 opcode (109 B)
        append_suffix: Vec<u8>, // selector + redeem push of state0 (696 B)
    }

    fn load_capture() -> Result<Capture, BoxError> {
        // Required: the silverc byte-capture for the covenant to deploy. The
        // captures live outside this repo (in the Kii data-room CAVEATS archive)
        // because they hold the publisher/oracle TESTNET secret key; never ship
        // one in a public tree.
        let path = env::var("KCP_CAPTURE_JSON")
            .map_err(|_| "KCP_CAPTURE_JSON is required — path to a covenant live-capture.json")?;
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        let get = |k: &str| -> Result<Vec<u8>, BoxError> {
            Ok(h(v
                .get(k)
                .and_then(|x| x.as_str())
                .ok_or_else(|| format!("capture missing {k}"))?))
        };
        Ok(Capture {
            oracle_sk: get("oracle_sk_hex")?,
            state0_script: get("state0_script_hex")?,
            state1_script: get("state1_script_hex")?,
            append_prefix: get("append_prefix_hex")?,
            append_suffix: get("append_suffix_hex")?,
        })
    }

    /// Run the reserve covenant spend through the real v2.0.0 engine with a
    /// `CovenantsContext::from_tx` (required — the covenant uses introspection,
    /// unlike CSFS). EngineFlags default `sigop_script_units = Gram(1000)` equals
    /// testnet-10's `mass_per_sig_op`, so the budget gate is faithful. Returns
    /// consumed script units on accept.
    fn covenant_engine_run(
        tx: &Transaction,
        idx: usize,
        entries: &[UtxoEntry],
    ) -> Result<u64, BoxError> {
        let populated = PopulatedTransaction::new(tx, entries.to_vec());
        let cov_ctx = CovenantsContext::from_tx(&populated)
            .map_err(|e| format!("CovenantsContext::from_tx: {e:?}"))?;
        let utxo = populated.utxo(idx).ok_or("no utxo for input")?;
        let sig_cache = Cache::new(0);
        let reused = SigHashReusedValuesUnsync::new();
        let ctx = EngineCtx::new(&sig_cache)
            .with_reused(&reused)
            .with_covenants_ctx(&cov_ctx);
        let flags = EngineFlags {
            covenants_enabled: true,
            ..Default::default()
        };
        let mut vm = TxScriptEngine::from_transaction_input(
            &populated,
            &tx.inputs[idx],
            idx,
            utxo,
            ctx,
            flags,
        );
        vm.execute()
            .map_err(|e| format!("covenant engine rejected: {e:?}"))?;
        Ok(vm.used_script_units().0)
    }

    /// Build the genesis covenant output (of `value`) + its derived covenant_id
    /// (the binding is excluded from the id, per `covenant_id`).
    fn build_genesis_output(
        value: u64,
        funding_outpoint: TransactionOutpoint,
        spk0: &ScriptPublicKey,
    ) -> (TransactionOutput, Hash) {
        let mut out = TransactionOutput::new(value, spk0.clone());
        let cov_id = covenant_id(funding_outpoint, std::iter::once((0u32, &out)));
        out.covenant = Some(CovenantBinding {
            authorizing_input: 0,
            covenant_id: cov_id,
        });
        (out, cov_id)
    }

    // KIP-9 storage mass: a covenant P2SH output has plurality 2 (130 storage
    // bytes), costing ~`C·4/value` mass; a small change output of value `v` costs
    // ~`C/v`. To stay under the 500_000 storage-mass standardness cap, if the
    // change would be small we absorb it into the (single) covenant output rather
    // than emit a high-mass tiny change. Threshold chosen so an emitted change
    // output contributes < ~100k mass.
    const MIN_CHANGE_FOR_MASS_SOMPI: u64 = 12_000_000;

    /// Build + engine-preflight the append spending the covenant UTXO at
    /// `genesis_outpoint`. Two-round budget: the oracle sig commits the input's
    /// compute budget, so measure under a generous budget, then re-sign at the
    /// covering budget. `signer`/`out_spk` are parameterised for negative tests.
    /// Returns the finalised, preflighted transaction + its input entry.
    #[allow(clippy::too_many_arguments)]
    fn build_and_preflight_append(
        cap: &Capture,
        spk0: &ScriptPublicKey,
        out_spk: &ScriptPublicKey,
        genesis_outpoint: TransactionOutpoint,
        amount: u64,
        daa: u64,
        cov_id: Hash,
        signer: &Keypair,
    ) -> Result<(Transaction, u64), BoxError> {
        let mut app_out = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, out_spk.clone());
        app_out.covenant = Some(CovenantBinding {
            authorizing_input: 0,
            covenant_id: cov_id,
        });
        let app_in = TransactionInput::new(genesis_outpoint, vec![], 0, 0);
        let mut tx = Transaction::new(
            TX_VERSION_TOCCATA,
            vec![app_in],
            vec![app_out],
            0,
            SUBNETWORK_ID_NATIVE,
            0,
            vec![],
        );
        let input_entry = UtxoEntry::new(amount, spk0.clone(), daa, false, Some(cov_id));

        let sigscript_for = |tx: &Transaction| -> Vec<u8> {
            let sighash = p2sh_input_sighash(tx, std::slice::from_ref(&input_entry), 0);
            let sig = schnorr_satisfier_sig(&sighash, signer); // 65 B
            let mut s = Vec::with_capacity(cap.append_prefix.len() + 65 + cap.append_suffix.len());
            s.extend_from_slice(&cap.append_prefix);
            s.extend_from_slice(&sig);
            s.extend_from_slice(&cap.append_suffix);
            s
        };

        // Version-1 (Toccata) inputs commit a COMPUTE BUDGET (u16; 1 unit =
        // 10_000 script units) and must keep sig_op_count = 0. The oracle sig
        // commits the budget via the sighash, so measure under the max budget,
        // then re-sign at the covering budget.
        tx.inputs[0].compute_commit = ComputeBudget(u16::MAX).into();
        tx.inputs[0].signature_script = sigscript_for(&tx);
        let used = covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry))?;

        // 1 budget unit = 10_000 script units; +3 units of margin above the free
        // per-input allowance.
        let budget_units = (used / 10_000 + 3).min(u16::MAX as u64) as u16;
        tx.inputs[0].compute_commit = ComputeBudget(budget_units).into();
        tx.inputs[0].signature_script = sigscript_for(&tx);
        let used_final = covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry))?;
        Ok((tx, used_final))
    }

    fn is_transient<E: std::fmt::Display>(e: &E) -> bool {
        let s = e.to_string();
        s.contains("not found") || s.contains("already spent") || s.contains("in the mempool")
    }

    /// OFFLINE proof against the v2.0.0 engine — the hard gate. Synthetic
    /// outpoints, no node, no funds. Proves the embedded-bytes + spliced-sig
    /// construction ACCEPTS, and that bad transitions REJECT.
    fn dry_run(cap: &Capture, oracle: &Keypair) -> Result<(), BoxError> {
        println!("=== DRY RUN — offline v2.0.0-engine proof (no node, no funds) ===");
        let spk0 = p2sh_lock_script(&cap.state0_script);
        let spk1 = p2sh_lock_script(&cap.state1_script);

        let funding = TransactionOutpoint::new(TransactionId::from_bytes([0xab; 32]), 0);
        let (gen_out, cov_id) = build_genesis_output(GENESIS_VALUE_SOMPI, funding, &spk0);
        println!("derived covenant_id: {cov_id}");

        // Genesis covenant-id reconstruction must validate.
        let gen_tx = Transaction::new(
            TX_VERSION_TOCCATA,
            vec![TransactionInput::new(funding, vec![], 0, 0)],
            vec![gen_out],
            0,
            SUBNETWORK_ID_NATIVE,
            0,
            vec![],
        );
        let fund_entry = UtxoEntry::new(
            GENESIS_VALUE_SOMPI + CARRIER_FEE_SOMPI,
            pay_to_address_script_dummy(),
            0,
            false,
            None,
        );
        CovenantsContext::from_tx(&PopulatedTransaction::new(&gen_tx, vec![fund_entry]))
            .map_err(|e| format!("genesis id reconstruction FAILED: {e:?}"))?;
        println!("[1] genesis covenant_id reconstruction: OK");

        let synth_genesis = TransactionOutpoint::new(TransactionId::from_bytes([0xcd; 32]), 0);

        // POSITIVE: valid append seq 0→1, correct oracle, output P2SH(state1).
        let (_tx, used) = build_and_preflight_append(
            cap,
            &spk0,
            &spk1,
            synth_genesis,
            GENESIS_VALUE_SOMPI,
            0,
            cov_id,
            oracle,
        )?;
        println!("[2] valid append (seq 0→1, oracle-signed): ACCEPT, used_script_units={used}");

        // NEGATIVE 1: output recreates seq0 (P2SH(state0)) — seq not incremented.
        match build_and_preflight_append(
            cap,
            &spk0,
            &spk0,
            synth_genesis,
            GENESIS_VALUE_SOMPI,
            0,
            cov_id,
            oracle,
        ) {
            Err(_) => println!("[3] append with non-incremented output state: REJECT (correct)"),
            Ok(_) => return Err("NEG1 wrongly ACCEPTED (seq not incremented)".into()),
        }

        // NEGATIVE 2: wrong oracle signature.
        let impostor = Keypair::from_seckey_slice(SECP256K1, &[0x9u8; 32])?;
        match build_and_preflight_append(
            cap,
            &spk0,
            &spk1,
            synth_genesis,
            GENESIS_VALUE_SOMPI,
            0,
            cov_id,
            &impostor,
        ) {
            Err(_) => println!("[4] append with wrong oracle signature: REJECT (correct)"),
            Ok(_) => return Err("NEG2 wrongly ACCEPTED (wrong oracle sig)".into()),
        }

        println!("DRY RUN PASSED — construction is engine-valid on v2.0.0. Safe to submit live.");
        Ok(())
    }

    // A throwaway non-covenant spk for the genesis funding entry in the dry run
    // (its script is never executed by from_tx; only the covenant outputs are).
    fn pay_to_address_script_dummy() -> ScriptPublicKey {
        ScriptPublicKey::new(0, kaspa_consensus_core::tx::ScriptVec::from_slice(&[0x51]))
        // OP_TRUE
    }

    #[tokio::main]
    pub async fn run() -> Result<(), BoxError> {
        let cap = load_capture()?;
        let oracle = Keypair::from_seckey_slice(SECP256K1, &cap.oracle_sk)?;

        if env::var("KCP_DRY_RUN").is_ok() {
            return dry_run(&cap, &oracle);
        }

        let node_url =
            env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://localhost:17210".to_string());
        let key_file = env::var("KCP_KEY_FILE")
            .map_err(|_| "KCP_KEY_FILE is required (or set KCP_DRY_RUN=1)")?;
        let net_suffix: u32 = env::var("KCP_NET_SUFFIX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        let node = NodeClient::new(NodeConfig::testnet(&node_url, net_suffix));
        let rpc = node.rpc().await?;
        let info = node.server_info().await?;
        println!(
            "connected: server={} network={} synced={} daa={}",
            info.server_version, info.network_id, info.is_synced, info.virtual_daa_score
        );
        if !info.network_id.contains("testnet") {
            return Err(format!("REFUSED: network_id '{}' is not testnet", info.network_id).into());
        }
        let wallet = Wallet::load(std::path::Path::new(&key_file), 0, Prefix::Testnet)
            .map_err(|e| format!("load wallet: {e}"))?;
        println!("wallet: {}", wallet.address_string());

        let spk0 = p2sh_lock_script(&cap.state0_script);
        let spk1 = p2sh_lock_script(&cap.state1_script);

        // ── GENESIS ───────────────────────────────────────────────────────────
        println!("\n--- GENESIS: fund covenant UTXO P2SH(state0) with derived covenant_id ---");
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
        let funding = cands
            .into_iter()
            .next()
            .ok_or("no UTXO covers genesis value+fee")?;
        let funding_outpoint =
            TransactionOutpoint::new(funding.outpoint.transaction_id, funding.outpoint.index);
        let fund_amount = funding.utxo_entry.amount;

        // If funding leaves only a small change, absorb it into the covenant
        // output (a tiny change output blows the KIP-9 storage-mass cap).
        let change_at_default = fund_amount.saturating_sub(GENESIS_VALUE_SOMPI + CARRIER_FEE_SOMPI);
        let (cov_value, change) = if change_at_default >= MIN_CHANGE_FOR_MASS_SOMPI {
            (GENESIS_VALUE_SOMPI, change_at_default)
        } else {
            (fund_amount - CARRIER_FEE_SOMPI, 0) // single covenant output
        };
        let (gen_out, cov_id) = build_genesis_output(cov_value, funding_outpoint, &spk0);
        println!("covenant_id: {cov_id}  (covenant value={cov_value} sompi, change={change})");
        let mut gen_outputs = vec![gen_out];
        if change > 0 {
            gen_outputs.push(TransactionOutput::new(
                change,
                pay_to_address_script(&wallet.address),
            ));
        }
        let gen_input = TransactionInput::new(funding_outpoint, vec![], 0, 0);
        let gen_tx = Transaction::new(
            TX_VERSION_TOCCATA,
            vec![gen_input],
            gen_outputs,
            0,
            SUBNETWORK_ID_NATIVE,
            0,
            vec![],
        );
        let fund_entry = UtxoEntry::new(
            fund_amount,
            funding.utxo_entry.script_public_key.clone(),
            funding.utxo_entry.block_daa_score,
            funding.utxo_entry.is_coinbase,
            funding.utxo_entry.covenant_id,
        );
        CovenantsContext::from_tx(&PopulatedTransaction::new(
            &gen_tx,
            vec![fund_entry.clone()],
        ))
        .map_err(|e| format!("genesis covenant preflight: {e:?}"))?;
        println!("genesis preflight: covenant_id reconstruction OK");

        let signed = sign(
            SignableTransaction::with_entries(gen_tx, vec![fund_entry]),
            wallet.keypair,
        );
        let genesis_txid = rpc
            .submit_transaction((&signed.tx).into(), false)
            .await
            .map_err(|e| format!("submit genesis: {e}"))?;
        println!("genesis tx_id: {genesis_txid}");
        let genesis_tid: TransactionId = genesis_txid;

        // ── APPEND ──────────────────────────────────────────────────────────────
        println!("\n--- APPEND: spend covenant UTXO → P2SH(state1) seq 0→1 (oracle-signed) ---");
        let p2sh0_addr = extract_script_pub_key_address(&spk0, Prefix::Testnet)
            .map_err(|e| format!("p2sh0 address: {e}"))?;

        let mut append_txid = String::new();
        for attempt in 1..=40u32 {
            let utxos = rpc
                .get_utxos_by_addresses(vec![p2sh0_addr.clone()])
                .await
                .map_err(|e| format!("get_utxos(p2sh0): {e}"))?;
            let Some(cov_utxo) = utxos
                .into_iter()
                .find(|e| e.outpoint.transaction_id == genesis_tid && e.outpoint.index == 0)
            else {
                if attempt < 40 {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                return Err("genesis covenant UTXO not confirmed after 80s".into());
            };
            let amount = cov_utxo.utxo_entry.amount;
            let daa = cov_utxo.utxo_entry.block_daa_score;
            if cov_utxo.utxo_entry.covenant_id != Some(cov_id) {
                return Err(format!(
                    "node covenant_id {:?} != derived {cov_id}",
                    cov_utxo.utxo_entry.covenant_id
                )
                .into());
            }

            let genesis_outpoint = TransactionOutpoint::new(genesis_tid, 0);
            let (tx, used) = build_and_preflight_append(
                &cap,
                &spk0,
                &spk1,
                genesis_outpoint,
                amount,
                daa,
                cov_id,
                &oracle,
            )?;
            println!("append preflight: ACCEPT, used_script_units={used} (covenants_enabled, real CovenantsContext)");

            match rpc.submit_transaction((&tx).into(), false).await {
                Ok(id) => {
                    append_txid = id.to_string();
                    break;
                }
                Err(e) if is_transient(&e) && attempt < 40 => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => return Err(format!("submit append: {e}").into()),
            }
        }
        if append_txid.is_empty() {
            return Err("append not submitted".into());
        }
        println!("append tx_id: {append_txid}");

        // ── FACTS-ready: KCP-RE-003 ───────────────────────────────────────────────
        println!("\n--- FACTS.yaml-ready: KCP-RE-003 ---");
        println!("  network: {}", info.network_id);
        println!("  covenant_id: {cov_id}");
        println!("  genesis_tx: {genesis_txid}");
        println!("  append_tx: {append_txid}");
        println!("  spk0(P2SH state0 seq0): {}", hex::encode(spk0.script()));
        println!("  spk1(P2SH state1 seq1): {}", hex::encode(spk1.script()));
        println!(
            "  oracle_xonly_pk: {}",
            hex::encode(oracle.x_only_public_key().0.serialize())
        );
        println!("  note: FIRST live covenant-id-bound deployment of the anchor-only reserve covenant; \
                  append engine-preflighted with CovenantsContext (covenants_enabled); anchor-only (gates nothing); \
                  v0 unaudited synthetic; sigscript = embedded silverc bytes + spliced oracle sig (no silverscript-lang dep).");
        Ok(())
    }
}
