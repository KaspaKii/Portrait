//! `kcp-vesting` — linear DAA-height vesting schedule.
//!
//! EVM equivalent: `VestingWallet` (Solidity pattern-library v5 shape).
//!
//! Uses Kaspa DAA heights as the on-chain clock. Unlike the EVM pattern's unix-timestamp
//! vesting, this crate uses the Kaspa Blue Score (DAA score) — approximately
//! one unit per second at the 1 BPS target rate, but governed by the DAA.
//!
//! **Pre-production, unaudited, testnet-only.**

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod schedule;
