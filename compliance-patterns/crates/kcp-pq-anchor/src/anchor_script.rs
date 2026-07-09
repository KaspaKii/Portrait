/// The KIP-16 proof fields for a tag-0x21 (RISC Zero) OpZkPrecompile script.
///
/// Build with known proof fields from your RISC Zero guest output, then call
/// [`build_pq_anchor_redeem`] to assemble the verifiable redeem script.
///
/// `hashfn` is always Poseidon2 (value 1) and is emitted internally as a 1-byte
/// data push (`0x01 0x01`) — the engine's `parse_hashfn` requires exactly a
/// 1-byte push, so a numeric `OP_1` (0x51) would be rejected.
pub struct PqAnchorScriptFields {
    pub claim: Vec<u8>,
    pub control_index: u32,
    /// Concatenated 32-byte Merkle path digests (variable length; must be a
    /// multiple of 32 bytes).
    pub control_digests: Vec<u8>,
    pub seal: Vec<u8>,
    /// SHA-256 of the raw journal bytes — caller must pre-compute.
    pub journal: [u8; 32],
    pub image_id: [u8; 32],
    pub control_id: [u8; 32],
    // hashfn always Poseidon2 = 1; emitted as a 1-byte data push internally.
}

/// Opcode byte for `OpZkPrecompile` in the rusty-kaspa v2.0.0 (Toccata) VM.
///
/// Defined in `crypto/txscript/src/opcodes/mod.rs` (tag `v2.0.0`, commit
/// `90dbf07`) as `opcode OpZkPrecompile<0xa6, 1>`. It pops the tag byte, then
/// the eight proof fields, runs the in-consensus RISC Zero verifier, and pushes
/// `true` on success.
const OP_ZK_PRECOMPILE: u8 = 0xa6;

/// RISC Zero succinct-receipt tag for `OpZkPrecompile` (`ZkTag::R0Succinct`).
const ZK_TAG_R0_SUCCINCT: u8 = 0x21;

/// Hash-function id pushed for `hashfn` — Poseidon2 (the only id the v2.0.0
/// tag-0x21 verifier accepts). Emitted as a 1-byte data push (`0x01 0x01`).
const HASHFN_POSEIDON2: u8 = 1;

/// Assemble the KIP-16 tag-0x21 redeem script for `fields`.
///
/// # Field order (must match the engine — verified by `tests/engine_accept.rs`)
///
/// The pinned engine (`rusty-kaspa` tag `v2.0.0`, commit `90dbf07`)
/// `R0SuccinctPrecompile::verify_zk` destructures the stack with `pop_raw::<8>()`
/// **after** `OpZkPrecompile` has popped the tag byte, yielding
/// `[claim, control_index, control_digests, seal, journal, image_id, control_id, hashfn]`
/// (first-pushed at index 0). The fields are therefore pushed in exactly that
/// order, then the tag byte, then `OpZkPrecompile` (`0xa6`):
///
/// ```text
/// <claim> <control_index> <control_digests> <seal> <journal> <image_id> <control_id> <hashfn> <0x21> OP_ZK_PRECOMPILE
/// ```
///
/// This exactly mirrors the engine's own accepting test vector
/// (`crypto/txscript/src/zk_precompiles/tests/helpers.rs::build_zk_script`) and
/// is checked by the offline engine-acceptance test in `tests/engine_accept.rs`,
/// which runs a real RISC Zero succinct proof through the consensus verifier
/// (valid accepts; a tampered journal / image id is rejected).
///
/// # Notes
///
/// * `hashfn` is emitted as a 1-byte data push of `1` (Poseidon2) — `0x01 0x01`.
///   The engine's `parse_hashfn` requires a 1-byte push; a numeric `OP_1`
///   (`0x51`) would be rejected.
/// * The script is invoked with `OpZkPrecompile` (`0xa6`), **not** `OP_0`.
///
/// # Invariants
///
/// * `fields.control_digests.len()` must be a multiple of 32.
pub fn build_pq_anchor_redeem(fields: &PqAnchorScriptFields) -> Result<Vec<u8>, PqAnchorError> {
    if !fields.control_digests.len().is_multiple_of(32) {
        return Err(PqAnchorError::ControlDigestsLenNotMultipleOf32);
    }
    let mut script: Vec<u8> = Vec::new();

    // Engine pop order (after the tag is popped by OpZkPrecompile):
    //   claim, control_index, control_digests, seal, journal, image_id,
    //   control_id, hashfn
    // Push in the same order so claim lands at the bottom of the eight-field
    // window and hashfn at the top.
    push_data(&mut script, &fields.claim);
    push_data(&mut script, &fields.control_index.to_le_bytes());
    push_data(&mut script, &fields.control_digests);
    push_data(&mut script, &fields.seal);
    push_data(&mut script, &fields.journal);
    push_data(&mut script, &fields.image_id);
    push_data(&mut script, &fields.control_id);
    // hashfn = Poseidon2 (1), as a 1-byte data push (NOT numeric OP_1).
    push_data(&mut script, &[HASHFN_POSEIDON2]);

    // Tag byte 0x21 identifies this as a RISC Zero succinct verification.
    push_data(&mut script, &[ZK_TAG_R0_SUCCINCT]);

    // OpZkPrecompile (0xa6) runs the in-consensus verifier and pushes true/false.
    script.push(OP_ZK_PRECOMPILE);

    Ok(script)
}

/// Push `data` as a minimal data push into `script`.
///
/// Uses standard Script push encoding:
/// - 0 bytes → OP_0 (0x00)
/// - 1..=75 bytes → length byte + data
/// - 76..=255 bytes → OP_PUSHDATA1 (0x4c) + length byte + data
/// - 256..=65535 bytes → OP_PUSHDATA2 (0x4d) + 2-byte LE length + data
fn push_data(script: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    match len {
        0 => script.push(0x00), // OP_0
        1..=75 => {
            script.push(len as u8);
            script.extend_from_slice(data);
        }
        76..=255 => {
            script.push(0x4c); // OP_PUSHDATA1
            script.push(len as u8);
            script.extend_from_slice(data);
        }
        256..=65535 => {
            script.push(0x4d); // OP_PUSHDATA2
            script.push((len & 0xff) as u8);
            script.push((len >> 8) as u8);
            script.extend_from_slice(data);
        }
        _ => {
            // OP_PUSHDATA4 — not expected for proof fields but handled for completeness
            script.push(0x4e); // OP_PUSHDATA4
            let le = (len as u32).to_le_bytes();
            script.extend_from_slice(&le);
            script.extend_from_slice(data);
        }
    }
}

/// Errors returned by `kcp-pq-anchor` functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PqAnchorError {
    ControlDigestsLenNotMultipleOf32,
}

impl std::fmt::Display for PqAnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ControlDigestsLenNotMultipleOf32 => {
                write!(f, "control_digests length must be a multiple of 32 bytes")
            }
        }
    }
}

impl std::error::Error for PqAnchorError {}
