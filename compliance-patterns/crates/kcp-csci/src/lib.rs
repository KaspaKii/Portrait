//! Covenant-Settled Compliance Instrument (CSCI) — scaffold.
//!
//! Provides the off-chain data structures and journal encoding for the CSCI
//! pattern described in `docs/FLAGSHIP-DESIGN.md`.
//!
//! **Pre-production, unaudited, testnet-only.**
//! The CSCI covenant locking script (.sil) has not yet been authored.
//! This crate provides the journal encoding and state-encoding helpers only;
//! on-chain settlement requires the compiled covenant script (future work).

pub mod binding;
pub mod error;
pub mod redeem;
pub mod state;

pub use binding::{CovIdBinding, KovId};
pub use error::{CsciError, Result};
pub use redeem::{build_csci_redeem, csci_proof_fields};
pub use state::{CsciState, CsciStateTransition};
