//! Portrait settlement — the sound cross-layer binding, end to end.
//!
//! THESIS (the project's whole point): a Portrait-generated covenant accepts a
//! spend **only because** a valid vProg RISC Zero succinct STARK verified
//! in-consensus (KIP-16 tag 0x21, `OpZkPrecompile`) and bound the next state.
//! Tampering with the bound journal (the next-state commitment) or the vProg
//! image id makes the in-consensus verifier reject the spend.
//!
//! This binary demonstrates exactly that, OFFLINE, through the **real** pinned
//! engine (`rusty-kaspa` tag `v2.0.0`, commit `90dbf07`):
//!
//!   * It assembles the tag-0x21 redeem script with
//!     `kcp_pq_anchor::build_pq_anchor_redeem`.
//!   * It runs that script through `kaspa_txscript::TxScriptEngine` with
//!     `covenants_enabled = true` — the same `OpZkPrecompile` →
//!     `R0SuccinctPrecompile::verify_zk` → `risc0_zkp::verify::verify` path that
//!     consensus uses.
//!   * VALID proof  → ACCEPT (the bound transition settles).
//!   * TAMPERED journal / image id → REJECT (the negative control).
//!
//! No network, no wallet, no fabricated evidence: the STARK is verified by the
//! actual verifier, so a passing run is real cryptographic proof of the binding.
//! The identical assembly is what a live on-chain spend would lock against — see
//! the "GO LIVE" section printed at the end and `PROVENANCE.json`.
//!
//! ## Proof source
//!
//! By default the proof fields are read from the crate's offline acceptance
//! vectors (a real succinct receipt copied verbatim from the engine's own CI):
//!
//!   `crates/kcp-pq-anchor/tests/fixtures/succinct.*.hex`
//!
//! To run against a proof you generated yourself, point `KCP_PROOF_DIR` at a
//! directory holding `seal.hex, claim.hex, hashfn.hex, control_index.hex,
//! control_digests.hex, journal.hex, image.hex, control_id.hex` — e.g. the
//! `proof-export/succinct/` directory of `kii-csci-prover` after a non-DEV_MODE
//! run (NB: the prover host must also export `control_id` — see PROVENANCE.json).
//!
//! Status: v0 — pre-production — unaudited — testnet-only.

use std::path::{Path, PathBuf};

use kaspa_consensus_core::hashing::sighash::SigHashReusedValuesUnsync;
use kaspa_consensus_core::tx::PopulatedTransaction;
use kaspa_txscript::caches::Cache;
use kaspa_txscript::{EngineFlags, TxScriptEngine};

use kcp_pq_anchor::anchor_script::{build_pq_anchor_redeem, PqAnchorScriptFields};

type BoxError = Box<dyn std::error::Error>;

fn proof_dir() -> PathBuf {
    if let Ok(d) = std::env::var("KCP_PROOF_DIR") {
        return PathBuf::from(d);
    }
    // Default: the crate's offline acceptance vectors, relative to this example.
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .join("../../crates/kcp-pq-anchor/tests/fixtures")
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(manifest).join("../../crates/kcp-pq-anchor/tests/fixtures")
        })
}

/// Read `<dir>/<name>` (hex) → bytes. Accepts both `succinct.<f>.hex` (engine
/// vector naming) and `<f>.hex` (prover-export naming).
fn read_hex(dir: &Path, name: &str) -> Result<Vec<u8>, BoxError> {
    let plain = dir.join(format!("{name}.hex"));
    let succinct = dir.join(format!("succinct.{name}.hex"));
    let path = if plain.exists() { plain } else { succinct };
    let s = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(hex::decode(s.trim())?)
}

fn arr32(dir: &Path, name: &str) -> Result<[u8; 32], BoxError> {
    read_hex(dir, name)?
        .try_into()
        .map_err(|_| format!("{name} is not 32 bytes").into())
}

/// Read the on-chain `journal` field — the 32-byte sha256(journal_bytes).
///
/// The engine acceptance vectors put the 32-byte hash directly in `journal.hex`,
/// but the CSCI prover writes the raw 104-byte journal to `journal.hex` and the
/// hash to `journal_hash.hex`. Prefer a 32-byte `journal.hex`; otherwise fall
/// back to `journal_hash.hex` so a prover proof-export dir works unmodified.
fn journal32(dir: &Path) -> Result<[u8; 32], BoxError> {
    let j = read_hex(dir, "journal")?;
    if j.len() == 32 {
        return Ok(j.try_into().expect("checked len 32"));
    }
    read_hex(dir, "journal_hash")?
        .try_into()
        .map_err(|_| "neither journal.hex nor journal_hash.hex is 32 bytes".into())
}

fn load_fields(dir: &Path) -> Result<PqAnchorScriptFields, BoxError> {
    let image = if dir.join("image.hex").exists() || dir.join("succinct.image.hex").exists() {
        "image"
    } else {
        "image_id"
    };
    Ok(PqAnchorScriptFields {
        claim: read_hex(dir, "claim")?,
        control_index: u32::from_le_bytes(
            read_hex(dir, "control_index")?
                .try_into()
                .map_err(|_| "control_index is not 4 bytes")?,
        ),
        control_digests: read_hex(dir, "control_digests")?,
        seal: read_hex(dir, "seal")?,
        journal: journal32(dir)?,
        image_id: arr32(dir, image)?,
        control_id: arr32(dir, "control_id")?,
    })
}

