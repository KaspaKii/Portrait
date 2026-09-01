//! KTT token operations as pure state transitions.
//!
//! Each function takes input [`KttState`]s and produces output [`KttState`]s,
//! enforcing the KCC20-shape invariants defined in FACTS SS-008b. All
//! validation is **off-chain** in v0; the output states would be submitted as
//! covenant arguments to `validateOutputStateWithTemplate` in a future on-chain
//! version.
//!
//! ## Invariants enforced
//!
//! | Code | Rule |
//! |---|---|
//! | KTT-1 | Supply conservation: when no minter input is involved, `sum(output.amount) == sum(input.amount)` |
//! | KTT-2 | No minter escalation: a non-minter input cannot produce a minter output |
//! | KTT-3 | Owner authorisation: the authorised owner(s) must be present for the spending input |
//! | KTT-4 | Identifier type validity: all output states carry a recognised identifier type |
//!
//! ## Compliance hooks
//!
//! [`transfer_rules`](crate::transfer_rules) provides a TransferRule bitmask
//! type. These hooks are **modelled** here (the `transfer_rules` parameter is
//! accepted and stored) but not enforced in v0. Enforcement requires an
//! on-chain compliance oracle or a covenant that embeds the ruleset.

use crate::{
    error::{Error, Result},
    state::KttState,
};

/// Context for owner authorisation checks (KTT-3).
///
/// In v0 the check is off-chain: the caller asserts which owner identifiers
/// are authorised (i.e. the signing key(s) present in the transaction). On-
/// chain enforcement of `OP_CHECKSIG` / covenant owner checks is the
/// documented next step.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Owner identifiers whose signatures are present in the transaction.
    /// Each entry is a 32-byte identifier matching the `owner_identifier`
    /// field of an input [`KttState`].
    pub authorised_owners: Vec<[u8; 32]>,
}

impl AuthContext {
    /// Returns `true` if `owner_identifier` is listed as authorised.
    pub fn is_authorised(&self, owner_identifier: &[u8; 32]) -> bool {
        self.authorised_owners.iter().any(|a| a == owner_identifier)
    }
}

/// Transfer tokens from one or more inputs to one or more outputs.
///
/// Rules applied:
/// - KTT-1: `sum(outputs.amount) == sum(inputs.amount)` (conservation).
/// - KTT-2: no output may be a minter unless a minter input is present.
/// - KTT-3: every input owner must be authorised.
/// - KTT-4: all output identifier types must be valid (already enforced by
///   [`KttState`] construction, but re-checked for belt-and-suspenders).
///
/// The `_transfer_rules` parameter is accepted (reserved for future on-chain
/// enforcement) but not evaluated in v0.
///
/// # Errors
///
/// Returns the first [`Error`] that violates an invariant.
pub fn transfer(
    inputs: &[KttState],
    outputs: &[KttState],
    auth: &AuthContext,
    _transfer_rules: u32,
) -> Result<()> {
    if inputs.is_empty() {
        return Err(Error::NoInputs);
    }
    if outputs.is_empty() {
        return Err(Error::NoOutputs);
    }

    // KTT-3: all input owners must be authorised.
    for input in inputs {
        if !auth.is_authorised(&input.owner_identifier) {
            return Err(Error::OwnerAuthAbsent);
        }
    }

    let has_minter_input = inputs.iter().any(|s| s.is_minter);

    // KTT-2: no minter escalation from non-minter inputs.
    if !has_minter_input {
        for output in outputs {
            if output.is_minter {
                return Err(Error::MinterEscalation);
            }
        }
    }

    // KTT-4: identifier types are valid (IdentifierType is an enum so
    // construction already guards this; check is illustrative in v0).
    check_output_identifier_types(outputs)?;

    // KTT-1: supply conservation (only when no minter input is involved).
    if !has_minter_input {
        let input_sum = sum_amounts(inputs)?;
        let output_sum = sum_amounts(outputs)?;
        if input_sum != output_sum {
            return Err(Error::SupplyConservation {
                input_sum,
                output_sum,
            });
        }
    }

    Ok(())
}

/// Split one input state into multiple output states.
///
/// Convenience wrapper around [`transfer`] for the common case of a single
/// input split into `n` outputs. Same invariants apply.
pub fn split(
    input: &KttState,
    outputs: &[KttState],
    auth: &AuthContext,
    transfer_rules: u32,
) -> Result<()> {
    transfer(std::slice::from_ref(input), outputs, auth, transfer_rules)
}

