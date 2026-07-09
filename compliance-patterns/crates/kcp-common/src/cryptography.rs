//! Cryptographic primitives for the Kaspa compliance library.
//!
//! **Pre-production, unaudited.** Pure offline; no engine dependency (except
//! [`sign_schnorr`], which requires the `wrpc` feature).
//!
//! ## EVM equivalents
//!
//! | This module | EVM equivalent |
//! |---|---|
//! | [`tagged_hash`] | `MessageHashUtils.sol` (tagged/domain-separated hashing) |
//! | [`script_digest`] | — (KCP-specific stable covenant-script identifier) |
//! | [`sign_schnorr`] (`wrpc`) | `ECDSA.sol` (signature production) |
//! | [`merkle_verify`] / [`merkle_hash_pair`] / [`merkle_leaf_hash`] | `MerkleProof.sol` (sorted-pair SHA-256 Merkle tree) |
//! | CSFS helper | `SignatureChecker.sol` shape — **DEFERRED** (`OP_CHECKSIGFROMSTACK` is used in `kcp-paired-attestation`; a standalone pure-Rust helper belongs here once that code is extracted) |
//! | PQ credential anchor | — **DEFERRED** (KIP-16 `OpZkPrecompile` tag-0x21 / RISC Zero succinct STARK is live on testnet-10; enables ML-DSA-44 / SLH-DSA anchors; no code in this crate yet — see `docs/NEXT-STEPS-pq-anchor.md`) |
//!
//! ## Post-quantum note
//!
//! All signing in this module uses secp256k1 Schnorr, which is not
//! post-quantum-safe. A PQ upgrade path exists via KIP-16 (live on TN10):
//! a RISC Zero guest verifies an ML-DSA-44 / SLH-DSA / Falcon-512 signature,
//! the succinct STARK is verified on-chain with `OpZkPrecompile 0xa6` tag `0x21`.
//! SilverScript has no ZK-verify builtin — this requires hand-rolled opcode
//! assembly. See `docs/NEXT-STEPS-pq-anchor.md` for the full design.
//!
//! ## `tagged_hash` vs `script_digest`
//!
//! These are distinct constructions and must not be conflated:
//! - [`tagged_hash`] follows the BIP-340 double-hashed-tag shape:
//!   `SHA256(SHA256(tag) || SHA256(tag) || data)`. Use for Schnorr-adjacent
//!   domain separation.
//! - [`script_digest`] is `SHA256(KCP_SCRIPT_DIGEST_v1 || script_bytes)` — a
//!   single-pass construction used as a stable covenant-script identifier in
//!   KIP-14 payloads. The two constructions produce different outputs for the
//!   same input.

use sha2::{Digest, Sha256};

/// Re-export of [`crate::digest::script_digest`] for callers who import all
/// cryptographic helpers from one place. The canonical implementation lives in
/// [`crate::digest`]; this re-export is a convenience alias.
pub use crate::digest::script_digest;

/// Compute a BIP-340 tagged hash: `SHA256(SHA256(tag) || SHA256(tag) || data)`.
///
/// The double-hashed tag prefix domain-separates the output from plain
/// SHA-256 and from hashes under different tags. Use this when constructing
/// Schnorr-adjacent message commitments for application-layer data.
///
/// **Not** the same construction as [`script_digest`] — see module doc for the
/// distinction.
///
/// # Caller contract
///
/// - `data` may be arbitrary bytes (raw or pre-hashed — caller's choice).
/// - The return value is a 32-byte digest suitable for passing to
///   `kaspa_bip32::secp256k1::Message::from_digest_slice` and then to
///   [`sign_schnorr`].
/// - **Do not** use this to construct the Kaspa P2SH transaction sighash.
///   For that, use [`crate::p2sh::p2sh_input_sighash`] (which calls
///   `kaspa_consensus_core`'s `calc_schnorr_signature_hash` internally).
/// - Supply a non-empty, application-specific `tag`. An empty tag is accepted
///   but defeats domain separation from any other caller also using an empty tag.
pub fn tagged_hash(tag: &[u8], data: &[u8]) -> [u8; 32] {
    debug_assert!(
        !tag.is_empty(),
        "tagged_hash: empty tag defeats domain separation"
    );
    let tag_hash: [u8; 32] = Sha256::digest(tag).into();
    let mut h = Sha256::new();
    h.update(tag_hash);
    h.update(tag_hash);
    h.update(data);
    h.finalize().into()
}

/// Produce a 65-byte Schnorr satisfier element: 64-byte signature over
/// `sighash` with the Schnorr private key, followed by the `SIG_HASH_ALL`
/// byte (`0x01`).
///
/// EVM equivalent: `ECDSA.sol` signature production — pre-production, unaudited.
///
/// This is the pure signing step; it does not depend on any transaction
/// structure. Use [`crate::p2sh::schnorr_satisfier_sig`] for the
/// spend-path helper that returns a `Vec<u8>` for direct use in a P2SH
/// signature script.
///
/// Requires the `wrpc` feature (pulls `kaspa_bip32::secp256k1`).
///
/// To use the output in a P2SH signature script, convert to `Vec<u8>` with
/// `.to_vec()` and pass to [`crate::p2sh::build_p2sh_signature_script`]. For
/// the spend-path helper that returns `Vec<u8>` directly, use
/// [`crate::p2sh::schnorr_satisfier_sig`] (which delegates here).
///
/// Note: a dedicated unit test for `sign_schnorr` is deferred — the identical
/// signing path is exercised by the `p2sh` engine round-trip tests. See
/// `KNOWN-ISSUES.md` for the explicit gap record.
#[cfg(feature = "wrpc")]
pub fn sign_schnorr(sighash: &[u8; 32], keypair: &kaspa_bip32::secp256k1::Keypair) -> [u8; 65] {
    use kaspa_bip32::secp256k1::Message;
    const SIG_HASH_ALL_BYTE: u8 = 0x01;
    let msg = Message::from_digest_slice(sighash).expect("32-byte digest");
    let sig: [u8; 64] = *keypair.sign_schnorr(msg).as_ref();
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&sig);
    out[64] = SIG_HASH_ALL_BYTE;
    out
}

