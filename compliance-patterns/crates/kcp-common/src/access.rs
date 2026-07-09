//! Access-control primitives: `Ownable` and `Multisig`.
//!
//! **Pre-production, unaudited.** Pure offline; no engine dependency.
//! These are EVM-equivalent building blocks in the sense of interface shape
//! and intended use — not a production-ready, audited implementation.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Maximum number of public keys in a [`Multisig`] struct.
pub const MAX_MULTISIG_KEYS: usize = 16;

// ── Ownable ───────────────────────────────────────────────────────────────────

/// Single-controller access primitive: wraps a 32-byte x-only Schnorr public
/// key designating the sole authorised controller of a resource.
///
/// EVM equivalent: `Ownable` (ERC-173)
/// — pre-production, unaudited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ownable([u8; 32]);

impl Serialize for Ownable {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for Ownable {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes (64 hex chars)"))?;
        Ok(Ownable(arr))
    }
}

impl Ownable {
    /// Construct an `Ownable` from a 32-byte x-only public key.
    pub fn new(xonly: [u8; 32]) -> Self {
        Ownable(xonly)
    }

    /// Return a reference to the inner x-only public key bytes.
    pub fn xonly_key(&self) -> &[u8; 32] {
        &self.0
    }

    /// Check the byte shape of this `Ownable` (always `Ok` — a 32-byte value
    /// is always structurally well-formed).
    ///
    /// **This method does NOT verify that the key is a valid secp256k1 curve
    /// point.** The all-zeros byte string, for example, passes this check but
    /// is not on the curve and would produce an unspendable lock script.
    /// Callers who need cryptographic validity must verify the key against the
    /// curve before embedding it in any script or on-chain operation.
    pub fn validate(&self) -> Result<()> {
        Ok(())
    }
}

// ── Multisig ──────────────────────────────────────────────────────────────────

/// k-of-n multisig access primitive: a threshold number of signatures from a
/// set of x-only Schnorr public keys are required to authorise an action.
///
/// EVM equivalent: `AccessControl` (role-based) or Gnosis Safe multisig pattern
/// — pre-production, unaudited.
///
/// Both fields are `pub` for flexibility, but always call [`Multisig::validate`]
/// before using a value in any script or on-chain operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Multisig {
    /// Number of signatures required (1 ≤ `threshold` ≤ `xonly_keys.len()`).
    pub threshold: u8,
    /// Ordered list of 32-byte x-only Schnorr public keys. Must contain between
    /// 1 and [`MAX_MULTISIG_KEYS`] entries, with no duplicates.
    #[serde(
        serialize_with = "serialize_vec_bytes32",
        deserialize_with = "deserialize_vec_bytes32"
    )]
    pub xonly_keys: Vec<[u8; 32]>,
}

impl Multisig {
    /// Construct a `Multisig` from a threshold and a list of x-only public keys.
    ///
    /// This constructor does not validate its arguments. Call
    /// [`Multisig::validate`] before using this value in any script or
    /// on-chain operation.
    pub fn new(threshold: u8, xonly_keys: Vec<[u8; 32]>) -> Self {
        Multisig {
            threshold,
            xonly_keys,
        }
    }

    /// Validate the `Multisig`, returning an error if any rule is violated:
    ///
    /// - `threshold >= 1`
    /// - `xonly_keys.len() >= 1`
    /// - `threshold <= xonly_keys.len()`
    /// - `xonly_keys.len() <= MAX_MULTISIG_KEYS`
    /// - no duplicate keys
    pub fn validate(&self) -> Result<()> {
        let n = self.xonly_keys.len();
        if n == 0 {
            return Err(Error::ConditionInvalid(
                "Multisig: xonly_keys must not be empty".into(),
            ));
        }
        if n > MAX_MULTISIG_KEYS {
            return Err(Error::ConditionInvalid(format!(
                "Multisig: xonly_keys.len() = {n} exceeds maximum {MAX_MULTISIG_KEYS}"
            )));
        }
        let t = self.threshold as usize;
        if t == 0 {
            return Err(Error::ConditionInvalid(
                "Multisig: threshold must be at least 1".into(),
            ));
        }
        if t > n {
            return Err(Error::ConditionInvalid(format!(
                "Multisig: threshold {t} exceeds key count {n}"
            )));
        }
        // n ≤ 16, so O(n²) is negligible and avoids an extra import.
        for i in 0..self.xonly_keys.len() {
            for j in (i + 1)..self.xonly_keys.len() {
                if self.xonly_keys[i] == self.xonly_keys[j] {
                    return Err(Error::ConditionInvalid(format!(
                        "Multisig: duplicate key at indices {i} and {j}"
                    )));
                }
            }
        }
        Ok(())
    }
}

