//! Compute budget for a tag-0x21 spend.
//!
//! A version-0 input commits its covenant-execution budget as a `SigopCount`,
//! and that field is a `u8`. **255 is therefore the maximum expressible
//! budget** — there is no larger one — which caps a tag-0x21 spend at
//! [`MAX_COMMITTABLE_SCRIPT_UNITS`] script units. RISC Zero succinct
//! verification sits close to that ceiling — the shipped reference proof
//! measures 25,446,182 units, **0.25% under it** — so the margin must be
//! **measured** for the proof shape you actually intend to fund, **before** any
//! value is locked: the proof fields are inside the redeem script, so they are
//! inside the P2SH address, and a script whose execution cannot be budgeted has
//! no other spending path. The funds are then permanently unrecoverable.
//!
//! # Measuring requires the `wrpc` feature
//!
//! [`measure_pq_anchor_units`] and [`fits_pq_verify_budget`] run the real
//! consensus VM, so they live behind the `wrpc` feature — the same posture as
//! every other engine-touching module in this workspace, keeping the default
//! build free of the `rusty-kaspa` git tree. The constants above are always
//! available. **Do not fund a tag-0x21 address without measuring:**
//!
//! ```sh
//! cargo test -p kcp-pq-anchor --features wrpc --test budget_ceiling -- --nocapture
//! ```

#[cfg(feature = "wrpc")]
use kaspa_consensus_core::{
    hashing::sighash::SigHashReusedValuesUnsync,
    mass::SigopCount,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{
        ComputeCommit, PopulatedTransaction, Transaction, TransactionId, TransactionInput,
        TransactionOutpoint, TransactionOutput, UtxoEntry,
    },
};
#[cfg(feature = "wrpc")]
use kaspa_txscript::{
    caches::Cache, pay_to_script_hash_script, script_builder::ScriptBuilder, EngineCtx,
    EngineFlags, TxScriptEngine,
};

/// Script units one `SigopCount` unit commits.
const SCRIPT_UNITS_PER_SIGOP: u64 = 100_000;

/// Script units every input gets for free, before any commitment.
const FREE_SCRIPT_UNITS: u64 = 9_999;

/// The largest covenant-execution budget a version-0 input can commit:
/// `255 × 100_000 + 9_999` = 25,509,999 script units.
///
/// `SigopCount` is a `u8`, so this is a hard ceiling, not a default. A tag-0x21
/// spend that consumes more than this can never be budgeted, and the value
/// under that P2SH is unspendable.
pub const MAX_COMMITTABLE_SCRIPT_UNITS: u64 =
    (u8::MAX as u64) * SCRIPT_UNITS_PER_SIGOP + FREE_SCRIPT_UNITS;

/// Returns the sigOpCount required for a KIP-16 tag-0x21 spend.
///
/// This is [`u8::MAX`] — the **maximum expressible** budget, not a measurement.
/// There is no larger budget to fall back to, and the reference proof already
/// uses 99.75% of it. Check that your proof shape fits with
/// `measure_pq_anchor_units` (feature `wrpc`) before funding the address.
pub const fn sigop_count_for_pq_verify() -> u8 {
    u8::MAX
}

/// Errors from measuring a redeem script's execution cost.
#[cfg(feature = "wrpc")]
#[derive(Debug, thiserror::Error)]
pub enum BudgetError {
    /// The signature script could not be assembled (redeem push rejected).
    #[error("signature script assembly failed: {0}")]
    ScriptAssembly(String),
    /// The engine rejected the script, so no cost could be measured.
    #[error("engine rejected the spend, no measurement taken: {0}")]
    EngineRejected(String),
}

/// Measure the script units a tag-0x21 P2SH spend of `redeem_script` consumes,
/// by running it through the pinned consensus VM as a **real transaction
/// input** (P2SH-wrapped, with the input's compute commitment set), not as a
/// standalone script.
///
/// Compare the result against [`MAX_COMMITTABLE_SCRIPT_UNITS`] before funding
/// the address — see [`fits_pq_verify_budget`]. Requires the `wrpc` feature.
///
/// # Errors
///
/// [`BudgetError::EngineRejected`] if the engine does not accept the spend (an
/// invalid proof has no meaningful cost).
#[cfg(feature = "wrpc")]
pub fn measure_pq_anchor_units(redeem_script: &[u8]) -> Result<u64, BudgetError> {
    let flags = EngineFlags {
        covenants_enabled: true,
        ..Default::default()
    };

    let mut builder = ScriptBuilder::with_flags(flags);
    builder
        .add_data(redeem_script)
        .map_err(|e| BudgetError::ScriptAssembly(format!("redeem push: {e}")))?;
    let signature_script = builder.drain().to_vec();

    let spk = pay_to_script_hash_script(redeem_script);
    const VALUE_SOMPI: u64 = 100_000_000;

    let outpoint = TransactionOutpoint::new(TransactionId::from_bytes([0x11; 32]), 0);
    let mut input = TransactionInput::new(outpoint, signature_script, 0, 0);
    input.compute_commit = ComputeCommit::from(SigopCount(sigop_count_for_pq_verify()));
    let output = TransactionOutput::new(VALUE_SOMPI / 2, spk.clone());
    let tx = Transaction::new(
        0,
        vec![input],
        vec![output],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    let entry = UtxoEntry::new(VALUE_SOMPI, spk, 0, false, None);

    let populated = PopulatedTransaction::new(&tx, vec![entry.clone()]);
    let sig_cache = Cache::new(0);
    let reused = SigHashReusedValuesUnsync::new();
    let mut vm = TxScriptEngine::from_transaction_input(
        &populated,
        &tx.inputs[0],
        0,
        &entry,
        EngineCtx::new(&sig_cache).with_reused(&reused),
        flags,
    );
    vm.execute()
        .map_err(|e| BudgetError::EngineRejected(format!("{e:?}")))?;
    Ok(vm.used_script_units().0)
}

/// Whether `used_script_units` can be committed by a version-0 input at all.
///
/// `false` means the spend is **unbudgetable**: do not fund the address.
#[cfg(feature = "wrpc")]
pub fn fits_pq_verify_budget(used_script_units: u64) -> bool {
    used_script_units <= MAX_COMMITTABLE_SCRIPT_UNITS
}
