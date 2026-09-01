//! CsciInstrument silverscript-covenant settlement — the self-enforcing layer.
//!
//! Settles the REAL silverscript covenant `CsciInstrument.sil` (portrait repo),
//! whose compiled `settle()` enforces, ON-CHAIN (consensus, not the vProg):
//!   require(proof_cov_id == OpInputCovenantId(0));   // covenant-id binding
//!   require(checkSig(auth, prev_states[0].owner));   // committed-owner auth
//!   seq := prev.seq + 1                              // seq monotonicity
//!
//! The covenant scripts + settle sigscript bytes are CAPTURED ONCE from
//! silverscript-lang (`csci-capture.json`, KCP_CSCI_CAPTURE) so this binary has
//! no silverscript-lang dependency — same discipline as reserve_covenant_live.
//! The only tx-dependent piece (the owner's 65-byte Schnorr sig) is spliced into
//! the captured sigscript at build time. Every spend is engine-preflighted
//! against the REAL pinned v2.0.0 engine (90dbf07) with a real CovenantsContext
//! before submit.
//!
//! Modes (KCP_MODE):
//!   dryrun  (default) — offline v2.0.0-engine proof: VALID settle ACCEPTS,
//!                       invalid-seq settle REJECTS. No node, no funds.
//!   live              — genesis lock P2SH(state0) + settle spend → P2SH(state1).
//!   negctl            — like live but the output is P2SH(wrong-seq state) so the
//!                       covenant's own silverscript require REJECTS at the node.
//!
//! Network (KCP_NET): `testnet` (default) or `mainnet`. Unset ⇒ testnet, exactly
//! as before (node URL default ws://127.0.0.1:17210, suffix KCP_NET_SUFFIX|10,
//! testnet address prefixes). `mainnet` selects the mainnet node (KCP_NODE_URL
//! default ws://127.0.0.1:17110) and mainnet address prefixes. `dryrun` stays
//! fully offline regardless of KCP_NET. Any mainnet SUBMIT is REFUSED unless
//! KCP_MAINNET_CONFIRM=yes-move-real-kas-on-mainnet is set — it moves real funds.
//!
//! Status: v0 — pre-production — unaudited. Testnet is the default, supported
//! path; the mainnet settle path is newly wired behind the KCP_MAINNET_CONFIRM
//! confirm-gate.

use std::env;
use std::path::Path;

use kaspa_consensus_core::hashing::sighash::SigHashReusedValuesUnsync;
use kaspa_consensus_core::{
    constants::TX_VERSION_TOCCATA,
    hashing::covenant_id::covenant_id,
    mass::ComputeBudget,
    sign::sign,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        CovenantBinding, PopulatedTransaction, ScriptPublicKey, SignableTransaction, Transaction,
        TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
    },
    Hash,
};
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_txscript::{
    caches::Cache, covenants::CovenantsContext, extract_script_pub_key_address,
    pay_to_address_script, pay_to_script_hash_script, EngineCtx, EngineFlags, TxScriptEngine,
};

use kcp_common::{
    p2sh::{p2sh_input_sighash, schnorr_satisfier_sig},
    tx::CARRIER_FEE_SOMPI,
    wallet::{Prefix, Wallet},
    wrpc::{NodeClient, NodeConfig},
};

type BoxError = Box<dyn std::error::Error>;

const GENESIS_VALUE_SOMPI: u64 = 100_000_000; // 1 TKAS into the covenant UTXO

/// Select the network from `KCP_NET` (default `testnet`). Returns the node
/// config, the address prefix, the substring the node's reported `network_id`
/// must contain (the network guard), and whether this is mainnet. With `KCP_NET`
/// unset the result is byte-for-byte the prior testnet behaviour: node URL
/// default ws://127.0.0.1:17210, suffix from `KCP_NET_SUFFIX` (default 10),
/// `Prefix::Testnet`, guard "testnet".
fn net_from_env() -> (NodeConfig, Prefix, &'static str, bool) {
    let net = env::var("KCP_NET").unwrap_or_else(|_| "testnet".to_string());
    if net == "mainnet" {
        let url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://127.0.0.1:17110".to_string());
        (NodeConfig::mainnet(url), Prefix::Mainnet, "mainnet", true)
    } else {
        let url = env::var("KCP_NODE_URL").unwrap_or_else(|_| "ws://127.0.0.1:17210".to_string());
        let suffix: u32 = env::var("KCP_NET_SUFFIX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        (
            NodeConfig::testnet(url, suffix),
            Prefix::Testnet,
            "testnet",
            false,
        )
    }
}

/// Fund-safety gate: a mainnet broadcast is only permitted when the operator has
/// explicitly set `KCP_MAINNET_CONFIRM` to the exact required phrase. Testnet is
/// never gated (`is_mainnet == false` ⇒ always Ok). Called immediately after the
/// network guard in every submit path, before any funds can move.
fn require_mainnet_confirm(is_mainnet: bool) -> Result<(), BoxError> {
    const PHRASE: &str = "yes-move-real-kas-on-mainnet";
    if is_mainnet && env::var("KCP_MAINNET_CONFIRM").ok().as_deref() != Some(PHRASE) {
        return Err(
            format!("REFUSED: mainnet submit requires KCP_MAINNET_CONFIRM={PHRASE}").into(),
        );
    }
    Ok(())
}

struct Capture {
    state0_script: Vec<u8>,
    state1_script: Vec<u8>,
    // Full settle leader sigscript template (with a placeholder 65-byte sig and a
    // placeholder 32-byte proof_cov_id). At build time we splice the real sig at
    // `sig_off` and the real proof_cov_id (= OpInputCovenantId(0) = the engine's
    // per-instance covenant_id) at `pcid_off`.
    template: Vec<u8>,
    sig_off: usize,  // offset of the 65-byte sig region
    pcid_off: usize, // offset of the 32-byte proof_cov_id region
}

fn h(s: &str) -> Vec<u8> {
    hex::decode(s).expect("hex")
}

fn load_capture(path: &str) -> Result<Capture, BoxError> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let g = |k: &str| -> Result<Vec<u8>, BoxError> {
        Ok(h(v
            .get(k)
            .and_then(|x| x.as_str())
            .ok_or(format!("missing {k}"))?))
    };
    let dummy_a = g("settle_sigscript_dummyA_hex")?;
    let dummy_b = g("settle_sigscript_dummyB_hex")?;
    if dummy_a.len() != dummy_b.len() {
        return Err("dummy sigscripts differ in length".into());
    }
    // Locate the 65-byte sig region by diffing the two dummy captures.
    let diffs: Vec<usize> = (0..dummy_a.len())
        .filter(|&i| dummy_a[i] != dummy_b[i])
        .collect();
    if diffs.len() != 65 || diffs != (diffs[0]..diffs[0] + 65).collect::<Vec<_>>() {
        return Err(format!("sig region not a contiguous 65-byte run: {diffs:?}").into());
    }
    let sig_off = diffs[0];
    // Locate the 32-byte proof_cov_id region by searching for the captured value.
    let pcid = g("proof_cov_id_hex")?;
    let pcid_off = dummy_a
        .windows(32)
        .position(|w| w == pcid.as_slice())
        .ok_or("proof_cov_id not found in sigscript")?;
    Ok(Capture {
        state0_script: g("state0_script_hex")?,
        state1_script: g("state1_script_hex")?,
        template: dummy_a,
        sig_off,
        pcid_off,
    })
}

/// Build the settle sigscript by splicing the real sig and the real proof_cov_id
/// (= the engine per-instance covenant_id, what OpInputCovenantId(0) returns).
fn settle_sigscript(cap: &Capture, sig65: &[u8], proof_cov_id: &[u8; 32]) -> Vec<u8> {
    let mut s = cap.template.clone();
    s[cap.sig_off..cap.sig_off + 65].copy_from_slice(sig65);
    s[cap.pcid_off..cap.pcid_off + 32].copy_from_slice(proof_cov_id);
    s
}