// ── Serde helpers for [u8; 32] as hex strings ─────────────────────────────────

fn serialize_vec_bytes32<S>(vec: &[[u8; 32]], s: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(vec.len()))?;
    for item in vec {
        seq.serialize_element(&hex::encode(item))?;
    }
    seq.end()
}

fn deserialize_vec_bytes32<'de, D>(d: D) -> std::result::Result<Vec<[u8; 32]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let strs: Vec<String> = Vec::deserialize(d)?;
    strs.into_iter()
        .map(|s| {
            let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
            bytes
                .try_into()
                .map_err(|_| serde::de::Error::custom("expected 32 bytes (64 hex chars)"))
        })
        .collect()
}

// ── Ownable2Step ──────────────────────────────────────────────────────────────

/// Two-step ownership transfer: the current owner proposes a new owner, and
/// the new owner must explicitly accept before the transfer takes effect.
///
/// EVM equivalent: `Ownable2Step` (Solidity pattern-library v5 shape)
/// — pre-production, unaudited.
///
/// **This is a pure value type.** It tracks state only; callers are responsible
/// for verifying the authorising key matches `current_owner()` or
/// `pending_owner()` as required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ownable2Step {
    current: [u8; 32],
    pending: Option<[u8; 32]>,
}

impl Ownable2Step {
    /// Construct a new `Ownable2Step` with the given initial owner and no
    /// pending transfer.
    pub fn new(owner: [u8; 32]) -> Self {
        Self {
            current: owner,
            pending: None,
        }
    }

    /// Returns the current owner's x-only public key.
    pub fn current_owner(&self) -> &[u8; 32] {
        &self.current
    }

    /// Returns the pending owner's x-only public key, if a transfer is in
    /// progress.
    pub fn pending_owner(&self) -> Option<&[u8; 32]> {
        self.pending.as_ref()
    }

    /// Propose a transfer to `new_owner`. The transfer does not take effect
    /// until `accept_ownership` is called.
    ///
    /// Callers must verify the authorising key matches `current_owner()` before
    /// calling this method.
    pub fn transfer_ownership(&self, new_owner: [u8; 32]) -> Self {
        Self {
            current: self.current,
            pending: Some(new_owner),
        }
    }

    /// Complete the transfer: sets `current` to the pending owner and clears
    /// `pending`.
    ///
    /// Returns `Err` if there is no pending transfer.
    ///
    /// Callers must verify the authorising key matches `pending_owner()` before
    /// calling this method.
    pub fn accept_ownership(&self) -> Result<Self> {
        match self.pending {
            Some(new_owner) => Ok(Self {
                current: new_owner,
                pending: None,
            }),
            None => Err(Error::ConditionInvalid(
                "Ownable2Step: no pending transfer to accept".into(),
            )),
        }
    }

    /// Cancel a pending transfer, leaving the current owner unchanged.
    ///
    /// Returns `Err` if there is no pending transfer.
    ///
    /// Callers must verify the authorising key matches `current_owner()` before
    /// calling this method.
    pub fn cancel_transfer(&self) -> Result<Self> {
        if self.pending.is_none() {
            return Err(Error::ConditionInvalid(
                "Ownable2Step: no pending transfer to cancel".into(),
            ));
        }
        Ok(Self {
            current: self.current,
            pending: None,
        })
    }
}

// ── AccessControl ─────────────────────────────────────────────────────────────

/// Default admin role — all-zeros bytes32, matching the conventional `DEFAULT_ADMIN_ROLE`.
pub const DEFAULT_ADMIN_ROLE: [u8; 32] = [0u8; 32];

/// A role entry: the set of keys that hold the role, and which role is the
/// admin of this role (i.e. who can grant/revoke it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleData {
    /// x-only public keys that currently hold this role.
    #[serde(
        serialize_with = "serialize_vec_bytes32",
        deserialize_with = "deserialize_vec_bytes32"
    )]
    pub members: Vec<[u8; 32]>,
    /// The role that can grant/revoke this role. Defaults to `DEFAULT_ADMIN_ROLE`.
    #[serde(
        serialize_with = "serialize_bytes32",
        deserialize_with = "deserialize_bytes32"
    )]
    pub admin_role: [u8; 32],
}