/// Merge multiple input states into one output state.
///
/// Convenience wrapper around [`transfer`] for the common case of `n` inputs
/// merged into a single output. Same invariants apply.
pub fn merge(
    inputs: &[KttState],
    output: &KttState,
    auth: &AuthContext,
    transfer_rules: u32,
) -> Result<()> {
    transfer(inputs, std::slice::from_ref(output), auth, transfer_rules)
}

/// Mint new tokens via a minter state.
///
/// The minter input produces:
/// - a **recipient branch**: a non-minter output state with the minted amount,
/// - a **minter branch**: the original minter state persisting (is_minter=true).
///
/// Rules applied:
/// - Requires at least one minter input (KTT error: [`Error::MintWithoutMinter`]).
/// - KTT-3: the minter input's owner must be authorised.
/// - KTT-2: non-minter inputs cannot produce minter outputs (the minter
///   branch comes from a minter input; non-minter inputs route to non-minter
///   outputs).
/// - KTT-4: output identifier types must be valid.
///
/// Supply conservation (KTT-1) is **not** required for the mint operation
/// by design: the minter branch may produce `recipient_amount` new tokens.
///
/// The `_transfer_rules` parameter is reserved for future on-chain enforcement.
pub fn mint(
    minter_input: &KttState,
    minted_output: &KttState,
    persisted_minter: &KttState,
    auth: &AuthContext,
    _transfer_rules: u32,
) -> Result<()> {
    // Requires a minter input.
    if !minter_input.is_minter {
        return Err(Error::MintWithoutMinter);
    }

    // KTT-3: minter input owner must be authorised.
    if !auth.is_authorised(&minter_input.owner_identifier) {
        return Err(Error::OwnerAuthAbsent);
    }

    // The minted output must not be a minter (only the minter branch persists).
    if minted_output.is_minter {
        return Err(Error::MinterEscalation);
    }

    // The persisted minter branch must remain a minter.
    if !persisted_minter.is_minter {
        return Err(Error::MintWithoutMinter);
    }

    // KTT-4.
    check_output_identifier_types(&[minted_output.clone(), persisted_minter.clone()])?;

    Ok(())
}