/// Run a covenant spend through the real v2.0.0 engine with a real CovenantsContext.
/// Returns consumed script units on accept.
fn covenant_engine_run(
    tx: &Transaction,
    idx: usize,
    entries: &[UtxoEntry],
) -> Result<u64, BoxError> {
    let populated = PopulatedTransaction::new(tx, entries.to_vec());
    let cov_ctx = CovenantsContext::from_tx(&populated)
        .map_err(|e| format!("CovenantsContext::from_tx: {e:?}"))?;
    let utxo =
        kaspa_consensus_core::tx::VerifiableTransaction::utxo(&populated, idx).ok_or("no utxo")?;
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

/// Build genesis covenant output P2SH(state0) + its derived covenant_id.
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

/// Build + engine-preflight the settle spending the covenant UTXO, continuing to
/// `out_spk` (P2SH(state1) for a valid spend; P2SH(wrong-seq) for the negctl).
/// Two-round budget: the sig commits the input budget, so measure then re-sign.
#[allow(clippy::too_many_arguments)]
fn build_and_preflight_settle(
    cap: &Capture,
    spk0: &ScriptPublicKey,
    out_spk: &ScriptPublicKey,
    genesis_outpoint: TransactionOutpoint,
    amount: u64,
    daa: u64,
    cov_id: Hash,
    signer: &kaspa_bip32::secp256k1::Keypair,
) -> Result<(Transaction, u64), BoxError> {
    let mut out = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, out_spk.clone());
    out.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id,
    });
    let input = TransactionInput::new(genesis_outpoint, vec![], 0, 0);
    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![input],
        vec![out],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let input_entry = UtxoEntry::new(amount, spk0.clone(), daa, false, Some(cov_id));
    // proof_cov_id MUST equal OpInputCovenantId(0) = the engine per-instance
    // covenant_id, or the silverscript `require(proof_cov_id == OpInputCovenantId(0))`
    // fails. (The vProg STARK journal separately binds the sha256 KovId — see
    // PROVENANCE: these are two different 32-byte values; the cross-binding gap.)
    let pcid: [u8; 32] = cov_id.as_bytes();

    let sigscript_for = |tx: &Transaction| -> Vec<u8> {
        let sighash = p2sh_input_sighash(tx, std::slice::from_ref(&input_entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, signer); // 65 B
        settle_sigscript(cap, &sig, &pcid)
    };

    tx.inputs[0].compute_commit = ComputeBudget(u16::MAX).into();
    tx.inputs[0].signature_script = sigscript_for(&tx);
    let used = covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry))?;

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

fn dry_run(cap: &Capture, owner: &kaspa_bip32::secp256k1::Keypair) -> Result<(), BoxError> {
    println!("=== CSCI silverscript DRY RUN — offline v2.0.0-engine proof (no node) ===");
    let spk0 = pay_to_script_hash_script(&cap.state0_script);
    let spk1 = pay_to_script_hash_script(&cap.state1_script);

    let funding = TransactionOutpoint::new(TransactionId::from_bytes([0xab; 32]), 0);
    let (_gen_out, cov_id) = build_genesis_output(GENESIS_VALUE_SOMPI, funding, &spk0);
    println!("derived covenant_id (engine): {cov_id}");

    let synth_genesis = TransactionOutpoint::new(TransactionId::from_bytes([0xcd; 32]), 0);

    // POSITIVE: valid settle seq 0→1, owner-signed, output P2SH(state1).
    let (_tx, used) = build_and_preflight_settle(
        cap,
        &spk0,
        &spk1,
        synth_genesis,
        GENESIS_VALUE_SOMPI,
        0,
        cov_id,
        owner,
    )?;
    println!("[1] valid settle (seq 0→1, owner-signed): ACCEPT, used_script_units={used}");

    // NEGATIVE (seq): output recreates seq0 (P2SH(state0)) — seq not incremented.
    match build_and_preflight_settle(
        cap,
        &spk0,
        &spk0,
        synth_genesis,
        GENESIS_VALUE_SOMPI,
        0,
        cov_id,
        owner,
    ) {
        Err(e) => {
            println!("[2] settle with non-incremented seq (output=state0): REJECT ✓\n      {e}")
        }
        Ok(_) => return Err("NEG(seq) wrongly ACCEPTED (seq not incremented)".into()),
    }

    // NEGATIVE (auth): wrong owner signature.
    let impostor = kaspa_bip32::secp256k1::Keypair::from_seckey_slice(
        kaspa_bip32::secp256k1::SECP256K1,
        &[0x9u8; 32],
    )?;
    match build_and_preflight_settle(
        cap,
        &spk0,
        &spk1,
        synth_genesis,
        GENESIS_VALUE_SOMPI,
        0,
        cov_id,
        &impostor,
    ) {
        Err(e) => println!("[3] settle with wrong owner sig: REJECT ✓\n      {e}"),
        Ok(_) => return Err("NEG(auth) wrongly ACCEPTED (wrong owner sig)".into()),
    }

    println!("DRY RUN PASSED — silverscript seq+auth enforcement is engine-valid on v2.0.0.");
    Ok(())
}

/// Read `<dir>/<name>.hex` or `succinct.<name>.hex`.
fn read_proof_hex(dir: &Path, name: &str) -> Result<Vec<u8>, BoxError> {
    let plain = dir.join(format!("{name}.hex"));
    let succ = dir.join(format!("succinct.{name}.hex"));
    let p = if plain.exists() { plain } else { succ };
    Ok(hex::decode(std::fs::read_to_string(&p)?.trim())?)
}

/// Build the tag-0x21 verifier redeem: <image_id> <control_id> <hashfn> <tag> OpZkPrecompile.
fn build_zk_redeem(dir: &Path) -> Result<(Vec<u8>, Vec<Vec<u8>>), BoxError> {
    const OP_ZK: u8 = 0xa6;
    let image = if dir.join("image.hex").exists() {
        "image"
    } else {
        "image_id"
    };
    let image_id = read_proof_hex(dir, image)?;
    let control_id = read_proof_hex(dir, "control_id")?;
    let claim = read_proof_hex(dir, "claim")?;
    let control_index = read_proof_hex(dir, "control_index")?;
    let control_digests = read_proof_hex(dir, "control_digests")?;
    let seal = read_proof_hex(dir, "seal")?;
    let journal = {
        let j = read_proof_hex(dir, "journal")?;
        if j.len() == 32 {
            j
        } else {
            read_proof_hex(dir, "journal_hash")?
        }
    };
    let mut redeem = Vec::new();
    push_zk(&mut redeem, &image_id);
    push_zk(&mut redeem, &control_id);
    push_zk(&mut redeem, &[1]); // hashfn poseidon2
    push_zk(&mut redeem, &[0x21]); // tag
    redeem.push(OP_ZK);
    // satisfier elements pushed in the signature script (bottom→top):
    let satisfier = vec![claim, control_index, control_digests, seal, journal];
    Ok((redeem, satisfier))
}

fn push_zk(s: &mut Vec<u8>, d: &[u8]) {
    match d.len() {
        0 => s.push(0x00),
        n @ 1..=75 => {
            s.push(n as u8);
            s.extend_from_slice(d);
        }
        n @ 76..=255 => {
            s.push(0x4c);
            s.push(n as u8);
            s.extend_from_slice(d);
        }
        n @ 256..=65535 => {
            s.push(0x4d);
            s.push((n & 0xff) as u8);
            s.push((n >> 8) as u8);
            s.extend_from_slice(d);
        }
        n => {
            s.push(0x4e);
            s.extend_from_slice(&(n as u32).to_le_bytes());
            s.extend_from_slice(d);
        }
    }
}

