//! Compliance-attestation hook: TransferRule bitmask.
//!
//! These constants model the compliance control vocabulary from the KTT donor
//! codebase (src/types/index.ts, lines 24-33). They are **modelled** in v0 —
//! the bitmask is accepted by token-operation functions as a parameter but is
//! not enforced on-chain. On-chain enforcement requires a compliance oracle or
//! a covenant that embeds the ruleset; that is the documented next step.
//!
//! Callers can combine flags with bitwise-OR:
//!
//! ```rust
//! use kcp_ktt_token::transfer_rules::{KYC_REQUIRED, WHITELIST_ONLY};
//! let rules: u32 = KYC_REQUIRED | WHITELIST_ONLY;
//! ```

/// KYC check required before transfer is permitted.
pub const KYC_REQUIRED: u32 = 0x0001;

/// Jurisdiction restriction: only participants in allowed jurisdictions.
pub const JURISDICTION: u32 = 0x0002;

/// Holding period: token must be held for a minimum number of blocks.
pub const HOLDING_PERIOD: u32 = 0x0004;

/// Per-transfer limit enforced (maximum amount per transaction).
pub const TRANSFER_LIMIT: u32 = 0x0008;

/// Authority may freeze balances.
pub const FREEZE_CAPABLE: u32 = 0x0010;

/// Authority may clawback tokens from any holder.
pub const CLAWBACK: u32 = 0x0020;

/// Only whitelisted addresses may receive tokens.
pub const WHITELIST_ONLY: u32 = 0x0040;

/// Only accredited investors may hold tokens.
pub const ACCREDITED_ONLY: u32 = 0x0080;

/// Returns `true` if `rules` has the given `flag` set.
pub fn has_flag(rules: u32, flag: u32) -> bool {
    rules & flag != 0
}

/// Returns a human-readable description of the active flags in `rules`.
pub fn describe(rules: u32) -> Vec<&'static str> {
    let mut active = Vec::new();
    if has_flag(rules, KYC_REQUIRED) {
        active.push("KYC_REQUIRED");
    }
    if has_flag(rules, JURISDICTION) {
        active.push("JURISDICTION");
    }
    if has_flag(rules, HOLDING_PERIOD) {
        active.push("HOLDING_PERIOD");
    }
    if has_flag(rules, TRANSFER_LIMIT) {
        active.push("TRANSFER_LIMIT");
    }
    if has_flag(rules, FREEZE_CAPABLE) {
        active.push("FREEZE_CAPABLE");
    }
    if has_flag(rules, CLAWBACK) {
        active.push("CLAWBACK");
    }
    if has_flag(rules, WHITELIST_ONLY) {
        active.push("WHITELIST_ONLY");
    }
    if has_flag(rules, ACCREDITED_ONLY) {
        active.push("ACCREDITED_ONLY");
    }
    active
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_flag_single() {
        assert!(has_flag(KYC_REQUIRED, KYC_REQUIRED));
        assert!(!has_flag(KYC_REQUIRED, JURISDICTION));
    }

    #[test]
    fn has_flag_combined() {
        let rules = KYC_REQUIRED | WHITELIST_ONLY;
        assert!(has_flag(rules, KYC_REQUIRED));
        assert!(has_flag(rules, WHITELIST_ONLY));
        assert!(!has_flag(rules, CLAWBACK));
    }

    #[test]
    fn describe_empty() {
        assert!(describe(0).is_empty());
    }

    #[test]
    fn describe_all() {
        let all = KYC_REQUIRED
            | JURISDICTION
            | HOLDING_PERIOD
            | TRANSFER_LIMIT
            | FREEZE_CAPABLE
            | CLAWBACK
            | WHITELIST_ONLY
            | ACCREDITED_ONLY;
        let names = describe(all);
        assert_eq!(names.len(), 8);
        assert!(names.contains(&"KYC_REQUIRED"));
        assert!(names.contains(&"ACCREDITED_ONLY"));
    }
}
