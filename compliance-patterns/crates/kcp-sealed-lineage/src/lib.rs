//! `kcp-sealed-lineage` — append-only sealed evidence lineage for the Kaspa
//! Toccata covenant engine.
//!
//! A sealed evidence lineage is a **dedicated UTXO chain**. The genesis event
//! locks value to the publisher's address with a `seq = 0` payload; each
//! append spends the lineage UTXO and re-locks it to the same publisher
//! address, carrying a structured payload that seals a commitment to an
//! off-chain record.
//!
//! ## v0 enforcement honesty
//!
//! Key-controlled UTXO chain: only the holder of the publisher's private key
//! can spend the lineage UTXO. This is enforced on-chain by the pay-to-address
//! locking script.
//!
//! Lineage invariants (L-1 through L-4: sequence, identity, event-class rules,
//! and temporal envelope) are carried in the payload and validated **off-chain**
//! by this library. Consensus does not introspect the payload or reject
//! malformed successors in v0.
//!
//! The documented next step is expressing these invariants as a covenant
//! declaration on the upstream covenant-declaration system, so that consensus
//! rejects bad successors. This requires covenant introspection opcodes
//! (`validateOutputState` / KIP-20) available on Toccata.
//!
//! ## Status
//!
//! **v0 — unaudited — testnet first.** Do not use with mainnet value.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod invariants;
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