/// ITEM 2 (offline investigation): can ONE raw P2SH redeem do BOTH the
/// silverscript covenant rules AND the tag-0x21 verify in a single input?
///
/// Compose redeem = [covenant_script] OP_VERIFY [image_id][control_id][hashfn][tag] OpZkPrecompile,
/// and a sigscript that pushes the covenant witness AND the zk satisfier, then the
/// composed redeem. Run through the real v2.0.0 engine and report EXACTLY what
/// happens (accept, or the precise opcode/limit that blocks it). No broadcast.
fn item2_single_redeem(cap: &Capture, proof_dir: &Path) -> Result<(), BoxError> {
    use kaspa_txscript::{max_script_element_size, max_scripts_size};
    println!("=== ITEM 2 — single composed redeem (covenant + tag-0x21), v2.0.0 ===");

    // Build the two halves.
    let (zk_redeem, zk_satisfier) = build_zk_redeem(proof_dir)?;
    // zk_redeem = [image_id][control_id][hashfn][tag] OpZkPrecompile. Reuse as the
    // tag-0x21 tail of the composed redeem.
    // Composed redeem = covenant_script  OP_VERIFY  zk_redeem.
    const OP_VERIFY: u8 = 0x69;
    let mut composed = cap.state0_script.clone();
    composed.push(OP_VERIFY);
    composed.extend_from_slice(&zk_redeem);
    println!(
        "composed redeem: {} bytes (covenant {} + OP_VERIFY 1 + zk {})",
        composed.len(),
        cap.state0_script.len(),
        zk_redeem.len()
    );
    println!(
        "engine limits (covenants_enabled): max_scripts_size={}, max_script_element_size={}",
        max_scripts_size(true),
        max_script_element_size(true)
    );
    if composed.len() > max_scripts_size(true) {
        println!(
            "BLOCKER: composed redeem {} > max_scripts_size {} — single redeem too large.",
            composed.len(),
            max_scripts_size(true)
        );
        return Ok(());
    }
    println!("  (composed redeem is within the script-size limit)");

    // The decisive structural problem, stated honestly, then tested:
    //   * The covenant script is generated by silverc as a STANDALONE P2SH redeem.
    //     It runs a selector dispatch (leader/delegate) and, on the leader path,
    //     consumes its OWN witness (sig, next_state_hash, proof_cov_id, selector,
    //     and reads prev-state via OpInputCovenantId / introspection) and leaves
    //     a single bool. It is NOT written to leave the zk satisfier (claim, seal,
    //     journal, ...) untouched beneath its result.
    //   * After OP_VERIFY consumes the covenant's bool, the zk tail expects the 5
    //     zk satisfier elements to be the TOP of the stack in the precise order
    //     [claim, control_index, control_digests, seal, journal]. For that to hold,
    //     those must have been pushed in the sigscript BELOW the covenant witness
    //     and survive covenant execution unconsumed.
    //
    // Try the most faithful layout: sigscript = zk_satisfier (bottom) then the
    // covenant witness prefix (the captured settle sigscript MINUS its redeem push)
    // then the composed redeem. Run it and report the engine's verdict.

    // Extract the covenant witness prefix from the capture template: everything
    // before the final push of the covenant script (the P2SH redeem push).
    let tmpl = &cap.template;
    let cov_script = &cap.state0_script;
    // The redeem push is the last occurrence of the covenant script bytes in the template.
    let redeem_push_start = tmpl
        .windows(cov_script.len())
        .rposition(|w| w == cov_script.as_slice())
        .ok_or("covenant redeem push not found in template")?;
    // Back up over the push-opcode/length prefix (OP_PUSHDATA2 for a 524-byte push = 3 bytes).
    let push_prefix_len = if cov_script.len() <= 75 {
        1
    } else if cov_script.len() <= 255 {
        2
    } else {
        3
    };
    let witness_prefix = &tmpl[..redeem_push_start - push_prefix_len];
    println!("covenant witness prefix: {} bytes", witness_prefix.len());

    // Variant A — covenant first: sigscript = zk satisfier (bottom) + covenant
    // witness, then the composed redeem push.
    let mut sigscript = Vec::new();
    for el in &zk_satisfier {
        push_zk(&mut sigscript, el);
    }
    sigscript.extend_from_slice(witness_prefix);
    push_zk(&mut sigscript, &composed); // P2SH: redeem pushed last

    // Build a synthetic spend of P2SH(composed) carrying the covenant binding.
    let composed_spk = pay_to_script_hash_script(&composed);
    let funding = TransactionOutpoint::new(TransactionId::from_bytes([0xab; 32]), 0);
    let out = TransactionOutput::new(GENESIS_VALUE_SOMPI, composed_spk.clone());
    let cov_id = covenant_id(funding, std::iter::once((0u32, &out)));
    let spend_outpoint = TransactionOutpoint::new(TransactionId::from_bytes([0xcd; 32]), 0);
    // Continuation output P2SH(state1) with the binding (mode=transition).
    let spk1 = pay_to_script_hash_script(&cap.state1_script);
    let mut cont = TransactionOutput::new(GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI, spk1);
    cont.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id,
    });
    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(spend_outpoint, vec![], 0, 0)],
        vec![cont],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    tx.inputs[0].compute_commit = ComputeBudget(u16::MAX).into();
    tx.inputs[0].signature_script = sigscript;
    let input_entry = UtxoEntry::new(GENESIS_VALUE_SOMPI, composed_spk, 0, false, Some(cov_id));

    println!("[variant A] covenant-first: [covenant] OP_VERIFY [zk] ... ");
    match covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry)) {
        Ok(units) => {
            println!("  ACCEPT ({units} units) — single composed redeem runs BOTH layers.")
        }
        Err(e) => println!("  REJECT — {e}"),
    }

    // Variant B — zk-first: composed = zk_redeem OP_VERIFY covenant_script. The zk
    // verify runs and is OP_VERIFY'd BEFORE the covenant's terminal OP_DROPs could
    // clobber the stack. sigscript = covenant witness (bottom) + zk satisfier.
    let mut composed_b = zk_redeem.clone();
    composed_b.push(OP_VERIFY);
    composed_b.extend_from_slice(&cap.state0_script);
    let composed_b_spk = pay_to_script_hash_script(&composed_b);
    let out_b = TransactionOutput::new(GENESIS_VALUE_SOMPI, composed_b_spk.clone());
    let cov_id_b = covenant_id(funding, std::iter::once((0u32, &out_b)));
    let mut sig_b = Vec::new();
    sig_b.extend_from_slice(witness_prefix);
    for el in &zk_satisfier {
        push_zk(&mut sig_b, el);
    }
    push_zk(&mut sig_b, &composed_b);
    let spk1b = pay_to_script_hash_script(&cap.state1_script);
    let mut cont_b = TransactionOutput::new(GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI, spk1b);
    cont_b.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id_b,
    });
    let mut tx_b = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(spend_outpoint, vec![], 0, 0)],
        vec![cont_b],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    tx_b.inputs[0].compute_commit = ComputeBudget(u16::MAX).into();
    tx_b.inputs[0].signature_script = sig_b;
    let entry_b = UtxoEntry::new(
        GENESIS_VALUE_SOMPI,
        composed_b_spk,
        0,
        false,
        Some(cov_id_b),
    );
    println!("[variant B] zk-first:       [zk] OP_VERIFY [covenant] ... ");
    let mut any_accept = false;
    match covenant_engine_run(&tx_b, 0, std::slice::from_ref(&entry_b)) {
        Ok(units) => {
            println!("  ACCEPT ({units} units) — single composed redeem runs BOTH layers.");
            any_accept = true;
        }
        Err(e) => println!("  REJECT — {e}"),
    }

    println!();
    if any_accept {
        println!("ITEM 2 RESULT: a single raw P2SH redeem CAN do both (one of the layouts ran).");
    } else {
        println!(
            "ITEM 2 RESULT: a single composed raw redeem is REJECTED in both layouts on v2.0.0."
        );
        println!(
            "  HONEST READING: silverc emits the covenant as a SELF-CONTAINED P2SH redeem with"
        );
        println!(
            "  its own selector dispatch and TERMINAL OP_DROPs that clear the stack to leave a"
        );
        println!(
            "  single bool — so it neither preserves the tag-0x21 satisfier beneath its result"
        );
        println!(
            "  (variant A) nor tolerates extra items left on the stack from a prior zk verify"
        );
        println!(
            "  (variant B). It also never invokes OpZkPrecompile. Concatenating the raw bytes"
        );
        println!("  therefore breaks the engine's stack/selector semantics. A clean SINGLE-script");
        println!(
            "  binding needs the silverscript-surface OpZkPrecompile opcode (intentionally NOT"
        );
        println!(
            "  pursued — no upstream). The IN-HOUSE way to enforce both is the 2-input combined"
        );
        println!("  tx (ITEM 1), which is LIVE on TN10. So item 2 honestly REMAINS 'needs the");
        println!("  surface opcode'; item 1 stands as the achieved combined on-chain enforcement.");
    }
    Ok(())
}

