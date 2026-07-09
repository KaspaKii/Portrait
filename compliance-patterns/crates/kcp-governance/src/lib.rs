//! `kcp-governance` — DAG-native governance primitives for the
//! kaspa-compliance-patterns library.
//!
//! Provides the `Governor`-equivalent for Kaspa's DAG model.
//! Because Kaspa's DAG does not have globally-sequential block numbers or
//! on-chain token-weighted voting, this crate uses **DAA heights as the clock**
//! and **k-of-n Schnorr multisig as the voting mechanism**.
//!
//! ## Core types
//!
//! - [`proposal::GovernanceProposal`] — a governance proposal with a DAA-height
//!   voting window and SHA-256 content id.
//! - [`vote::MultiSigVote`] — k-of-n approval tracker (signatories + threshold).
//! - [`action::TimelockAction`] — post-pass execution delay (minimum DAA heights).
//! - [`governor::GovernorState`] — combines the above into a complete governance
//!   session with lifecycle transitions.
//!
//! ## Design decisions
//!
//! **No token-weighted voting.** Kaspa has no native on-chain governance token
//! equivalent to ERC20 votes. `MultiSigVote` uses a fixed signatory set; extend
//! with a token-weighted layer once a KRC20-equivalent with snapshotted balances
//! exists on Kaspa mainnet.
//!
//! **Pure value types.** Every type is a plain Rust struct with no UTXO or
//! covenant binding. Callers are responsible for anchoring governor state to a
//! covenant (e.g., using `kcp-sealed-lineage` for append-only state continuity).
//!
//! **DAA-height approximation.** The DAG does not guarantee strict serialisation
//! of heights across concurrent blocks. Use DAA heights as *approximate* clocks;
//! exact height equality is not reliable for time-critical applications.
//!
//! **Pre-production, unaudited, testnet-only.** This crate is v0 — not audited,
//! not production-ready. See `KNOWN-ISSUES.md`.
//!
//! ## EVM equivalence map
//!
//! | EVM type | Kii equivalent |
//! |---|---|
//! | `Governor` | [`governor::GovernorState`] |
//! | `GovernorVotes` | [`vote::MultiSigVote`] (k-of-n, not token-weighted) |
//! | `TimelockController` | [`action::TimelockAction`] |
//! | `IVotes` / `Votes` | Deferred — no KRC20 snapshots available yet |

#![forbid(unsafe_code)]

pub mod action;
pub mod error;
pub mod governor;
pub mod proposal;
pub mod vote;