/// Run the assembled redeem script through the real consensus VM (covenants on).
fn engine_verify(script: &[u8]) -> Result<(), String> {
    let sig_cache = Cache::new(0);
    let reused = SigHashReusedValuesUnsync::new();
    let mut vm = TxScriptEngine::<PopulatedTransaction, SigHashReusedValuesUnsync>::from_script(
        script,
        &reused,
        &sig_cache,
        EngineFlags {
            covenants_enabled: true,
            ..Default::default()
        },
    );
    vm.execute().map_err(|e| format!("{e:?}"))
}

fn main() -> Result<(), BoxError> {
    println!("=== Portrait Settlement — sound cross-layer binding (KIP-16 tag 0x21) ===");
    println!("    engine: rusty-kaspa v2.0.0 (90dbf07), covenants_enabled=true, OFFLINE\n");

    let dir = proof_dir();
    println!("[proof] dir: {}", dir.display());
    let fields = load_fields(&dir)?;
    println!("[proof] seal:            {} bytes", fields.seal.len());
    println!("[proof] claim:           {}", hex::encode(&fields.claim));
    println!(
        "[proof] journal (next-state commitment): {}",
        hex::encode(fields.journal)
    );
    println!(
        "[proof] image_id (vProg):                {}",
        hex::encode(fields.image_id)
    );
    println!(
        "[proof] control_id:      {}",
        hex::encode(fields.control_id)
    );
    println!();

    // ── 1. VALID: the bound transition settles ──────────────────────────────
    let script = build_pq_anchor_redeem(&fields)?;
    println!("[1/3] redeem script assembled: {} bytes", script.len());
    print!("      running real in-consensus verifier on the VALID proof ... ");
    engine_verify(&script).map_err(|e| format!("VALID proof unexpectedly REJECTED: {e}"))?;
    println!("ACCEPT ✓");
    println!("      → a covenant gated on this script WOULD allow the spend, because the");
    println!(
        "        STARK verified and bound journal={}",
        hex::encode(fields.journal)
    );
    println!();

    // ── 2. NEGATIVE CONTROL: tampered next-state commitment ─────────────────
    let mut tampered = fields_clone(&fields);
    tampered.journal[0] ^= 0x01;
    let script_bad = build_pq_anchor_redeem(&tampered)?;
    print!("[2/3] tampering the bound journal (next-state) and re-running verifier ... ");
    match engine_verify(&script_bad) {
        Ok(()) => return Err("SECURITY FAILURE: tampered journal was ACCEPTED".into()),
        Err(_) => println!("REJECT ✓"),
    }
    println!("      → the covenant would REJECT the spend: the proof does not bind this state.");
    println!();

    // ── 3. NEGATIVE CONTROL: wrong vProg image id ───────────────────────────
    let mut wrong_img = fields_clone(&fields);
    wrong_img.image_id[0] ^= 0x01;
    let script_img = build_pq_anchor_redeem(&wrong_img)?;
    print!("[3/3] swapping the vProg image id and re-running verifier ... ");
    match engine_verify(&script_img) {
        Ok(()) => return Err("SECURITY FAILURE: wrong image id was ACCEPTED".into()),
        Err(_) => println!("REJECT ✓"),
    }
    println!("      → only the pinned vProg can settle; a different program cannot.");
    println!();

    println!("══ RESULT ═══════════════════════════════════════════════════════════════");
    println!("✓ VALID proof ACCEPTED, tampered journal & wrong image id REJECTED — by the");
    println!("  real consensus verifier. The cross-layer binding is SOUND and proven offline.");
    println!();
    println!("══ GO LIVE (TN10 broadcast — not run here) ══════════════════════════════");
    println!("This binary proves acceptance against the real engine. A live on-chain");
    println!("settlement additionally needs a P2SH lock+spend carrying this redeem script.");
    println!("Required, in order:");
    println!("  1. Generate a REAL succinct proof for YOUR covenant/journal (NOT DEV_MODE):");
    println!("       cd kii-csci-prover");
    println!(
        "       cargo run --release            # ~5-15 min CPU, writes proof-export/succinct/"
    );
    println!("     NOTE: the prover host must ALSO export control_id (sr.control_id) — the");
    println!("     pinned engine's verify_zk pops 8 fields incl. control_id. See PROVENANCE.json.");
    println!("  2. Re-run THIS binary against that proof to confirm engine acceptance:");
    println!("       KCP_PROOF_DIR=kii-csci-prover/proof-export/succinct \\");
    println!(
        "         cargo run --manifest-path examples/portrait-settlement/Cargo.toml --release"
    );
    println!("  3. Build + broadcast the P2SH lock (funds → P2SH(redeem)) then the spend");
    println!("     (sigscript pushes claim,control_index,control_digests,seal,journal; redeem");
    println!("     pushes image_id,control_id,hashfn,tag + OpZkPrecompile — see the engine's");
    println!("     p2sh_scripts() split). Funded TN10 wallet + synced node required.");
    println!();
    println!("Pre-production · unaudited · testnet-only · MIT · Stichting Kii Foundation");
    Ok(())
}

fn fields_clone(f: &PqAnchorScriptFields) -> PqAnchorScriptFields {
    PqAnchorScriptFields {
        claim: f.claim.clone(),
        control_index: f.control_index,
        control_digests: f.control_digests.clone(),
        seal: f.seal.clone(),
        journal: f.journal,
        image_id: f.image_id,
        control_id: f.control_id,
    }
}