/// COMBINED 2-input offline proof: one tx where input[0] is the CsciInstrument
/// silverscript covenant UTXO (enforces seq/auth/cov-id-binding) AND input[1] is
/// the tag-0x21 P2SH(redeem) verifier UTXO (runs OpZkPrecompile over the real
/// STARK). BOTH inputs are run through the real v2.0.0 engine; the tx is only
/// "spendable" if both accept → genuinely combined on-chain co-enforcement.
fn combined_dry_run(
    cap: &Capture,
    owner: &kaspa_bip32::secp256k1::Keypair,
    proof_dir: &Path,
) -> Result<(), BoxError> {
    use kcp_common::p2sh::{build_p2sh_signature_script, p2sh_lock_script};
    println!("=== COMBINED 2-input DRY RUN — silverscript covenant + tag-0x21, v2.0.0 ===");
    let spk0 = pay_to_script_hash_script(&cap.state0_script);
    let spk1 = pay_to_script_hash_script(&cap.state1_script);
    let (zk_redeem, zk_satisfier) = build_zk_redeem(proof_dir)?;
    let zk_spk = p2sh_lock_script(&zk_redeem);
    println!(
        "zk redeem: {} bytes; covenant state0 script: {} bytes",
        zk_redeem.len(),
        cap.state0_script.len()
    );

    // Synthetic genesis outpoints for input[0] (covenant) and input[1] (verifier).
    let cov_funding = TransactionOutpoint::new(TransactionId::from_bytes([0xab; 32]), 0);
    let (_g, cov_id) = build_genesis_output(GENESIS_VALUE_SOMPI, cov_funding, &spk0);
    let cov_outpoint = TransactionOutpoint::new(TransactionId::from_bytes([0xcd; 32]), 0);
    let zk_outpoint = TransactionOutpoint::new(TransactionId::from_bytes([0xef; 32]), 0);

    let cov_amount = GENESIS_VALUE_SOMPI;
    let zk_amount = GENESIS_VALUE_SOMPI;

    // Continuation output for the covenant (P2SH(state1) with the binding).
    let mut cont = TransactionOutput::new(cov_amount - CARRIER_FEE_SOMPI, spk1.clone());
    cont.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id,
    });
    // The verifier output is a plain pay-to-self (no covenant).
    let zk_out = TransactionOutput::new(zk_amount - CARRIER_FEE_SOMPI, spk0.clone());

    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![
            TransactionInput::new(cov_outpoint, vec![], 0, 0),
            TransactionInput::new(zk_outpoint, vec![], 0, 0),
        ],
        vec![cont, zk_out],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    // Compute budgets: covenant input needs ~14 units; the zk input needs ~255
    // sigops worth (it is version-1 so uses ComputeBudget; ~25M units / 10_000 = 2500).
    tx.inputs[0].compute_commit = ComputeBudget(20).into();
    tx.inputs[1].compute_commit = ComputeBudget(u16::MAX).into();

    let cov_entry = UtxoEntry::new(cov_amount, spk0.clone(), 0, false, Some(cov_id));
    let zk_entry = UtxoEntry::new(zk_amount, zk_spk.clone(), 0, false, None);
    let entries = vec![cov_entry.clone(), zk_entry.clone()];

    // Sign + assemble input[0] (covenant) — sighash binds the whole 2-in/2-out tx.
    let pcid: [u8; 32] = cov_id.as_bytes();
    let cov_sighash = p2sh_input_sighash(&tx, &entries, 0);
    let cov_sig = schnorr_satisfier_sig(&cov_sighash, owner);
    tx.inputs[0].signature_script = settle_sigscript(cap, &cov_sig, &pcid);
    // Assemble input[1] (verifier) — no signature needed (OpZkPrecompile only).
    tx.inputs[1].signature_script = build_p2sh_signature_script(&zk_satisfier, &zk_redeem)?;

    // Run BOTH inputs through the real engine with one shared CovenantsContext.
    let populated = PopulatedTransaction::new(&tx, entries.clone());
    let cov_ctx = CovenantsContext::from_tx(&populated)
        .map_err(|e| format!("CovenantsContext::from_tx: {e:?}"))?;
    for idx in 0..2usize {
        let utxo = kaspa_consensus_core::tx::VerifiableTransaction::utxo(&populated, idx)
            .ok_or("no utxo")?;
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
            .map_err(|e| format!("input[{idx}] rejected: {e:?}"))?;
        let layer = if idx == 0 {
            "silverscript covenant (seq/auth/cov-id)"
        } else {
            "tag-0x21 STARK verifier"
        };
        println!(
            "  input[{idx}] {layer}: ACCEPT ({} script units)",
            vm.used_script_units().0
        );
    }
    println!("COMBINED DRY RUN PASSED — BOTH layers accept in ONE tx on v2.0.0.");
    println!("NOTE: this dry-run uses SYNTHETIC outpoints, so its engine covid ({cov_id})");
    println!("  does not match a real proof's journal covid. Genuine CROSS-BINDING to the");
    println!("  per-instance covid is demonstrated LIVE by KCP_MODE=settle-bound (the journal");
    println!("  commits OpInputCovenantId(0)) and KCP_MODE=xbind-negctl (a proof bound to");
    println!("  covid_A is rejected when spending a different instance covid_B). See PROVENANCE.");
    Ok(())
}

/// Read the 32-byte covenant_id committed at journal[0..32] of a proof dir's
/// raw 104-byte journal (journal.hex when 104 B). Used to CHECK that the proof
/// is bound to the per-instance covid we are about to spend (cross-binding).
fn proof_journal_covid(dir: &Path) -> Result<[u8; 32], BoxError> {
    let j = read_proof_hex(dir, "journal")?;
    if j.len() != 104 {
        return Err(format!(
            "journal.hex is {} bytes, expected the raw 104-byte journal",
            j.len()
        )
        .into());
    }
    let mut covid = [0u8; 32];
    covid.copy_from_slice(&j[0..32]);
    Ok(covid)
}

