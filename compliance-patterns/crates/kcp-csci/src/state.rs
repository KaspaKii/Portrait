//! CSCI state encoding and transition logic.
//!
//! The CSCI state is a superset of `KttState` (42 bytes) with a `seq` counter
//! appended (8 bytes LE), for a total of 50 bytes. See FLAGSHIP-DESIGN.md §3.3.

use kcp_ktt_token::state::{IdentifierType, KttState};
use kcp_pq_anchor::journal_spec::JournalSpec;
use sha2::{Digest, Sha256};

use crate::error::{CsciError, Result};

/// 50-byte CSCI state encoding: KttState (42 B) + seq (8 B LE).
pub const CSCI_STATE_LEN: usize = 50;

/// CSCI instrument state — a KTT state extended with a monotonic sequence counter.
///
/// `seq` starts at 0 at genesis and increments by 1 on every compliant transfer.
/// The covenant enforces `seq = prev_seq + 1` on each spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsciState {
    pub ktt: KttState,
    /// Monotonic transfer counter. 0 at genesis.
    pub seq: u64,
    /// sha256 of the canonical rule set bytes, committed at genesis.
    pub rule_hash: [u8; 32],
    /// KIP-20 covenant ID — identifies this CSCI instance.
    pub covenant_id: [u8; 32],
}

impl CsciState {
    /// Create the genesis state (seq = 0).
    pub fn new_genesis(
        owner: [u8; 32],
        amount: u64,
        rule_hash: [u8; 32],
        covenant_id: [u8; 32],
    ) -> Self {
        Self {
            ktt: KttState {
                identifier_type: IdentifierType::Pubkey,
                owner_identifier: owner,
                amount,
                is_minter: false,
            },
            seq: 0,
            rule_hash,
            covenant_id,
        }
    }

    /// Encode the CSCI state to 50 bytes:
    /// `KttState::encode()` (42 B) || `seq` (8 B LE).
    pub fn encode(&self) -> [u8; CSCI_STATE_LEN] {
        let mut out = [0u8; CSCI_STATE_LEN];
        let ktt_bytes = self.ktt.encode();
        out[..42].copy_from_slice(&ktt_bytes);
        out[42..50].copy_from_slice(&self.seq.to_le_bytes());
        out
    }

    /// `sha256(encode())` — the `new_state_hash` committed in the vProg journal.
    pub fn state_hash(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }

    /// Build the `JournalSpec::CsciTransition` for this state as the *target*
    /// of a transfer — i.e. the journal the vProg would commit to when this
    /// state is the result of the transition.
    pub fn as_journal_spec(&self) -> JournalSpec {
        JournalSpec::CsciTransition {
            covenant_id: self.covenant_id,
            new_state_hash: self.state_hash(),
            rule_hash: self.rule_hash,
            seq: self.seq,
        }
    }
}

/// The result of a compliant CSCI transfer.
///
/// The off-chain compliance engine (vProg) accepts a `CsciStateTransition`
/// as input, verifies the rule predicate, and commits `new_state.as_journal_spec()`
/// as its journal. The builder then uses `journal_hash()` as the `journal` argument
/// in `build_pq_anchor_redeem` and pushes `journal_bytes()` as a clear-text data
/// push in the unlocking script.
#[derive(Debug, Clone)]
pub struct CsciStateTransition {
    pub prev: CsciState,
    pub new_state: CsciState,
}

impl CsciStateTransition {
    /// Compute and validate a transfer:
    /// - `amount` sompi to `new_owner` (32-byte x-only Schnorr key)
    /// - `rule_hash` must match `prev.rule_hash` (rule set immutable per instance)
    /// - `seq` increments by exactly 1
    /// - `amount` must be > 0 and ≤ `prev.ktt.amount`
    pub fn transfer(
        prev: &CsciState,
        new_owner: [u8; 32],
        amount: u64,
        rule_hash: [u8; 32],
    ) -> Result<Self> {
        if amount == 0 {
            return Err(CsciError::ZeroAmount);
        }
        if amount > prev.ktt.amount {
            return Err(CsciError::InsufficientBalance {
                amount,
                balance: prev.ktt.amount,
            });
        }
        if rule_hash != prev.rule_hash {
            return Err(CsciError::RuleHashMismatch {
                expected: hex_fmt(&prev.rule_hash),
                actual: hex_fmt(&rule_hash),
            });
        }
        let seq = prev.seq.checked_add(1).ok_or(CsciError::SeqOverflow)?;
        let new_state = CsciState {
            ktt: KttState {
                identifier_type: IdentifierType::Pubkey,
                owner_identifier: new_owner,
                amount,
                is_minter: false,
            },
            seq,
            rule_hash,
            covenant_id: prev.covenant_id,
        };
        Ok(Self {
            prev: prev.clone(),
            new_state,
        })
    }

    /// The `JournalSpec` the vProg commits to for this transition.
    pub fn journal_spec(&self) -> JournalSpec {
        self.new_state.as_journal_spec()
    }

    /// 32-byte journal hash — the `journal` argument for `build_pq_anchor_redeem`.
    pub fn journal_hash(&self) -> [u8; 32] {
        self.journal_spec().journal_hash()
    }

    /// Raw journal bytes (104 B) — must be pushed in clear in the unlocking script
    /// so the covenant locking script can inspect fields and independently compute
    /// `sha256(journal_bytes)` to bind them to the proof.
    pub fn journal_bytes(&self) -> Vec<u8> {
        self.journal_spec()
            .journal_bytes()
            .expect("CsciTransition always returns Some")
    }
}

fn hex_fmt(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
