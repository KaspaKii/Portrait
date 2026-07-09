//! Derive the real covenant_id (KovId) for the tag-0x21 P2SH covenant.
//!
//! covenant_id = blake2b256(redeem_script) where
//!   redeem = <image_id> <control_id> <hashfn> <tag 0x21> OpZkPrecompile
//! i.e. EXACTLY the 32-byte value the P2SH lock (`OP_BLAKE2B <hash> OP_EQUAL`)
//! commits. This is the stable on-chain identity of the covenant program that
//! locks the settled UTXO — the same program for every UTXO of this instrument,
//! stable across respending. Feeding it to the vProg guest as `covenant_id`
//! binds the STARK proof to the very covenant it settles (KIP-20 cross-layer
//! binding; see crates/kcp-csci/src/binding.rs).
//!
//! Usage: KCP_PROOF_DIR=<dir with image[.hex] + control_id.hex> \
//!          cargo run -p portrait-settlement --bin covid --release
//! Prints: image_id, control_id, redeem hex, redeem len, covenant_id.
//!
//! Status: v0 — pre-production — unaudited — testnet-only.

use std::path::{Path, PathBuf};

use kcp_common::p2sh::redeem_script_hash;

type BoxError = Box<dyn std::error::Error>;

const ZK_TAG_R0_SUCCINCT: u8 = 0x21;
const HASHFN_POSEIDON2: u8 = 1;
const OP_ZK_PRECOMPILE: u8 = 0xa6;

fn read_hex(dir: &Path, name: &str) -> Result<Vec<u8>, BoxError> {
    let plain = dir.join(format!("{name}.hex"));
    let succinct = dir.join(format!("succinct.{name}.hex"));
    let path = if plain.exists() { plain } else { succinct };
    let s = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(hex::decode(s.trim())?)
}

fn push_data(s: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    match len {
        0 => s.push(0x00),
        1..=75 => {
            s.push(len as u8);
            s.extend_from_slice(data);
        }
        76..=255 => {
            s.push(0x4c);
            s.push(len as u8);
            s.extend_from_slice(data);
        }
        256..=65535 => {
            s.push(0x4d);
            s.push((len & 0xff) as u8);
            s.push((len >> 8) as u8);
            s.extend_from_slice(data);
        }
        _ => {
            s.push(0x4e);
            s.extend_from_slice(&(len as u32).to_le_bytes());
            s.extend_from_slice(data);
        }
    }
}

fn main() -> Result<(), BoxError> {
    let dir =
        PathBuf::from(std::env::var("KCP_PROOF_DIR").map_err(|_| "KCP_PROOF_DIR is required")?);
    let image_name = if dir.join("image.hex").exists() || dir.join("succinct.image.hex").exists() {
        "image"
    } else {
        "image_id"
    };
    let image_id = read_hex(&dir, image_name)?;
    let control_id = read_hex(&dir, "control_id")?;

    let mut redeem = Vec::new();
    push_data(&mut redeem, &image_id);
    push_data(&mut redeem, &control_id);
    push_data(&mut redeem, &[HASHFN_POSEIDON2]);
    push_data(&mut redeem, &[ZK_TAG_R0_SUCCINCT]);
    redeem.push(OP_ZK_PRECOMPILE);

    let covenant_id = redeem_script_hash(&redeem); // blake2b256(redeem), as P2SH commits

    eprintln!("image_id:    {}", hex::encode(&image_id));
    eprintln!("control_id:  {}", hex::encode(&control_id));
    eprintln!("redeem ({} bytes): {}", redeem.len(), hex::encode(&redeem));
    eprintln!(
        "covenant_id (blake2b256(redeem)): {}",
        hex::encode(covenant_id)
    );
    // stdout: just the covenant_id hex, for scripting.
    println!("{}", hex::encode(covenant_id));
    Ok(())
}