/// PHASE 1 of the cross-binding bootstrap: lock the CsciInstrument covenant P2SH
/// and print the per-instance covenant_id = covenant_id(funding_outpoint,[output])
/// that OpInputCovenantId(0) will return when this UTXO is spent. Does NOT spend.
async fn lock_only(cap: &Capture, wallet: &Wallet) -> Result<(), BoxError> {
    let (node_cfg, _prefix, guard, is_mainnet) = net_from_env();
    let node = NodeClient::new(node_cfg);
    let rpc = node.rpc().await?;
    let info = node.server_info().await?;
    if !info.network_id.contains(guard) {
        return Err(format!("REFUSED: '{}' is not {guard}", info.network_id).into());
    }
    require_mainnet_confirm(is_mainnet)?;
    let spk0 = pay_to_script_hash_script(&cap.state0_script);

    println!("--- LOCK (phase 1): fund covenant UTXO P2SH(state0) ---");
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
    let change = fund_amount - GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI;

    let (gen_out, cov_id) = build_genesis_output(GENESIS_VALUE_SOMPI, funding_outpoint, &spk0);
    let mut gen_outputs = vec![gen_out];
    if change >= 12_000_000 {
        gen_outputs.push(TransactionOutput::new(
            change,
            pay_to_address_script(&wallet.address),
        ));
    }
    let gen_tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(funding_outpoint, vec![], 0, 0)],
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
    let signed = sign(
        SignableTransaction::with_entries(gen_tx, vec![fund_entry]),
        wallet.keypair,
    );
    let genesis_txid = rpc
        .submit_transaction((&signed.tx).into(), false)
        .await
        .map_err(|e| format!("submit genesis: {e}"))?;
    println!("\n══ LOCKED ════════════════════════════════════════════════════════");
    println!("  genesis_txid:                 {genesis_txid}");
    println!("  per_instance_covenant_id:     {cov_id}");
    println!("  (next: generate the real proof with KCP_COVENANT_ID={cov_id},");
    println!("   then run KCP_MODE=settle-bound KCP_GENESIS_TXID={genesis_txid})");
    Ok(())
}

/// PHASE 3 of the cross-binding bootstrap: spend the already-locked covenant UTXO
/// (KCP_GENESIS_TXID) with a proof whose journal commits the SAME per-instance
/// covenant_id. Asserts journal[0..32] == the UTXO's on-chain covenant_id before
/// submitting, so a confirmed settle proves cross-binding.
async fn settle_bound(cap: &Capture, wallet: &Wallet) -> Result<(), BoxError> {
    let genesis_hex = env::var("KCP_GENESIS_TXID").map_err(|_| "KCP_GENESIS_TXID is required")?;
    let genesis_tid: TransactionId = genesis_hex
        .parse()
        .map_err(|e| format!("parse genesis txid: {e}"))?;
    let proof_dir = env::var("KCP_PROOF_DIR").map_err(|_| "KCP_PROOF_DIR is required")?;
    let proof_dir = Path::new(&proof_dir);

    let (node_cfg, prefix, guard, is_mainnet) = net_from_env();
    let node = NodeClient::new(node_cfg);
    let rpc = node.rpc().await?;
    let info = node.server_info().await?;
    if !info.network_id.contains(guard) {
        return Err(format!("REFUSED: '{}' is not {guard}", info.network_id).into());
    }
    require_mainnet_confirm(is_mainnet)?;
    let spk0 = pay_to_script_hash_script(&cap.state0_script);
    let spk1 = pay_to_script_hash_script(&cap.state1_script);
    let p2sh0_addr = extract_script_pub_key_address(&spk0, prefix)
        .map_err(|e| format!("p2sh0 address: {e}"))?;

    // Locate the locked covenant UTXO and read its ON-CHAIN covenant_id (ground truth).
    let utxos = rpc
        .get_utxos_by_addresses(vec![p2sh0_addr.clone()])
        .await
        .map_err(|e| format!("get_utxos(p2sh0): {e}"))?;
    let cov_utxo = utxos
        .into_iter()
        .find(|e| e.outpoint.transaction_id == genesis_tid && e.outpoint.index == 0)
        .ok_or("locked covenant UTXO not found at the covenant address")?;
    let amount = cov_utxo.utxo_entry.amount;
    let daa = cov_utxo.utxo_entry.block_daa_score;
    let onchain_cov_id = cov_utxo
        .utxo_entry
        .covenant_id
        .ok_or("UTXO has no covenant_id binding")?;
    println!("on-chain per-instance covenant_id: {onchain_cov_id}");

    // CROSS-BINDING CHECK: the proof's journal MUST commit this exact covid.
    let journal_covid = proof_journal_covid(proof_dir)?;
    if journal_covid != onchain_cov_id.as_bytes() {
        return Err(format!(
            "CROSS-BINDING MISMATCH: proof journal covid {} != on-chain covenant_id {} — \
             regenerate the proof with KCP_COVENANT_ID={}",
            hex::encode(journal_covid),
            onchain_cov_id,
            onchain_cov_id
        )
        .into());
    }
    println!("cross-binding CHECK: journal[0..32] == on-chain covenant_id ✓ ({onchain_cov_id})");

    // Spend the covenant: proof_cov_id spliced = OpInputCovenantId(0) = onchain_cov_id.
    let genesis_outpoint = TransactionOutpoint::new(genesis_tid, 0);
    let (tx, used) = build_and_preflight_settle(
        cap,
        &spk0,
        &spk1,
        genesis_outpoint,
        amount,
        daa,
        onchain_cov_id,
        &wallet.keypair,
    )?;
    println!("settle preflight: ACCEPT, used_script_units={used}");

    let settle_txid = rpc
        .submit_transaction((&tx).into(), false)
        .await
        .map_err(|e| format!("submit settle: {e}"))?;
    println!("\n══ CROSS-BOUND SETTLE ════════════════════════════════════════════");
    println!("  genesis_txid:               {genesis_hex}");
    println!("  per_instance_covenant_id:   {onchain_cov_id}");
    println!("  settle_txid:                {settle_txid}");
    println!("  journal commits the SAME per-instance covid → silverscript require");
    println!("  (proof_cov_id == OpInputCovenantId(0)) and the STARK journal reference");
    println!("  the SAME 32-byte id. Cross-bound within the silverscript input.");
    Ok(())
}

