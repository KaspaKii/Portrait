//! `kcp-paired-attestation` — two-party mutual attestation for the Kaspa
//! Toccata covenant engine.
//!
//! Two counterparties each commit to a shared record under their own blinding
//! factor. "Mating" proves both committed to the same record (equality under
//! disclosed blinds), verified **off-chain**. The attestation sequence is then
//! anchored as an on-chain two-step lineage.
//!
//! ## v0 enforcement honesty
//!
//! - Mating is verified **off-chain** by [`mate::verify_mate`]. The on-chain
//!   transaction carries the mate event but consensus does not introspect it.
//! - The full two-party on-chain datasig binding is **proven viable**:
//!   FACTS SS-024-v4 confirms `OpCheckSigFromStack` (`checkDataSig`)
//!   binds key+message correctly on kaspad v2.0.0. The next step is P2SH
//!   spend-path plumbing in `kcp-common`, which is not in v0 scope.
//! - In v0, one wallet anchors both steps. Two-party on-chain signing is the
//!   documented next step.
//!
//! ## Status
//!
//! **v0 — unaudited — testnet first.** Do not use with mainnet value.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod invariants;
pub mod mate;
pub mod payload;
pub mod record;

#[cfg(feature = "wrpc")]
pub mod onchain;

#[cfg(feature = "wrpc")]
pub mod tx;

/// Crate version (smoke-test surface for the skeleton CI).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn skeleton_smoke() {
        assert!(!crate::VERSION.is_empty());
    }
}