/// Role-based access control.
///
/// EVM equivalent: `AccessControl` (Solidity pattern-library v5 shape)
/// — pre-production, unaudited.
///
/// Roles are identified by a 32-byte tag. Use `DEFAULT_ADMIN_ROLE` (`[0u8; 32]`)
/// for the top-level admin role.
///
/// **This is a pure value type.** All mutating operations return a new
/// `AccessControl` rather than mutating in place. Callers are responsible for
/// verifying that the authorising key has the required role before calling
/// grant/revoke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessControl {
    roles: Vec<([u8; 32], RoleData)>, // (role_id, data) — ordered Vec for determinism
}

impl Default for AccessControl {
    /// Construct an `AccessControl` with `DEFAULT_ADMIN_ROLE` granted to no
    /// one. The caller must grant the admin role to an initial admin key via
    /// `grant_role(DEFAULT_ADMIN_ROLE, admin_key)` before use.
    fn default() -> Self {
        Self { roles: Vec::new() }
    }
}

impl AccessControl {
    /// Construct a new `AccessControl` with the given key as the initial
    /// `DEFAULT_ADMIN_ROLE` holder.
    pub fn new(initial_admin: [u8; 32]) -> Self {
        let mut ac = Self::default();
        // Grant DEFAULT_ADMIN_ROLE to initial_admin without admin check
        // (bootstrapping).
        ac = ac.grant_role_unchecked(DEFAULT_ADMIN_ROLE, initial_admin);
        ac
    }

    /// Returns `true` if `key` has `role`.
    pub fn has_role(&self, role: [u8; 32], key: &[u8; 32]) -> bool {
        self.role_data(role)
            .map(|d| d.members.contains(key))
            .unwrap_or(false)
    }

    /// Returns the `RoleData` for `role`, if it exists.
    pub fn role_data(&self, role: [u8; 32]) -> Option<&RoleData> {
        self.roles.iter().find(|(r, _)| r == &role).map(|(_, d)| d)
    }

    /// Grant `role` to `key`. Returns `Err` if `role` does not exist and has
    /// no data to initialise from.
    ///
    /// This is the unchecked internal variant — callers who want admin
    /// enforcement must call `grant_role` instead.
    fn grant_role_unchecked(&self, role: [u8; 32], key: [u8; 32]) -> Self {
        let mut roles = self.roles.clone();
        if let Some(pos) = roles.iter().position(|(r, _)| r == &role) {
            let data = &mut roles[pos].1;
            if !data.members.contains(&key) {
                data.members.push(key);
            }
        } else {
            roles.push((
                role,
                RoleData {
                    members: vec![key],
                    admin_role: DEFAULT_ADMIN_ROLE,
                },
            ));
        }
        Self { roles }
    }

    /// Grant `role` to `key`.
    ///
    /// Callers must verify that the authorising key has the admin role of
    /// `role` (via `has_role(role_admin(role), authoriser)`) before calling
    /// this method.
    pub fn grant_role(&self, role: [u8; 32], key: [u8; 32]) -> Self {
        self.grant_role_unchecked(role, key)
    }

    /// Revoke `role` from `key`. Returns `Err` if `key` does not hold `role`.
    ///
    /// Callers must verify that the authorising key has the admin role of
    /// `role` before calling this method.
    pub fn revoke_role(&self, role: [u8; 32], key: [u8; 32]) -> Result<Self> {
        let mut roles = self.roles.clone();
        if let Some(pos) = roles.iter().position(|(r, _)| r == &role) {
            let members = &mut roles[pos].1.members;
            if let Some(i) = members.iter().position(|k| k == &key) {
                members.remove(i);
                return Ok(Self { roles });
            }
        }
        Err(Error::ConditionInvalid(
            "AccessControl: key does not hold the specified role".into(),
        ))
    }

    /// Renounce `role` for `key` (self-removal). Returns `Err` if `key` does
    /// not hold `role`.
    ///
    /// Callers must verify the authorising key matches `key` before calling
    /// this method (a key may only renounce its own roles).
    pub fn renounce_role(&self, role: [u8; 32], key: [u8; 32]) -> Result<Self> {
        self.revoke_role(role, key)
    }

    /// Returns the admin role of `role` — the role that can grant/revoke
    /// `role`. Defaults to `DEFAULT_ADMIN_ROLE` if `role` has no data.
    pub fn role_admin(&self, role: [u8; 32]) -> [u8; 32] {
        self.role_data(role)
            .map(|d| d.admin_role)
            .unwrap_or(DEFAULT_ADMIN_ROLE)
    }

