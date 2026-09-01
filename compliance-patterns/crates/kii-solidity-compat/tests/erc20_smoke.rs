use kii_solidity_compat::erc20::Token;

const MINTER: [u8; 32] = [0x0fu8; 32];
const ALICE: [u8; 32] = [0x01u8; 32];
const BOB: [u8; 32] = [0x02u8; 32];

fn token() -> Token {
    Token::new("Acme Token", "ACM", 8, MINTER)
}

#[test]
fn initial_mint_produces_correct_states() {
    let token = token();
    let (recipient, minter_persisted) = token.initial_mint(ALICE, 1_000_000).unwrap();
    assert_eq!(recipient.amount, 1_000_000);
    assert_eq!(recipient.owner_identifier, ALICE);
    assert!(!recipient.is_minter);
    assert!(minter_persisted.is_minter);
    assert_eq!(minter_persisted.owner_identifier, MINTER);
}

#[test]
fn transfer_splits_correctly() {
    let token = token();
    let (alice_state, _) = token.initial_mint(ALICE, 1_000_000).unwrap();
    let (to_bob, change) = token.transfer(&alice_state, BOB, 300_000).unwrap();
    assert_eq!(to_bob.amount, 300_000);
    assert_eq!(to_bob.owner_identifier, BOB);
    assert_eq!(change.amount, 700_000);
    assert_eq!(change.owner_identifier, ALICE);
}

#[test]
fn transfer_full_balance_zero_change() {
    let token = token();
    let (alice_state, _) = token.initial_mint(ALICE, 500).unwrap();
    let (to_bob, change) = token.transfer(&alice_state, BOB, 500).unwrap();
    assert_eq!(to_bob.amount, 500);
    assert_eq!(change.amount, 0);
}

#[test]
fn transfer_insufficient_balance_rejected() {
    let token = token();
    let (alice_state, _) = token.initial_mint(ALICE, 100).unwrap();
    let err = token.transfer(&alice_state, BOB, 101).unwrap_err();
    assert!(matches!(
        err,
        kii_solidity_compat::Error::InsufficientBalance {
            available: 100,
            requested: 101
        }
    ));
}

#[test]
fn mint_more_produces_recipient_and_minter() {
    let token = token();
    let (_, minter_state) = token.initial_mint(ALICE, 1_000_000).unwrap();
    let (new_tokens, new_minter) = token.mint_more(&minter_state, BOB, 500_000).unwrap();
    assert_eq!(new_tokens.amount, 500_000);
    assert_eq!(new_tokens.owner_identifier, BOB);
    assert!(new_minter.is_minter);
}

#[test]
fn burn_reduces_balance() {
    let token = token();
    let (alice_state, _) = token.initial_mint(ALICE, 1_000_000).unwrap();
    let remaining = token.burn(&alice_state, 400_000).unwrap();
    assert_eq!(remaining.amount, 600_000);
    assert_eq!(remaining.owner_identifier, ALICE);
    assert!(!remaining.is_minter);
}

#[test]
fn burn_all_produces_zero_remaining() {
    let token = token();
    let (alice_state, _) = token.initial_mint(ALICE, 750).unwrap();
    let remaining = token.burn(&alice_state, 750).unwrap();
    assert_eq!(remaining.amount, 0);
}

#[test]
fn burn_exceeds_balance_rejected() {
    let token = token();
    let (alice_state, _) = token.initial_mint(ALICE, 100).unwrap();
    let err = token.burn(&alice_state, 101).unwrap_err();
    assert!(matches!(
        err,
        kii_solidity_compat::Error::InsufficientBalance {
            available: 100,
            requested: 101
        }
    ));
}
