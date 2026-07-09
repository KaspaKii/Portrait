//! Scaffold generators — one module per pattern.
//!
//! Each generator writes a ready-to-compile Cargo project to disk.
//! **Pre-production, unaudited.** Generated projects carry the same maturity
//! stamp as the library.

pub mod composite;
pub mod from_solidity_erc20;
pub mod from_solidity_ownable;
pub mod from_solidity_timelock;
pub mod from_solidity_vault;
pub mod governance;
pub mod ktt_token;
pub mod paired_attestation;
pub mod pq_anchor;
pub mod sealed_lineage;
pub mod timelock;
pub mod transferable_record;
pub mod vault;
pub mod vesting;
pub mod yield_vault;