/// CROSS-BINDING negative control: lock a FRESH covenant instance B (covid_B),
/// then attempt to spend it presenting proof_cov_id = covid_A (the journal covid
/// from instance A's proof). Since OpInputCovenantId(0) = covid_B != covid_A, the
/// silverscript `require(proof_cov_id == OpInputCovenantId(0))` must REJECT. This
/// proves the binding is to the specific covenant identity (distinct from the
/// seq-violation and ZK-integrity rejects).
async fn xbind_negctl(cap: &Capture, wallet: &Wallet) -> Result<(), BoxError> {
    let proof_dir = env::var("KCP_PROOF_DIR").map_err(|_| "KCP_PROOF_DIR is required")?;
    let covid_a = proof_journal_covid(Path::new(&proof_dir))?; // journal's covid (instance A)

    let (node_cfg, prefix, guard, is_mainnet) = net_from_env();
    let node = NodeClient::new(node_cfg);
    let rpc = node.rpc().await?;
    let info = node.server_info().await?;
    if !info.network_id.contains(guard) {
        return Err(format!("REFUSED: '{}' is not {guard}", info.network_id).into());
    }
    require_mainnet_confirm(is_mainnet)?;
    let spk0 = pay_to_script_hash_script(&cap.state0_script);
    let spk1 = pay_to_script_hash_script(&cap.state1_script);

    // Lock a FRESH instance B.
    println!("--- xbind-negctl: lock a FRESH covenant instance B ---");
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
    let change = fund_amount - GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI;
    let (gen_out, covid_b) = build_genesis_output(GENESIS_VALUE_SOMPI, funding_outpoint, &spk0);
    println!("covid_A (proof journal):       {}", hex::encode(covid_a));
    println!("covid_B (fresh instance):      {covid_b}");
    if covid_a == covid_b.as_bytes() {
        return Err("covid_A == covid_B (unexpected) — cannot run cross-binding negctl".into());
    }
    let mut gen_outputs = vec![gen_out];
    if change >= 12_000_000 {
        gen_outputs.push(TransactionOutput::new(
            change,
            pay_to_address_script(&wallet.address),
        ));
    }
    let gen_tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(funding_outpoint, vec![], 0, 0)],
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
    let signed = sign(
        SignableTransaction::with_entries(gen_tx, vec![fund_entry]),
        wallet.keypair,
    );
    let genesis_txid = rpc
        .submit_transaction((&signed.tx).into(), false)
        .await
        .map_err(|e| format!("submit genesis B: {e}"))?;
    println!("instance-B genesis tx_id: {genesis_txid}");
    let genesis_tid: TransactionId = genesis_txid;

    // Wait for confirmation, then attempt the cross-mismatched spend.
    let p2sh0_addr = extract_script_pub_key_address(&spk0, prefix)
        .map_err(|e| format!("p2sh0 address: {e}"))?;
    let mut node_err = String::new();
    let mut accepted = String::new();
    for attempt in 1..=60u32 {
        let utxos = rpc
            .get_utxos_by_addresses(vec![p2sh0_addr.clone()])
            .await
            .map_err(|e| format!("get_utxos(p2sh0): {e}"))?;
        let Some(cov_utxo) = utxos
            .into_iter()
            .find(|e| e.outpoint.transaction_id == genesis_tid && e.outpoint.index == 0)
        else {
            if attempt < 60 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            return Err("instance-B covenant UTXO not confirmed".into());
        };
        let amount = cov_utxo.utxo_entry.amount;
        let daa = cov_utxo.utxo_entry.block_daa_score;
        let genesis_outpoint = TransactionOutpoint::new(genesis_tid, 0);

        // Build a spend of instance B but splice proof_cov_id = covid_A (the journal
        // covid). OpInputCovenantId(0) = covid_B != covid_A → require must fail.
        // Sign over the real tx; commit a realistic budget so the rejection is the
        // covenant require, not a mass error. Submit so the LIVE NODE rejects.
        let mut out = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, spk1.clone());
        out.covenant = Some(CovenantBinding {
            authorizing_input: 0,
            covenant_id: covid_b,
        });
        let input = TransactionInput::new(genesis_outpoint, vec![], 0, 0);
        let mut tx = Transaction::new(
            TX_VERSION_TOCCATA,
            vec![input],
            vec![out],
            0,
            SUBNETWORK_ID_NATIVE,
            0,
            vec![],
        );
        let input_entry = UtxoEntry::new(amount, spk0.clone(), daa, false, Some(covid_b));
        tx.inputs[0].compute_commit = ComputeBudget(20).into();
        let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&input_entry), 0);
        let sig = schnorr_satisfier_sig(&sighash, &wallet.keypair);
        tx.inputs[0].signature_script = settle_sigscript(cap, &sig, &covid_a); // covid_A spliced

        // Local engine should already reject (cross-mismatch); report it then submit.
        match covenant_engine_run(&tx, 0, std::slice::from_ref(&input_entry)) {
            Err(e) => println!("local v2.0.0 engine REJECTED (cross-mismatch, expected): {e}"),
            Ok(_) => {
                return Err("SECURITY FAILURE: local engine ACCEPTED covid_A on instance B".into())
            }
        }
        match rpc.submit_transaction((&tx).into(), false).await {
            Ok(id) => {
                accepted = id.to_string();
                break;
            }
            Err(e) => {
                node_err = e.to_string();
                break;
            }
        }
    }

    println!("\n══ CROSS-BINDING NEGATIVE CONTROL ════════════════════════════════");
    println!("  instance_B_genesis_txid: {genesis_txid}");
    println!("  covid_A (journal):       {}", hex::encode(covid_a));
    println!("  covid_B (spent instance):{covid_b}");
    if !accepted.is_empty() {
        return Err(format!(
            "SECURITY FAILURE: node ACCEPTED covid_A proof spending instance B as {accepted}"
        )
        .into());
    }
    println!("  NODE REJECTED ✓");
    println!("  node_error: {node_err}");
    Ok(())
}

