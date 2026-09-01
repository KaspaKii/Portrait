//! `kcp-paired-attestation` — two-party mutual attestation for the Kaspa
//! Toccata covenant engine.
//!
//! Two counterparties each commit to a shared record under their own blinding
//! factor. "Mating" proves both committed to the same record (equality under
//! disclosed blinds), verified **off-chain**. The attestation sequence is then
//! anchored as an on-chain two-step lineage.
//!
//! ## Enforcement honesty
//!
//! - **v0 path** (`tx`, `invariants`): mating is verified **off-chain** by
//!   [`mate::verify_mate`], one wallet anchors both steps, and the on-chain
//!   transaction carries the mate event without consensus introspecting it.
//! - **v1 path** (`onchain` module, feature `wrpc`): the two-party datasig binding
//!   ships — both oracle signatures over the shared `msg_hash` are verified
//!   in-consensus by `OP_CHECKSIGFROMSTACK` at spend time `[KCP-PA-002]`.
//! - **The v1 covenant is not custody.** Its satisfier is not bound to the
//!   spending transaction (CSFS signs `msg_hash`, and the redeem script has no
//!   `OP_CHECKSIG` over the spend), so both signatures become public the moment
//!   a spend hits a mempool and any observer can re-spend the same outpoint to
//!   themselves. Use it for attestation signalling at dust value only. See the
//!   README threat model and `KNOWN-ISSUES.md`.
//!
//! ## Status
//!
//! **Unaudited — testnet first.** Do not use with mainnet value.

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
