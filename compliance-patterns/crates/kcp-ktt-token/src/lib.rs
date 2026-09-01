//! `kcp-ktt-token` — KCC20-shape-aligned regulated-token profile (KTT) for
//! the Kaspa Toccata covenant engine.
//!
//! ## What KTT is
//!
//! KTT is a regulated-token profile whose on-chain state mirrors the 4-field
//! KCC20 covenant-token shape:
//!
//! | Field | Type | Meaning |
//! |---|---|---|
//! | `owner_identifier` | `[u8; 32]` | Pubkey, script-hash, or covenant-id identifying the owner |
//! | `identifier_type` | [`IdentifierType`](crate::state::IdentifierType) | Discriminant (0x00 = pubkey, 0x01 = script-hash, 0x02 = covenant-id) |
//! | `amount` | `u64` | Token balance in the smallest representable unit |
//! | `is_minter` | `bool` | Whether this state-holder controls issuance |
//!
//! ## Terminology disambiguation
//!
//! **KCC20**, **KRC-20**, and **KTT** are three distinct things — see the
//! workspace README for the full explanation. KTT is *shape-aligned* with
//! KCC20 (it targets the same 4-field covenant state layout and the same
//! `validateOutputState` / `validateOutputStateWithTemplate` enforcement
//! primitives). It is not a port of the KRC-20 off-chain indexer protocol.
//!
//! ## v0 scope and enforcement honesty
//!
//! State transitions in v0 are validated **off-chain** and carrier-anchored
//! on testnet. The on-chain binding target is the KCC20
//! `validateOutputStateWithTemplate` enforcement primitive, which is
//! **engine-enforced and verified real** (kcc20_tests.rs 7/7 green against
//! the rusty-kaspa engine — see pattern-library FACTS SS-026). Authoring the
//! Kii covenant and a matching test harness on the v2.0.0 release engine is
//! the documented next step.
//!
//! Compliance-attestation hooks (TransferRule-style bitmask) are **modelled**
//! in [`transfer_rules`](crate::transfer_rules) but not enforced in v0.
//!
//! ## Status
//!
//! **v0 — unaudited — testnet first.** Do not use with mainnet value.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod payload;
pub mod state;
pub mod token;
pub mod transfer_rules;

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
