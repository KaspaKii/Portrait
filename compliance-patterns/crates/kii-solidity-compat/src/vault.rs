//! `Vault`-shaped facade over `kcp-vault::SpendCondition`.
//!
//! EVM equivalent: conceptually bridges `ERC4626` (tokenised vault), `Escrow`,
//! and `TimelockController` with the Kaspa covenant model.
//!
//! In Ethereum a vault or escrow is a smart-contract address that holds ETH or
//! tokens under programmatic conditions. In Kaspa, a vault is a UTXO locked
//! under a **covenant script** — the spending condition is enforced by the
//! Toccata consensus engine, not a contract runtime.
//!
//! | EVM concept | This type |
//! |---|---|
//! | `ERC4626` constructor | `Vault::new(condition)` |
//! | `deposit(amount, receiver)` | `vault.deposit(amount_sompi)` → `VaultDescriptor` |
//! | `maxWithdraw(owner)` | `descriptor.amount_sompi` |
//! | Condition check | `vault.evaluate(ctx)` |
//! | Spend (withdraw) | broadcast a tx satisfying the condition script |
//!
//! **Not present:** `approve`/`allowance`, tokenised shares (ERC4626 receipt
//! token). Model tokenised shares via `kcp-ktt-token` if needed.
//!
//! **Kaspa difference:** `deposit` does not move funds — it returns a
//! `VaultDescriptor` that describes what the output UTXO should look like.
//! Broadcasting that UTXO as a transaction output creates the vault lock
//! on-chain.
//!
//! **Pre-production, unaudited, testnet-only.**

use kcp_vault::{
    condition::SpendCondition,
    evaluator::{evaluate, EvalContext},
};

use crate::error::{Error, Result};

/// A vault descriptor — the Kaspa equivalent of a vault smart contract.
///
/// Holds the spending condition and the locked amount.
#[derive(Debug, Clone)]
pub struct VaultDescriptor {
    /// The spending condition that governs when and by whom the UTXO can be
    /// spent.
    pub condition: SpendCondition,
    /// The amount locked in the vault (in sompi).
    pub amount_sompi: u64,
}

/// A `Vault` factory — the Kaspa equivalent of deploying a vault contract.
///
/// `Vault` holds the spending condition template. Call [`deposit`] to create
/// a `VaultDescriptor` for a specific amount.
///
/// [`deposit`]: Vault::deposit
#[derive(Debug, Clone)]
pub struct Vault {
    condition: SpendCondition,
}

impl Vault {
    /// Create a new vault with the given spending condition.
    ///
    /// The condition is validated immediately; `Err(VaultConditionInvalid)` is
    /// returned for structurally invalid conditions (see
    /// [`SpendCondition::validate`]).
    ///
    /// Equivalent to deploying an `ERC4626` or `Escrow` contract with the
    /// specified unlock logic.
    pub fn new(condition: SpendCondition) -> Result<Self> {
        condition
            .validate()
            .map_err(|e| Error::VaultConditionInvalid(e.to_string()))?;
        Ok(Self { condition })
    }

    /// Describe a deposit of `amount_sompi` into this vault.
    ///
    /// Returns a [`VaultDescriptor`] that represents the UTXO to create. The
    /// caller is responsible for including it as an output in the locking
    /// transaction.
    ///
    /// Equivalent to `ERC4626.deposit(assets, receiver)` — note there is no
    /// on-chain call; the descriptor must be turned into a UTXO by the caller.
    ///
    /// Returns `Err(ZeroDeposit)` if `amount_sompi == 0`.
    pub fn deposit(&self, amount_sompi: u64) -> Result<VaultDescriptor> {
        if amount_sompi == 0 {
            return Err(Error::ZeroDeposit);
        }
        Ok(VaultDescriptor {
            condition: self.condition.clone(),
            amount_sompi,
        })
    }

    /// Evaluate whether the spending condition is currently satisfied.
    ///
    /// `ctx` provides the current DAA height, unix timestamp, and the set of
    /// signers whose signatures are present in the spending transaction.
    ///
    /// Returns `true` if the vault can be spent under these conditions.
    pub fn evaluate(&self, ctx: &EvalContext) -> bool {
        evaluate(&self.condition, ctx)
    }

    /// Return the vault's spending condition.
    pub fn condition(&self) -> &SpendCondition {
        &self.condition
    }
}

impl VaultDescriptor {
    /// Evaluate whether the spending condition is currently satisfied.
    pub fn evaluate(&self, ctx: &EvalContext) -> bool {
        evaluate(&self.condition, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kcp_vault::condition::SpendCondition;

    fn key(b: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = b;
        k
    }

    fn timelock_condition(deadline: u64) -> SpendCondition {
        SpendCondition::TimelockHeight {
            deadline,
            controller_xonly: key(1),
        }
    }

    fn multisig_condition() -> SpendCondition {
        SpendCondition::MultiSig {
            threshold: 2,
            xonly_keys: vec![key(1), key(2), key(3)],
        }
    }

    #[test]
    fn new_rejects_invalid_condition() {
        let bad = SpendCondition::MultiSig {
            threshold: 5,
            xonly_keys: vec![key(1)], // threshold > keys
        };
        assert!(matches!(
            Vault::new(bad),
            Err(Error::VaultConditionInvalid(_))
        ));
    }

    #[test]
    fn deposit_zero_rejected() {
        let vault = Vault::new(timelock_condition(1_000)).unwrap();
        assert!(matches!(vault.deposit(0), Err(Error::ZeroDeposit)));
    }

    #[test]
    fn deposit_returns_descriptor() {
        let vault = Vault::new(timelock_condition(1_000)).unwrap();
        let desc = vault.deposit(500_000_000).unwrap();
        assert_eq!(desc.amount_sompi, 500_000_000);
    }

    #[test]
    fn timelock_evaluates_correctly() {
        let vault = Vault::new(timelock_condition(1_000)).unwrap();
        let before = EvalContext {
            daa_score: 999,
            unix_seconds: 0,
            signers_present: vec![],
        };
        let at = EvalContext {
            daa_score: 1_000,
            unix_seconds: 0,
            signers_present: vec![],
        };
        assert!(!vault.evaluate(&before));
        assert!(vault.evaluate(&at));
    }

    #[test]
    fn multisig_evaluates_correctly() {
        let vault = Vault::new(multisig_condition()).unwrap();
        let one_signer = EvalContext {
            daa_score: 0,
            unix_seconds: 0,
            signers_present: vec![key(1)],
        };
        let two_signers = EvalContext {
            daa_score: 0,
            unix_seconds: 0,
            signers_present: vec![key(1), key(2)],
        };
        assert!(!vault.evaluate(&one_signer));
        assert!(vault.evaluate(&two_signers));
    }

    #[test]
    fn descriptor_evaluate_delegates() {
        let vault = Vault::new(timelock_condition(500)).unwrap();
        let desc = vault.deposit(1_000_000).unwrap();
        let ctx = EvalContext {
            daa_score: 500,
            unix_seconds: 0,
            signers_present: vec![],
        };
        assert!(desc.evaluate(&ctx));
    }
}
