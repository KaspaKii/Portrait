//! hello-vault — the shortest path to "the real Kaspa script engine accepted
//! my covenant spend."
//!
//! Locks a synthetic UTXO under a **2-of-2 multisig P2SH covenant script**,
//! then spends it back by satisfying the script. Runs **entirely offline** —
//! no live node, no funds, no network — but uses the **real `rusty-kaspa`
//! script engine** via `kcp_common::p2sh::verify_p2sh_spend_offline`.
//!
//! This main is adapted directly from the kcp-vault unit test
//! `multisig_2of2_lock_spend_executes_on_engine`
//! (`crates/kcp-vault/src/onchain.rs`) — same engine path, same assertion,
//! exposed as a forkable example.
//!
//! Status: v0 — unaudited — testnet first.

use kaspa_bip32::secp256k1::{Keypair, SECP256K1};
use kaspa_consensus_core::{
    mass::SigopCount,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        ComputeCommit, ScriptPublicKey, ScriptVec, Transaction, TransactionId,
        TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
    },
};
use kcp_common::{
    p2sh::{
        build_p2sh_signature_script, p2sh_input_sighash, p2sh_lock_script,
        p2sh_redeem_sigop_count, schnorr_satisfier_sig, verify_p2sh_spend_offline,
    },
    tx::CARRIER_FEE_SOMPI,
};
use kcp_vault::{condition::SpendCondition, script::compile_condition};

/// Value locked under the multisig P2SH redeem script (2 KAS = 200_000_000 sompi).
const LOCK_VALUE_SOMPI: u64 = 200_000_000;

/// Build a deterministic test keypair from a single repeated byte.
/// **NOT a secret.** These keys never touch funds; the synthetic UTXO this
/// example builds does not exist on any network.
fn test_keypair(byte: u8) -> Keypair {
    Keypair::from_seckey_slice(SECP256K1, &[byte; 32])
        .expect("32 repeated bytes is a valid secp256k1 secret key")
}

/// A minimal destination `ScriptPublicKey` for the spend output (`OP_TRUE`).
fn op_true_spk() -> ScriptPublicKey {
    ScriptPublicKey::new(0, ScriptVec::from_slice(&[0x51]))
}

/// Build a synthetic P2SH spend transaction + its corresponding UTXO entry.
/// The transaction is well-formed but never broadcast; the outpoint
/// references a UTXO that does not exist on any network.
fn build_spend_tx(redeem: &[u8], amount: u64) -> (Transaction, UtxoEntry) {
    let p2sh_spk = p2sh_lock_script(redeem);
    let prev = TransactionOutpoint::new(TransactionId::from_slice(&[0xabu8; 32]), 0);
    let input = TransactionInput::new(prev, vec![], 0, 0);
    let output = TransactionOutput::new(amount - CARRIER_FEE_SOMPI, op_true_spk());
    let mut tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let sigops = p2sh_redeem_sigop_count(redeem);
    let commit: ComputeCommit = SigopCount(sigops).into();
    for inp in tx.inputs.iter_mut() {
        inp.compute_commit = commit;
    }
    let entry = UtxoEntry::new(amount, p2sh_spk, 0, false, None);
    (tx, entry)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("hello-vault — offline P2SH multisig covenant demo");
    println!("(no live node, no funds, no network — real rusty-kaspa engine)\n");

    // [1/5] Build a 2-of-2 multisig spending condition
    let kp1 = test_keypair(0x11);
    let kp2 = test_keypair(0x22);
    let pk1 = kp1.x_only_public_key().0.serialize();
    let pk2 = kp2.x_only_public_key().0.serialize();
    let condition = SpendCondition::MultiSig {
        threshold: 2,
        xonly_keys: vec![pk1, pk2],
    };
    println!("[1/5] built 2-of-2 multisig spending condition");

    // [2/5] Compile to a real Kaspa redeem script
    let redeem = compile_condition(&condition)?;
    println!("[2/5] compiled to {}-byte redeem script", redeem.len());

    // [3/5] Build synthetic spend tx + P2SH-locked UTXO
    let (mut tx, entry) = build_spend_tx(&redeem, LOCK_VALUE_SOMPI);
    println!(
        "[3/5] built synthetic spend tx + P2SH-locked UTXO ({} sompi)",
        LOCK_VALUE_SOMPI
    );

    // [4/5] Sign with both keys + build the P2SH satisfier
    let sighash = p2sh_input_sighash(&tx, std::slice::from_ref(&entry), 0);
    let sig1 = schnorr_satisfier_sig(&sighash, &kp1);
    let sig2 = schnorr_satisfier_sig(&sighash, &kp2);
    tx.inputs[0].signature_script = build_p2sh_signature_script(&[sig1, sig2], &redeem)?;
    println!("[4/5] signed with both keys + built P2SH satisfier");

    // [5/5] Run the spend through the real rusty-kaspa script engine
    match verify_p2sh_spend_offline(&tx, 0, &entry, false) {
        Ok(()) => {
            println!(
                "[5/5] ✓ PASSED — real rusty-kaspa script engine accepted the spend\n"
            );
            println!(
                "You just ran the same engine path that produced [KCP-VT-002]\n\
                 on testnet-10. See crates/kcp-vault/README.md for variants:\n\
                   - MultiSig (k-of-n)\n\
                   - TimelockHeight / TimelockUnixSeconds (CLTV)\n\
                   - Composite Any/All (branch-selected P2SH)\n"
            );
            Ok(())
        }
        Err(e) => Err(format!("engine REJECTED the spend: {e}").into()),
    }
}
