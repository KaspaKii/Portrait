//! `kcp-vault` — covenant-locked custody for the Kaspa Toccata covenant
//! engine: timelock (height / unix-seconds), k-of-n multisig, and composite
//! AND/OR spending conditions.
//!
//! ## v0 scope
//!
//! Deliberately narrow: timelock (DAA-height and unix-seconds variants),
//! k-of-n multisig, and composite [`SpendCondition::All`] /
//! [`SpendCondition::Any`] conditions with a maximum nesting depth of 8.
//! Oracle-gated and emission-schedule conditions are out of scope for v0.
//!
//! ## Evidence model (honest)
//!
//! **v0**: compiles real covenant scripts and anchors their digest on-chain in
//! a carrier transaction payload. The vault UTXO is key-controlled (pay-to-
//! address).
//!
//! **v1** (`onchain` module, feature `wrpc`): value is **locked under the
//! compiled script** via P2SH and **spent by satisfying that script** —
//! consensus-enforced, not just digest-anchored. Supported spend paths:
//! `MultiSig` (k-of-n), `TimelockHeight`/`TimelockUnixSeconds` (CLTV),
//! `Any(2)` (branch-selected via OP_IF/OP_ELSE), and `All(leaves)` (all-leaf
//! satisfier in reverse push order). All satisfiers are verified by the real
//! rusty-kaspa script engine offline before any submission.
//!
//! ## Status
//!
//! **v0 — unaudited — testnet first.** Do not use with mainnet value.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod condition;
pub mod error;
pub mod evaluator;
pub mod payload;

#[cfg(feature = "wrpc")]
pub mod script;

#[cfg(feature = "wrpc")]
pub mod tx;

#[cfg(feature = "wrpc")]
pub mod onchain;

/// Crate version (smoke-test surface for the skeleton CI).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn skeleton_smoke() {
        assert!(!crate::VERSION.is_empty());
    }
}
