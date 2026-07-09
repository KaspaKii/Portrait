//! `kcp-common` — shared plumbing for the kaspa-compliance-patterns crates:
//! deterministic canonicalisation and hashing, script digests, wallet key
//! derivation, a wRPC node client, and carrier-transaction submission.
//!
//! Provenance: portions of this crate are derived from the Kii Kastract
//! codebase (same author), relicensed MIT for this library under the IP
//! grant recorded 2026-06-11. The transaction path here is the one that has
//! produced real testnet transactions in the donor.
//!
//! Feature `wrpc` enables the node-facing modules ([`wallet`], [`wrpc`],
//! [`tx`]); it pulls the rusty-kaspa dependency tree (git, branch
//! `toccata`). Without it the crate is pure and offline.
//!
//! Status: **v0 — unaudited — testnet first.**

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod access;
pub mod canonical;
pub mod cryptography;
pub mod digest;
pub mod error;
pub mod security;
pub mod utils;

#[cfg(feature = "wrpc")]
pub mod p2sh;
#[cfg(feature = "wrpc")]
pub mod tx;
#[cfg(feature = "wrpc")]
pub mod wallet;
#[cfg(feature = "wrpc")]
pub mod wrpc;

/// Crate version (smoke-test surface for the skeleton CI).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn skeleton_smoke() {
        assert!(!crate::VERSION.is_empty());
    }
}
