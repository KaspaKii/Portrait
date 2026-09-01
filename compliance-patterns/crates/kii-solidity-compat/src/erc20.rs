//! `ERC20`-shaped facade over `kcp-ktt-token`.
//!
//! An Ethereum developer will recognise the method names: `initial_mint`,
//! `transfer`, `burn`, `mint_more`. The underlying model is UTXO-based, not
//! account-based — see each method's doc for what that means in practice.
//!
//! **Not present:** `balanceOf` (query UTXOs from the node and sum amounts),
//! `approve`/`allowance`/`transferFrom` (the allowance pattern doesn't port to
//! UTXO; model delegated spending as a covenant condition instead).

use kcp_ktt_token::{
    state::{IdentifierType, KttState},
    token::{burn as ktt_burn, mint as ktt_mint, transfer as ktt_transfer, AuthContext},
};

use crate::error::{Error, Result};

/// A KTT token descriptor — the Kaspa equivalent of an `ERC20` contract.
///
/// `Token` holds the token's metadata and the minter's key. It has no mutable
/// state; all operations return new [`KttState`] values that represent UTXOs to
/// be included in a transaction.
///
/// **Kaspa difference:** there is no contract at an address, no storage slot for
/// the total supply, and no global balances mapping. A holder's balance is the
/// sum of the KTT-marked UTXOs they can spend. "Deploying" a token means
/// broadcasting the initial mint transaction that creates those UTXOs.
#[derive(Debug, Clone)]
pub struct Token {
    /// Human-readable token name (e.g. `"Acme Token"`).
    pub name: String,
    /// Ticker symbol (e.g. `"ACM"`).
    pub symbol: String,
    /// Decimal places for display. Does not affect on-chain `amount` (always in
    /// the smallest unit, like sompi).
    pub decimals: u8,
    /// 32-byte x-only Schnorr public key of the issuer / minter.
    pub minter_key: [u8; 32],
}

impl Token {
    /// Construct a new token descriptor — the `ERC20(name, symbol)` constructor
    /// equivalent.
    ///
    /// This does **not** issue any UTXOs. Call [`initial_mint`](Token::initial_mint)
    /// to create the first token states.
    pub fn new(
        name: impl Into<String>,
        symbol: impl Into<String>,
        decimals: u8,
        minter_key: [u8; 32],
    ) -> Self {
        Self {
            name: name.into(),
            symbol: symbol.into(),
            decimals,
            minter_key,
        }
    }

    /// Issue the initial token supply — the `_mint(to, amount)` constructor
    /// pattern.
    ///
    /// Returns `(recipient_state, persisted_minter_state)`.
    ///
    /// - `recipient_state` — the UTXO carrying `amount` tokens for `recipient`.
    /// - `persisted_minter_state` — the minter UTXO that must be included as an
    ///   output of the issuance transaction so future minting remains possible.
    ///
    /// **Kaspa difference:** there is no implicit `msg.sender`. The minter is
    /// identified by [`Token::minter_key`] and must sign the issuance transaction.
    pub fn initial_mint(&self, recipient: [u8; 32], amount: u64) -> Result<(KttState, KttState)> {
        let minter_input = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: self.minter_key,
            amount: 0,
            is_minter: true,
        };
        let recipient_output = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: recipient,
            amount,
            is_minter: false,
        };
        let minter_persisted = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: self.minter_key,
            amount: 0,
            is_minter: true,
        };
        let auth = AuthContext {
            authorised_owners: vec![self.minter_key],
        };
        ktt_mint(
            &minter_input,
            &recipient_output,
            &minter_persisted,
            &auth,
            0,
        )?;
        Ok((recipient_output, minter_persisted))
    }

    /// Transfer tokens to another key — the `transfer(to, amount)` ERC20 method.
    ///
    /// Takes one UTXO (`from_state`) and produces `(recipient_output,
    /// change_output)`:
    ///
    /// - `recipient_output` — `amount` tokens, owned by `to`.
    /// - `change_output` — `from_state.amount - amount` tokens returned to the
    ///   sender (may be 0 when the full balance is transferred).
    ///
    /// **Kaspa difference:** you pass the *UTXO* being spent, not an address
    /// with a balance. If a holder has multiple UTXOs, they are separate inputs;
    /// this method handles one at a time. Use `kcp_ktt_token::token::transfer`
    /// directly for multi-input fan-in scenarios.
    ///
    /// **Not present:** `approve`/`transferFrom`. Model delegated spending as a
    /// covenant spend condition instead.
    pub fn transfer(
        &self,
        from_state: &KttState,
        to: [u8; 32],
        amount: u64,
    ) -> Result<(KttState, KttState)> {
        if amount > from_state.amount {
            return Err(Error::InsufficientBalance {
                available: from_state.amount,
                requested: amount,
            });
        }
        let recipient_output = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: to,
            amount,
            is_minter: false,
        };
        let change_output = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: from_state.owner_identifier,
            amount: from_state.amount - amount,
            is_minter: false,
        };
        let auth = AuthContext {
            authorised_owners: vec![from_state.owner_identifier],
        };
        ktt_transfer(
            std::slice::from_ref(from_state),
            &[recipient_output.clone(), change_output.clone()],
            &auth,
            0,
        )?;
        Ok((recipient_output, change_output))
    }

    /// Mint additional tokens via the minter UTXO — the `_mint(to, amount)`
    /// pattern for post-deployment issuance.
    ///
    /// Returns `(recipient_state, persisted_minter_state)`.
    ///
    /// The `minter_state` input must have `is_minter = true` and be owned by
    /// [`Token::minter_key`].
    pub fn mint_more(
        &self,
        minter_state: &KttState,
        recipient: [u8; 32],
        amount: u64,
    ) -> Result<(KttState, KttState)> {
        let recipient_output = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: recipient,
            amount,
            is_minter: false,
        };
        let minter_persisted = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: self.minter_key,
            amount: 0,
            is_minter: true,
        };
        let auth = AuthContext {
            authorised_owners: vec![self.minter_key],
        };
        ktt_mint(minter_state, &recipient_output, &minter_persisted, &auth, 0)?;
        Ok((recipient_output, minter_persisted))
    }

    /// Burn tokens — the `_burn(from, amount)` ERC20 pattern.
    ///
    /// Returns the `remaining_state` UTXO (amount = original - burned).
    /// The caller must include the remaining UTXO as a transaction output; if
    /// the full balance is burned the remaining amount is 0.
    pub fn burn(&self, owner_state: &KttState, amount: u64) -> Result<KttState> {
        if amount > owner_state.amount {
            return Err(Error::InsufficientBalance {
                available: owner_state.amount,
                requested: amount,
            });
        }
        let remaining = owner_state.amount - amount;
        let output = KttState {
            identifier_type: IdentifierType::Pubkey,
            owner_identifier: owner_state.owner_identifier,
            amount: remaining,
            is_minter: false,
        };
        let auth = AuthContext {
            authorised_owners: vec![owner_state.owner_identifier],
        };
        ktt_burn(owner_state, &output, amount, &auth, 0)?;
        Ok(output)
    }
}