/// ITEM 1 LIVE: broadcast ONE transaction whose acceptance requires BOTH inputs
/// to pass — input[0] = CsciInstrument silverscript covenant UTXO (seq/auth/
/// cov-id), input[1] = tag-0x21 P2SH(redeem) verifier UTXO (OpZkPrecompile over
/// the real STARK). Cross-bound: the covenant is locked first, its per-instance
/// covid read, and the proof's journal must commit that covid (so OpInputCovenantId(0)
/// and the proof reference the same id; the verifier redeem is built from the same proof).
///
/// `negctl=true` tampers the proof journal (input[1]) so the verifier MUST reject,
/// making the WHOLE combined tx rejected at the node.
async fn combined_live(cap: &Capture, wallet: &Wallet, negctl: bool) -> Result<(), BoxError> {
    use kcp_common::p2sh::{build_p2sh_signature_script, lock_to_p2sh_tx, p2sh_lock_script};

    let proof_dir = env::var("KCP_PROOF_DIR").map_err(|_| "KCP_PROOF_DIR is required")?;
    let proof_dir = Path::new(&proof_dir);
    let genesis_hex = env::var("KCP_GENESIS_TXID")
        .map_err(|_| "KCP_GENESIS_TXID is required (lock the covenant first with KCP_MODE=lock)")?;
    let genesis_tid: TransactionId = genesis_hex
        .parse()
        .map_err(|e| format!("parse genesis txid: {e}"))?;

    let (node_cfg, prefix, guard, is_mainnet) = net_from_env();
    let node = NodeClient::new(node_cfg);
    let rpc = node.rpc().await?;
    let info = node.server_info().await?;
    if !info.network_id.contains(guard) {
        return Err(format!("REFUSED: '{}' is not {guard}", info.network_id).into());
    }
    require_mainnet_confirm(is_mainnet)?;
    println!(
        "connected: server={} network={} synced={}  (mode: combined-{})",
        info.server_version,
        info.network_id,
        info.is_synced,
        if negctl { "negctl" } else { "live" }
    );

    let spk0 = pay_to_script_hash_script(&cap.state0_script);
    let spk1 = pay_to_script_hash_script(&cap.state1_script);
    let (zk_redeem, mut zk_satisfier) = build_zk_redeem(proof_dir)?;
    let zk_spk = p2sh_lock_script(&zk_redeem);

    // ── locate the already-locked covenant UTXO; read its on-chain covid ──────
    let p2sh0_addr = extract_script_pub_key_address(&spk0, prefix)
        .map_err(|e| format!("p2sh0 address: {e}"))?;
    let cov_utxo = rpc
        .get_utxos_by_addresses(vec![p2sh0_addr.clone()])
        .await
        .map_err(|e| format!("get_utxos(p2sh0): {e}"))?
        .into_iter()
        .find(|e| e.outpoint.transaction_id == genesis_tid && e.outpoint.index == 0)
        .ok_or("locked covenant UTXO not found — run KCP_MODE=lock first")?;
    let cov_amount = cov_utxo.utxo_entry.amount;
    let cov_daa = cov_utxo.utxo_entry.block_daa_score;
    let cov_id = cov_utxo
        .utxo_entry
        .covenant_id
        .ok_or("covenant UTXO has no covenant_id binding")?;
    println!("covenant per-instance covenant_id: {cov_id}");

    // Cross-binding check (skip for negctl, which intentionally breaks the proof).
    if !negctl {
        let journal_covid = proof_journal_covid(proof_dir)?;
        if journal_covid != cov_id.as_bytes() {
            return Err(format!(
                "CROSS-BINDING MISMATCH: journal covid {} != covenant covid {} (regen proof with KCP_COVENANT_ID={})",
                hex::encode(journal_covid),
                cov_id,
                cov_id
            )
            .into());
        }
        println!("cross-binding CHECK: journal[0..32] == covenant covid ✓");
    }

    // ── lock the verifier P2SH(redeem) UTXO ───────────────────────────────────
    println!(
        "--- lock verifier P2SH(redeem) ({} byte redeem) ---",
        zk_redeem.len()
    );
    let zk_lock_txid = lock_to_p2sh_tx(rpc.as_ref(), wallet, &zk_redeem, GENESIS_VALUE_SOMPI)
        .await
        .map_err(|e| format!("verifier lock: {e}"))?;
    println!("verifier lock tx: {zk_lock_txid}");
    let zk_lock_tid: TransactionId = zk_lock_txid
        .parse()
        .map_err(|e| format!("parse verifier lock txid: {e}"))?;
    let zk_p2sh_addr = extract_script_pub_key_address(&zk_spk, prefix)
        .map_err(|e| format!("zk p2sh address: {e}"))?;
    let mut zk_utxo = None;
    for _ in 1..=120u32 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Some(u) = rpc
            .get_utxos_by_addresses(vec![zk_p2sh_addr.clone()])
            .await
            .map_err(|e| format!("get_utxos(zk): {e}"))?
            .into_iter()
            .find(|e| e.outpoint.transaction_id == zk_lock_tid && e.outpoint.index == 0)
        {
            zk_utxo = Some(u);
            break;
        }
    }
    let zk_utxo = zk_utxo.ok_or("verifier P2SH UTXO did not confirm")?;
    let zk_amount = zk_utxo.utxo_entry.amount;
    let zk_daa = zk_utxo.utxo_entry.block_daa_score;
    println!("verifier P2SH UTXO confirmed");

    // For negctl: tamper the journal element (the 5th satisfier element) so the
    // STARK verification of input[1] fails at the node.
    if negctl {
        let j = zk_satisfier.last_mut().expect("journal element");
        j[0] ^= 0x01;
        println!("negctl: tampered the proof journal (input[1] STARK must fail)");
    }

    // ── build the combined 2-input / 2-output tx ──────────────────────────────
    let cov_outpoint = TransactionOutpoint::new(genesis_tid, 0);
    let zk_outpoint = TransactionOutpoint::new(zk_lock_tid, 0);
    // The node charges 10 sompi/mass; compute mass ~477k → fee ~4.77M sompi, plus
    // the standard min-fee headroom. Leave a generous 60M-sompi fee (0.6 TKAS),
    // taken entirely from the verifier output; the covenant continuation keeps its
    // value (minus the small carrier fee) so value-conservation holds.
    const COMBINED_FEE_SOMPI: u64 = 60_000_000;
    let mut cont = TransactionOutput::new(cov_amount - CARRIER_FEE_SOMPI, spk1.clone());
    cont.covenant = Some(CovenantBinding {
        authorizing_input: 0,
        covenant_id: cov_id,
    });
    let zk_out_value = zk_amount
        .checked_sub(COMBINED_FEE_SOMPI)
        .ok_or("verifier UTXO too small to cover the combined fee")?;
    let zk_out = TransactionOutput::new(zk_out_value, pay_to_address_script(&wallet.address));
    let mut tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![
            TransactionInput::new(cov_outpoint, vec![], 0, 0),
            TransactionInput::new(zk_outpoint, vec![], 0, 0),
        ],
        vec![cont, zk_out],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    // Compute mass = size*1 + spk_size*10 + 100*sum(budget_units), max 500_000.
    // The 222 KB seal already costs ~223k of size-mass, so the budgets must be
    // TIGHT: input[1] (tag-0x21, ~25.0M script units) needs ceil(25M/10_000)=~2500
    // budget units (~250k mass); input[0] (covenant, ~105k units) needs ~12. Total
    // ~223k + ~251k ≈ ~474k < 500k.
    tx.inputs[0].compute_commit = ComputeBudget(14).into();
    tx.inputs[1].compute_commit = ComputeBudget(2510).into();

    let cov_entry = UtxoEntry::new(cov_amount, spk0.clone(), cov_daa, false, Some(cov_id));
    let zk_entry = UtxoEntry::new(zk_amount, zk_spk.clone(), zk_daa, false, None);
    let entries = vec![cov_entry.clone(), zk_entry.clone()];

    // input[0]: covenant — sig over the whole 2-in/2-out tx; proof_cov_id = cov_id.
    let pcid: [u8; 32] = cov_id.as_bytes();
    let cov_sighash = p2sh_input_sighash(&tx, &entries, 0);
    let cov_sig = schnorr_satisfier_sig(&cov_sighash, &wallet.keypair);
    tx.inputs[0].signature_script = settle_sigscript(cap, &cov_sig, &pcid);
    // input[1]: verifier — satisfier pushes + redeem (no signature; OpZkPrecompile).
    tx.inputs[1].signature_script = build_p2sh_signature_script(&zk_satisfier, &zk_redeem)?;

    // Offline preflight of BOTH inputs (skip for negctl, which we WANT the node to reject).
    if !negctl {
        let populated = PopulatedTransaction::new(&tx, entries.clone());
        let cov_ctx = CovenantsContext::from_tx(&populated)
            .map_err(|e| format!("CovenantsContext::from_tx: {e:?}"))?;
        for idx in 0..2usize {
            let utxo = kaspa_consensus_core::tx::VerifiableTransaction::utxo(&populated, idx)
                .ok_or("no utxo")?;
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
                .map_err(|e| format!("combined preflight input[{idx}] rejected: {e:?}"))?;
            let layer = if idx == 0 {
                "silverscript covenant"
            } else {
                "tag-0x21 verifier"
            };
            println!(
                "  preflight input[{idx}] {layer}: ACCEPT ({} units)",
                vm.used_script_units().0
            );
        }
    }

    let submit = rpc.submit_transaction((&tx).into(), false).await;
    println!(
        "\n══ COMBINED {} ════════════════════════════════════════════════",
        if negctl {
            "NEGATIVE CONTROL"
        } else {
            "LIVE SETTLE"
        }
    );
    println!("  covenant_genesis_txid: {genesis_hex}");
    println!("  verifier_lock_txid:    {zk_lock_txid}");
    println!("  per_instance_covid:    {cov_id}");
    match submit {
        Ok(id) => {
            if negctl {
                return Err(format!("SECURITY FAILURE: combined negctl ACCEPTED as {id}").into());
            }
            println!("  combined_settle_txid:  {id}");
            println!("  → one tx, accepted ONLY because BOTH the silverscript covenant AND the");
            println!("    tag-0x21 STARK verification passed at consensus.");
        }
        Err(e) => {
            if !negctl {
                return Err(format!("combined live submit failed: {e}").into());
            }
            println!("  combined_settle_txid:  null (NODE REJECTED ✓)");
            println!("  node_error: {e}");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let cap_path = env::var("KCP_CSCI_CAPTURE")
        .map_err(|_| "KCP_CSCI_CAPTURE is required — path to csci-capture.json")?;
    let cap = load_capture(&cap_path)?;
    let key_file = env::var("KCP_KEY_FILE").map_err(|_| "KCP_KEY_FILE is required")?;
    // Wallet address prefix follows KCP_NET (testnet by default ⇒ identical to before).
    let (_, wallet_prefix, _, _) = net_from_env();
    let wallet = Wallet::load(Path::new(&key_file), 0, wallet_prefix)
        .map_err(|e| format!("load wallet: {e}"))?;
    let mode = env::var("KCP_MODE").unwrap_or_else(|_| "dryrun".to_string());

    // Sanity: the captured covenant commits OUR owner key (else our sig can't auth).
    let owner_xonly: [u8; 32] = wallet.keypair.x_only_public_key().0.serialize();
    if !cap.state0_script.windows(32).any(|w| w == owner_xonly) {
        return Err("captured state0_script does not embed the wallet owner pubkey".into());
    }

    if mode == "dryrun" {
        return dry_run(&cap, &wallet.keypair);
    }
    if mode == "combined" {
        let proof_dir =
            env::var("KCP_PROOF_DIR").map_err(|_| "KCP_PROOF_DIR is required for combined mode")?;
        return combined_dry_run(&cap, &wallet.keypair, Path::new(&proof_dir));
    }
    if mode == "item2" {
        let proof_dir =
            env::var("KCP_PROOF_DIR").map_err(|_| "KCP_PROOF_DIR is required for item2 mode")?;
        return item2_single_redeem(&cap, Path::new(&proof_dir));
    }
    if mode == "lock" {
        return lock_only(&cap, &wallet).await;
    }
    if mode == "settle-bound" {
        return settle_bound(&cap, &wallet).await;
    }
    if mode == "xbind-negctl" {
        return xbind_negctl(&cap, &wallet).await;
    }
    if mode == "combined-live" {
        return combined_live(&cap, &wallet, false).await;
    }
    if mode == "combined-negctl" {
        return combined_live(&cap, &wallet, true).await;
    }

    // Any mode reaching here MUST be an explicit live-broadcast mode. Fail closed
    // on a typo'd/unknown mode rather than silently broadcasting (audit M-2).
    if mode != "live" && mode != "negctl" {
        return Err(format!(
            "unknown KCP_MODE '{mode}'. Valid: dryrun (default, offline), combined, item2, \
             lock, settle-bound, xbind-negctl, combined-live, combined-negctl, live, negctl"
        )
        .into());
    }

    // ── live / negctl ────────────────────────────────────────────────────────
    let (node_cfg, prefix, guard, is_mainnet) = net_from_env();
    let node = NodeClient::new(node_cfg);
    let rpc = node.rpc().await?;
    let info = node.server_info().await?;
    println!(
        "connected: server={} network={} synced={}",
        info.server_version, info.network_id, info.is_synced
    );
    if !info.network_id.contains(guard) {
        return Err(format!("REFUSED: '{}' is not {guard}", info.network_id).into());
    }
    require_mainnet_confirm(is_mainnet)?;

    let spk0 = pay_to_script_hash_script(&cap.state0_script);
    let spk1 = pay_to_script_hash_script(&cap.state1_script);
    // negctl spends to P2SH(state0) again (seq NOT incremented) — covenant must reject.
    let out_spk = if mode == "negctl" {
        spk0.clone()
    } else {
        spk1.clone()
    };

    // GENESIS: fund P2SH(state0) with derived covenant_id.
    println!("\n--- GENESIS: fund covenant UTXO P2SH(state0) ---");
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
    let change = fund_amount - GENESIS_VALUE_SOMPI - CARRIER_FEE_SOMPI;

    let (gen_out, cov_id) = build_genesis_output(GENESIS_VALUE_SOMPI, funding_outpoint, &spk0);
    println!("covenant_id (engine, per-instance): {cov_id}");
    let mut gen_outputs = vec![gen_out];
    if change >= 12_000_000 {
        gen_outputs.push(TransactionOutput::new(
            change,
            pay_to_address_script(&wallet.address),
        ));
    }
    let gen_tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(funding_outpoint, vec![], 0, 0)],
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

    // SETTLE: spend covenant UTXO → out_spk.
    println!("\n--- SETTLE ({mode}): spend covenant UTXO seq 0→1 (owner-signed) ---");
    let p2sh0_addr = extract_script_pub_key_address(&spk0, prefix)
        .map_err(|e| format!("p2sh0 address: {e}"))?;
    let mut settle_txid = String::new();
    let mut node_err = String::new();
    for attempt in 1..=60u32 {
        let utxos = rpc
            .get_utxos_by_addresses(vec![p2sh0_addr.clone()])
            .await
            .map_err(|e| format!("get_utxos(p2sh0): {e}"))?;
        let Some(cov_utxo) = utxos
            .into_iter()
            .find(|e| e.outpoint.transaction_id == genesis_tid && e.outpoint.index == 0)
        else {
            if attempt < 60 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            return Err("genesis covenant UTXO not confirmed".into());
        };
        let amount = cov_utxo.utxo_entry.amount;
        let daa = cov_utxo.utxo_entry.block_daa_score;
        let genesis_outpoint = TransactionOutpoint::new(genesis_tid, 0);

        // Preflight. For negctl the engine itself rejects here (seq not advanced),
        // so we submit a deliberately-unpreflighted tx to capture the NODE error.
        let preflight = build_and_preflight_settle(
            &cap,
            &spk0,
            &out_spk,
            genesis_outpoint,
            amount,
            daa,
            cov_id,
            &wallet.keypair,
        );
        let tx = match (preflight, mode.as_str()) {
            (Ok((tx, used)), _) => {
                println!("settle preflight: ACCEPT, used_script_units={used}");
                tx
            }
            (Err(e), "negctl") => {
                // Expected: the silverscript covenant rejects the non-incremented
                // seq at the engine. Build the tx with a REALISTIC compute budget
                // (the same the valid settle needs, ~104_898 units → ~14 budget
                // units) so the node's rejection is the silverscript seq `require`
                // (VerifyError), NOT a mass-limit error. Submit so the LIVE NODE
                // does the rejecting on-chain.
                println!("negctl preflight REJECTED locally (expected): {e}");
                let mut out = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, out_spk.clone());
                out.covenant = Some(CovenantBinding {
                    authorizing_input: 0,
                    covenant_id: cov_id,
                });
                let input = TransactionInput::new(genesis_outpoint, vec![], 0, 0);
                let mut tx = Transaction::new(
                    TX_VERSION_TOCCATA,
                    vec![input],
                    vec![out],
                    0,
                    SUBNETWORK_ID_NATIVE,
                    0,
                    vec![],
                );
                let input_entry = UtxoEntry::new(amount, spk0.clone(), daa, false, Some(cov_id));
                // ~14 units covers the ~104_898 script units a valid settle uses;
                // the invalid one fails on the seq require well before exhausting it.
                tx.inputs[0].compute_commit = ComputeBudget(20).into();
                let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&input_entry), 0);
                let sig = schnorr_satisfier_sig(&sighash, &wallet.keypair);
                tx.inputs[0].signature_script = settle_sigscript(&cap, &sig, &cov_id.as_bytes());
                tx
            }
            (Err(e), _) => return Err(format!("settle preflight failed: {e}").into()),
        };

        match rpc.submit_transaction((&tx).into(), false).await {
            Ok(id) => {
                settle_txid = id.to_string();
                break;
            }
            Err(e) if is_transient(&e) && attempt < 60 && mode != "negctl" => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => {
                node_err = e.to_string();
                break;
            }
        }
    }

    println!("\n══ RESULT ({mode}) ════════════════════════════════════════════════");
    println!("  genesis_txid: {genesis_txid}");
    println!("  covenant_id:  {cov_id}");
    if !settle_txid.is_empty() {
        if mode == "negctl" {
            return Err(
                format!("SECURITY FAILURE: negctl settle ACCEPTED as {settle_txid}").into(),
            );
        }
        println!("  settle_txid:  {settle_txid}");
    } else {
        println!("  settle_txid:  null");
        println!("  node_error:   {node_err}");
    }
    Ok(())
}