/// Burn tokens by reducing the amount on an input state.
///
/// Produces a single output state with `input.amount - burn_amount`.
///
/// Rules applied:
/// - KTT-3: input owner must be authorised.
/// - KTT-2: non-minter input cannot produce a minter output (enforced
///   regardless of the `output.is_minter` field the caller supplies).
/// - KTT-4: output identifier type must be valid.
/// - Burn amount must not exceed the input amount.
///
/// Supply conservation (KTT-1) is intentionally **not** required: the
/// remaining amount may be less than the input amount.
pub fn burn(
    input: &KttState,
    output: &KttState,
    burn_amount: u64,
    auth: &AuthContext,
    _transfer_rules: u32,
) -> Result<()> {
    // KTT-3: owner must be authorised.
    if !auth.is_authorised(&input.owner_identifier) {
        return Err(Error::OwnerAuthAbsent);
    }

    // Burn amount must not exceed input.
    if burn_amount > input.amount {
        return Err(Error::BurnExceedsInput {
            burn: burn_amount,
            available: input.amount,
        });
    }

    // The remaining output amount must equal input minus burn.
    let expected_remaining = input.amount - burn_amount;
    if output.amount != expected_remaining {
        return Err(Error::SupplyConservation {
            input_sum: expected_remaining,
            output_sum: output.amount,
        });
    }

    // KTT-2: non-minter input cannot produce a minter output.
    if !input.is_minter && output.is_minter {
        return Err(Error::MinterEscalation);
    }

    // KTT-4.
    check_output_identifier_types(std::slice::from_ref(output))?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Sum the amounts of a set of states, returning [`Error::AmountOverflow`] on
/// arithmetic overflow.
fn sum_amounts(states: &[KttState]) -> Result<u64> {
    states.iter().try_fold(0u64, |acc, s| {
        acc.checked_add(s.amount).ok_or(Error::AmountOverflow)
    })
}

/// Check that all output states carry a valid identifier type.
///
/// Since [`crate::state::IdentifierType`] is a Rust enum, invalid values
/// cannot be constructed via the public API. This function is a belt-and-
/// suspenders guard for the KTT-4 invariant.
fn check_output_identifier_types(outputs: &[KttState]) -> Result<()> {
    for output in outputs {
        // Re-encode and decode the identifier_type byte to confirm it is in
        // the valid range. This is always true for correctly-constructed states
        // but makes the KTT-4 check explicit.
        let byte = output.identifier_type.to_byte();
        crate::state::IdentifierType::from_byte(byte).map_err(|_| Error::InvalidIdentifierType)?;
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::IdentifierType;

    fn make_state(owner: [u8; 32], amount: u64, is_minter: bool) -> KttState {
        KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: owner,
            amount,
            is_minter,
        }
    }

    fn auth(owners: &[[u8; 32]]) -> AuthContext {
        AuthContext {
            authorised_owners: owners.to_vec(),
        }
    }

    const ALICE: [u8; 32] = [0x01u8; 32];
    const BOB: [u8; 32] = [0x02u8; 32];
    const MINTER: [u8; 32] = [0x0fu8; 32];

    // ── transfer ──────────────────────────────────────────────────────────────

    #[test]
    fn transfer_simple_ok() {
        let input = make_state(ALICE, 1_000, false);
        let output = make_state(BOB, 1_000, false);
        transfer(&[input], &[output], &auth(&[ALICE]), 0).unwrap();
    }

    #[test]
    fn transfer_split_ok() {
        let input = make_state(ALICE, 1_000, false);
        let out1 = make_state(BOB, 400, false);
        let out2 = make_state(ALICE, 600, false);
        transfer(&[input], &[out1, out2], &auth(&[ALICE]), 0).unwrap();
    }

    #[test]
    fn transfer_merge_ok() {
        let in1 = make_state(ALICE, 400, false);
        let in2 = make_state(ALICE, 600, false);
        let output = make_state(BOB, 1_000, false);
        transfer(&[in1, in2], &[output], &auth(&[ALICE]), 0).unwrap();
    }

    #[test]
    fn transfer_ktt1_violation() {
        let input = make_state(ALICE, 1_000, false);
        let output = make_state(BOB, 999, false); // one short
        let err = transfer(&[input], &[output], &auth(&[ALICE]), 0).unwrap_err();
        assert!(
            matches!(
                err,
                Error::SupplyConservation {
                    input_sum: 1_000,
                    output_sum: 999
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn transfer_ktt2_minter_escalation() {
        let input = make_state(ALICE, 1_000, false); // non-minter input
        let output = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: BOB,
            amount: 1_000,
            is_minter: true, // escalation attempt
        };
        let err = transfer(&[input], &[output], &auth(&[ALICE]), 0).unwrap_err();
        assert!(matches!(err, Error::MinterEscalation), "unexpected: {err}");
    }

    #[test]
    fn transfer_ktt3_owner_auth_absent() {
        let input = make_state(ALICE, 1_000, false);
        let output = make_state(BOB, 1_000, false);
        // Auth does not include ALICE.
        let err = transfer(&[input], &[output], &auth(&[BOB]), 0).unwrap_err();
        assert!(matches!(err, Error::OwnerAuthAbsent), "unexpected: {err}");
    }

    #[test]
    fn transfer_no_inputs_rejected() {
        let output = make_state(BOB, 0, false);
        let err = transfer(&[], &[output], &auth(&[ALICE]), 0).unwrap_err();
        assert!(matches!(err, Error::NoInputs), "unexpected: {err}");
    }

    #[test]
    fn transfer_no_outputs_rejected() {
        let input = make_state(ALICE, 1_000, false);
        let err = transfer(&[input], &[], &auth(&[ALICE]), 0).unwrap_err();
        assert!(matches!(err, Error::NoOutputs), "unexpected: {err}");
    }

    // ── split / merge wrappers ────────────────────────────────────────────────

    #[test]
    fn split_ok() {
        let input = make_state(ALICE, 500, false);
        let out1 = make_state(ALICE, 200, false);
        let out2 = make_state(BOB, 300, false);
        split(&input, &[out1, out2], &auth(&[ALICE]), 0).unwrap();
    }

    #[test]
    fn merge_ok() {
        let in1 = make_state(ALICE, 200, false);
        let in2 = make_state(ALICE, 300, false);
        let output = make_state(BOB, 500, false);
        merge(&[in1, in2], &output, &auth(&[ALICE]), 0).unwrap();
    }

    // ── mint ──────────────────────────────────────────────────────────────────

    #[test]
    fn mint_ok() {
        let minter_in = make_state(MINTER, 0, true);
        let minted_out = make_state(ALICE, 1_000_000, false);
        let persisted = make_state(MINTER, 0, true);
        mint(&minter_in, &minted_out, &persisted, &auth(&[MINTER]), 0).unwrap();
    }

    #[test]
    fn mint_without_minter_rejected() {
        let non_minter_in = make_state(ALICE, 1_000, false);
        let minted_out = make_state(BOB, 1_000, false);
        let persisted = make_state(ALICE, 0, true); // can't make minter from non-minter
        let err = mint(&non_minter_in, &minted_out, &persisted, &auth(&[ALICE]), 0).unwrap_err();
        assert!(matches!(err, Error::MintWithoutMinter), "unexpected: {err}");
    }

    #[test]
    fn mint_ktt2_minted_output_cannot_be_minter() {
        let minter_in = make_state(MINTER, 0, true);
        let minted_out = make_state(ALICE, 1_000_000, true); // escalation: minted recipient is minter
        let persisted = make_state(MINTER, 0, true);
        let err = mint(&minter_in, &minted_out, &persisted, &auth(&[MINTER]), 0).unwrap_err();
        assert!(matches!(err, Error::MinterEscalation), "unexpected: {err}");
    }

    #[test]
    fn mint_ktt3_owner_auth_absent() {
        let minter_in = make_state(MINTER, 0, true);
        let minted_out = make_state(ALICE, 1_000_000, false);
        let persisted = make_state(MINTER, 0, true);
        // Auth does not include MINTER.
        let err = mint(&minter_in, &minted_out, &persisted, &auth(&[ALICE]), 0).unwrap_err();
        assert!(matches!(err, Error::OwnerAuthAbsent), "unexpected: {err}");
    }

    // ── burn ──────────────────────────────────────────────────────────────────

    #[test]
    fn burn_ok() {
        let input = make_state(ALICE, 1_000_000, false);
        let output = make_state(ALICE, 900_000, false);
        burn(&input, &output, 100_000, &auth(&[ALICE]), 0).unwrap();
    }

    #[test]
    fn burn_all_ok() {
        let input = make_state(ALICE, 500, false);
        let output = make_state(ALICE, 0, false);
        burn(&input, &output, 500, &auth(&[ALICE]), 0).unwrap();
    }

    #[test]
    fn burn_exceeds_input_rejected() {
        let input = make_state(ALICE, 500, false);
        let output = make_state(ALICE, 0, false);
        let err = burn(&input, &output, 501, &auth(&[ALICE]), 0).unwrap_err();
        assert!(
            matches!(
                err,
                Error::BurnExceedsInput {
                    burn: 501,
                    available: 500
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn burn_ktt2_no_minter_escalation() {
        let input = make_state(ALICE, 1_000, false);
        let output = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: ALICE,
            amount: 900,
            is_minter: true, // escalation
        };
        let err = burn(&input, &output, 100, &auth(&[ALICE]), 0).unwrap_err();
        assert!(matches!(err, Error::MinterEscalation), "unexpected: {err}");
    }

    #[test]
    fn burn_ktt3_owner_auth_absent() {
        let input = make_state(ALICE, 1_000, false);
        let output = make_state(ALICE, 900, false);
        let err = burn(&input, &output, 100, &auth(&[BOB]), 0).unwrap_err();
        assert!(matches!(err, Error::OwnerAuthAbsent), "unexpected: {err}");
    }

    #[test]
    fn burn_wrong_remaining_amount_rejected() {
        let input = make_state(ALICE, 1_000, false);
        let output = make_state(ALICE, 800, false); // should be 900
        let err = burn(&input, &output, 100, &auth(&[ALICE]), 0).unwrap_err();
        assert!(
            matches!(
                err,
                Error::SupplyConservation {
                    input_sum: 900,
                    output_sum: 800
                }
            ),
            "unexpected: {err}"
        );
    }
}