    /// Set the admin role of `role` to `admin_role`.
    ///
    /// Callers must verify that the authorising key has the current admin role
    /// of `role` before calling this method.
    pub fn set_role_admin(&self, role: [u8; 32], admin_role: [u8; 32]) -> Self {
        let mut roles = self.roles.clone();
        if let Some(pos) = roles.iter().position(|(r, _)| r == &role) {
            roles[pos].1.admin_role = admin_role;
        } else {
            roles.push((
                role,
                RoleData {
                    members: vec![],
                    admin_role,
                },
            ));
        }
        Self { roles }
    }
}

// ── Serde helpers for [u8; 32] (single) ──────────────────────────────────────

fn serialize_bytes32<S>(bytes: &[u8; 32], s: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&hex::encode(bytes))
}

fn deserialize_bytes32<'de, D>(d: D) -> std::result::Result<[u8; 32], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
    bytes
        .try_into()
        .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = seed;
        k
    }

    // ── Ownable ───────────────────────────────────────────────────────────────

    #[test]
    fn ownable_round_trips_xonly_key() {
        let k = key(0x42);
        let o = Ownable::new(k);
        assert_eq!(o.xonly_key(), &k);
    }

    /// validate() is shape-only (always Ok). This test guards against
    /// accidental regression (e.g. a refactor that makes it panic).
    /// It does NOT assert cryptographic validity — see the validate() doc.
    #[test]
    fn ownable_validate_is_shape_only() {
        assert!(Ownable::new(key(1)).validate().is_ok());
        // All-zeros is not on the secp256k1 curve but passes the shape check.
        assert!(Ownable::new([0u8; 32]).validate().is_ok());
    }

    #[test]
    fn ownable_serde_round_trip() {
        let o = Ownable::new(key(0xab));
        let json = serde_json::to_string(&o).unwrap();
        let back: Ownable = serde_json::from_str(&json).unwrap();
        assert_eq!(o, back);
    }

    // ── Multisig valid ────────────────────────────────────────────────────────

    #[test]
    fn multisig_1_of_1_valid() {
        let m = Multisig::new(1, vec![key(1)]);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn multisig_2_of_3_valid() {
        let m = Multisig::new(2, vec![key(1), key(2), key(3)]);
        assert!(m.validate().is_ok());
    }

    /// threshold == keys.len() (n-of-n) must be valid.
    #[test]
    fn multisig_n_of_n_valid() {
        let m = Multisig::new(2, vec![key(1), key(2)]);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn multisig_max_keys_valid() {
        let keys: Vec<[u8; 32]> = (0..MAX_MULTISIG_KEYS as u8).map(key).collect();
        let m = Multisig::new(MAX_MULTISIG_KEYS as u8, keys);
        assert!(m.validate().is_ok());
    }

    // ── Multisig invalid ──────────────────────────────────────────────────────

    #[test]
    fn multisig_empty_keys_rejected() {
        let m = Multisig::new(1, vec![]);
        assert!(m.validate().is_err());
    }

    #[test]
    fn multisig_threshold_zero_rejected() {
        let m = Multisig::new(0, vec![key(1)]);
        let err = m.validate().unwrap_err();
        assert!(err.to_string().contains("threshold must be at least 1"));
    }

    #[test]
    fn multisig_threshold_exceeds_keys_rejected() {
        let m = Multisig::new(3, vec![key(1), key(2)]);
        let err = m.validate().unwrap_err();
        assert!(err.to_string().contains("threshold 3 exceeds key count 2"));
    }

    /// threshold == keys.len() + 1 must be invalid (boundary above n-of-n).
    #[test]
    fn multisig_threshold_one_above_n_rejected() {
        let m = Multisig::new(4, vec![key(1), key(2), key(3)]);
        let err = m.validate().unwrap_err();
        assert!(err.to_string().contains("threshold 4 exceeds key count 3"));
    }

    #[test]
    fn multisig_too_many_keys_rejected() {
        let keys: Vec<[u8; 32]> = (0..(MAX_MULTISIG_KEYS + 1) as u8).map(key).collect();
        let m = Multisig::new(1, keys);
        let err = m.validate().unwrap_err();
        assert!(err.to_string().contains("exceeds maximum"));
    }

    #[test]
    fn multisig_duplicate_keys_rejected() {
        let m = Multisig::new(1, vec![key(1), key(2), key(1)]);
        let err = m.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate key"));
    }

    #[test]
    fn multisig_serde_round_trip() {
        let m = Multisig::new(2, vec![key(1), key(2), key(3)]);
        let json = serde_json::to_string(&m).unwrap();
        let back: Multisig = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    // ── Ownable2Step ─────────────────────────────────────────────────────────

    #[test]
    fn ownable2step_initial_state() {
        let o = Ownable2Step::new(key(1));
        assert_eq!(o.current_owner(), &key(1));
        assert!(o.pending_owner().is_none());
    }

    #[test]
    fn ownable2step_transfer_then_accept() {
        let o = Ownable2Step::new(key(1));
        let o2 = o.transfer_ownership(key(2));
        assert_eq!(o2.pending_owner(), Some(&key(2)));
        let o3 = o2.accept_ownership().unwrap();
        assert_eq!(o3.current_owner(), &key(2));
        assert!(o3.pending_owner().is_none());
    }

    #[test]
    fn ownable2step_accept_with_no_pending_fails() {
        let o = Ownable2Step::new(key(1));
        assert!(o.accept_ownership().is_err());
    }

    #[test]
    fn ownable2step_cancel_transfer() {
        let o = Ownable2Step::new(key(1)).transfer_ownership(key(2));
        let o2 = o.cancel_transfer().unwrap();
        assert_eq!(o2.current_owner(), &key(1));
        assert!(o2.pending_owner().is_none());
    }

    #[test]
    fn ownable2step_cancel_with_no_pending_fails() {
        let o = Ownable2Step::new(key(1));
        assert!(o.cancel_transfer().is_err());
    }

    #[test]
    fn ownable2step_serde_round_trip() {
        let o = Ownable2Step::new(key(0xAB)).transfer_ownership(key(0xCD));
        let json = serde_json::to_string(&o).unwrap();
        let back: Ownable2Step = serde_json::from_str(&json).unwrap();
        assert_eq!(o, back);
    }

    // ── AccessControl ─────────────────────────────────────────────────────────

    fn role(seed: u8) -> [u8; 32] {
        let mut r = [0u8; 32];
        r[31] = seed; // put seed at end to avoid conflict with DEFAULT_ADMIN_ROLE
        r
    }

    #[test]
    fn access_control_initial_admin() {
        let ac = AccessControl::new(key(1));
        assert!(ac.has_role(DEFAULT_ADMIN_ROLE, &key(1)));
        assert!(!ac.has_role(DEFAULT_ADMIN_ROLE, &key(2)));
    }

    #[test]
    fn access_control_grant_and_has_role() {
        let ac = AccessControl::new(key(1)).grant_role(role(1), key(2));
        assert!(ac.has_role(role(1), &key(2)));
        assert!(!ac.has_role(role(1), &key(3)));
    }

    #[test]
    fn access_control_revoke_role() {
        let ac = AccessControl::new(key(1)).grant_role(role(1), key(2));
        let ac2 = ac.revoke_role(role(1), key(2)).unwrap();
        assert!(!ac2.has_role(role(1), &key(2)));
    }

    #[test]
    fn access_control_revoke_nonexistent_role_fails() {
        let ac = AccessControl::new(key(1));
        assert!(ac.revoke_role(role(1), key(2)).is_err());
    }

    #[test]
    fn access_control_renounce_role() {
        let ac = AccessControl::new(key(1)).grant_role(role(1), key(2));
        let ac2 = ac.renounce_role(role(1), key(2)).unwrap();
        assert!(!ac2.has_role(role(1), &key(2)));
    }

    #[test]
    fn access_control_set_role_admin() {
        let ac = AccessControl::new(key(1)).set_role_admin(role(1), role(2));
        assert_eq!(ac.role_admin(role(1)), role(2));
    }

    #[test]
    fn access_control_default_admin_role_is_zero() {
        assert_eq!(DEFAULT_ADMIN_ROLE, [0u8; 32]);
    }

    #[test]
    fn access_control_grant_idempotent() {
        let ac = AccessControl::new(key(1))
            .grant_role(role(1), key(2))
            .grant_role(role(1), key(2)); // second grant is no-op
        let data = ac.role_data(role(1)).unwrap();
        assert_eq!(data.members.iter().filter(|&&k| k == key(2)).count(), 1);
    }

    #[test]
    fn access_control_serde_round_trip() {
        let ac = AccessControl::new(key(1))
            .grant_role(role(1), key(2))
            .grant_role(role(2), key(3));
        let json = serde_json::to_string(&ac).unwrap();
        let back: AccessControl = serde_json::from_str(&json).unwrap();
        assert_eq!(ac, back);
    }
}