// ── MerkleProof ───────────────────────────────────────────────────────────────

/// Compute `sha256(min(a, b) ‖ max(a, b))` — the hash of a sorted Merkle
/// node pair. Sorting ensures the proof is order-independent (the sibling
/// can appear on either side of the tree without changing the proof elements).
///
/// EVM equivalent: `MerkleProof._hashPair` (Solidity pattern-library v5 shape).
pub fn merkle_hash_pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    if a <= b {
        h.update(a);
        h.update(b);
    } else {
        h.update(b);
        h.update(a);
    }
    h.finalize().into()
}

/// Compute the Merkle leaf hash: `sha256(data)`.
///
/// This is a convenience helper for callers who need to convert raw data to a
/// leaf hash before passing it to [`merkle_verify`]. The standard EVM pattern-library tree
/// double-hashes leaves (`keccak256(keccak256(data))`); the Kaspa equivalent
/// uses `sha256(sha256(data))` to obtain a leaf. For simpler use cases, a
/// single `sha256(data)` leaf hash is also acceptable — **callers must be
/// consistent in how leaves are hashed across all parties**.
pub fn merkle_leaf_hash(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Verify a Merkle inclusion proof.
///
/// EVM equivalent: `MerkleProof.verify` (Solidity pattern-library v5 shape).
///
/// `leaf` must already be hashed (use [`merkle_leaf_hash`] to produce a leaf
/// from raw data). `proof` is the ordered list of sibling hashes from leaf to
/// root. Returns `true` iff the computed root equals `root`.
///
/// An empty `proof` is valid iff `leaf == root` (a single-element tree).
pub fn merkle_verify(leaf: &[u8; 32], proof: &[[u8; 32]], root: &[u8; 32]) -> bool {
    let mut computed = *leaf;
    for sibling in proof {
        computed = merkle_hash_pair(&computed, sibling);
    }
    &computed == root
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_hash_is_deterministic() {
        assert_eq!(
            tagged_hash(b"kcp/test", b"data"),
            tagged_hash(b"kcp/test", b"data")
        );
    }

    #[test]
    fn tagged_hash_domain_separates() {
        assert_ne!(
            tagged_hash(b"kcp/tag1", b"data"),
            tagged_hash(b"kcp/tag2", b"data")
        );
    }

    #[test]
    fn tagged_hash_differs_from_raw_sha256() {
        let raw: [u8; 32] = Sha256::digest(b"data").into();
        assert_ne!(tagged_hash(b"kcp/tag", b"data"), raw);
    }

    #[test]
    fn script_digest_reexport_accessible() {
        let d: [u8; 32] = script_digest(b"OP_TRUE");
        assert_ne!(d, [0u8; 32]);
    }
}

#[cfg(test)]
mod merkle_tests {
    use super::*;

    fn leaf(seed: u8) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = seed;
        b
    }

    #[test]
    fn merkle_single_element_no_proof() {
        let l = leaf(1);
        assert!(merkle_verify(&l, &[], &l));
    }

    #[test]
    fn merkle_single_element_wrong_root() {
        let l = leaf(1);
        let wrong = leaf(2);
        assert!(!merkle_verify(&l, &[], &wrong));
    }

    #[test]
    fn merkle_two_leaf_tree() {
        // Tree: root = hash_pair(leaf_a, leaf_b)
        // Proof for leaf_a = [leaf_b]; proof for leaf_b = [leaf_a]
        let la = leaf(1);
        let lb = leaf(2);
        let root = merkle_hash_pair(&la, &lb);

        assert!(merkle_verify(&la, &[lb], &root));
        assert!(merkle_verify(&lb, &[la], &root));
    }

    #[test]
    fn merkle_four_leaf_tree() {
        // Balanced 4-leaf tree:
        //         root
        //        /    \
        //      n01    n23
        //      / \    / \
        //     l0  l1 l2  l3
        let l0 = leaf(0);
        let l1 = leaf(1);
        let l2 = leaf(2);
        let l3 = leaf(3);
        let n01 = merkle_hash_pair(&l0, &l1);
        let n23 = merkle_hash_pair(&l2, &l3);
        let root = merkle_hash_pair(&n01, &n23);

        // Proof for l0: [l1, n23]
        assert!(merkle_verify(&l0, &[l1, n23], &root));
        // Proof for l3: [l2, n01]
        assert!(merkle_verify(&l3, &[l2, n01], &root));
    }

    #[test]
    fn merkle_invalid_proof_rejected() {
        let la = leaf(1);
        let lb = leaf(2);
        let root = merkle_hash_pair(&la, &lb);
        // Wrong sibling
        let wrong_sibling = leaf(99);
        assert!(!merkle_verify(&la, &[wrong_sibling], &root));
    }

    #[test]
    fn merkle_hash_pair_is_order_independent() {
        let a = leaf(10);
        let b = leaf(20);
        assert_eq!(merkle_hash_pair(&a, &b), merkle_hash_pair(&b, &a));
    }

    #[test]
    fn merkle_leaf_hash_is_sha256() {
        use sha2::{Digest, Sha256};
        let data = b"hello kaspa";
        let expected: [u8; 32] = Sha256::digest(data).into();
        assert_eq!(merkle_leaf_hash(data), expected);
    }
}
