//! `kcp-transferable-record` — transferable registry record for the Kaspa
//! Toccata covenant engine.
//!
//! A transferable record is a dedicated UTXO chain. The record is created by
//! locking value to the initial controller's address (a pay-to-address script
//! of an x-only pubkey — the "transfer gate v0"). Each transfer spends the
//! record UTXO with the current controller's Schnorr key and re-locks it to
//! the next controller's address, carrying a structured payload.
//!
//! ## v0 enforcement honesty
//!
//! Control is enforced **on-chain** by the key only: only the holder of the
//! current controller's private key can spend the record UTXO.
//!
//! Lineage continuity (sequence numbers, record identity, commitments) is
//! carried **in the payload** and verified **off-chain** by this library.
//! Consensus does not introspect the payload or reject malformed successors in
//! v0. Full introspection-enforced lineage — where the covenant script rejects
//! any successor that violates the lineage invariants at the consensus layer —
//! is the documented next step.
//!
//! ## Status
//!
//! **v0 — unaudited — testnet first.** Do not use with mainnet value.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod lineage;
pub mod payload;
pub mod record;

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
