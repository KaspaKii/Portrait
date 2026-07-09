# Governance covenants

> **Maturity:** pre-production, unaudited, testnet-only — perishable evidence.
> Covenant type-checks are structural/relational (no SMT). `value_conserved` is
> N-field additive-delta cancellation, not an SMT proof. `multisig_threshold` is
> a structural count of distinct committed keys signed, **not** a proof that the
> boolean combination is a true k-of-n threshold.

The governance family holds covenants that govern **authority and treasury
spend** rather than arbitrary application state. Both covenants in this family
authorise every state-mutating transition against keys **committed in state at
genesis**, never against caller-supplied pubkeys (the C2 capability pattern).
Each covenant emits exactly one `.sil`. Neither is a vProg covenant (no guest
program); both type-check with the structural/relational checker only.

Source: `library/governance/` in the Portrait repo.

## SocialRecovery

Source: `library/governance/social-recovery/SocialRecovery.portrait` (emits one
`.sil`: `SocialRecovery.sil`).

**Purpose.** A guardian-based 2-of-3 account-recovery covenant: the `owner` key
can be rotated without the owner's participation when any two of three committed
guardians cooperate — the canonical "lost my key" recovery flow. It rotates
*authority*, not funds; there is no value field.

**State** (role `account`):

- `owner: pubkey` — current account owner (rotated by recovery)
- `guardian_a: pubkey` — committed guardian 1 (recovery authority)
- `guardian_b: pubkey` — committed guardian 2 (recovery authority)
- `guardian_c: pubkey` — committed guardian 3 (recovery authority)
- `pending_owner: pubkey` — nominee staged by a proposal (genesis: placeholder)
- `recovering: int` — arm/disarm flag (0 = idle, 1 = recovery armed; genesis = 0)

**Transitions:**

- **`propose_recovery(sig auth_x, sig auth_y, pubkey new_owner)`** — any 2-of-3
  guardians nominate a `pending_owner` and arm the recovery. Guards:
  - `requires checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_b) || checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_c) || checkSig(auth_x, guardian_b) && checkSig(auth_y, guardian_c);`
  - `requires recovering == 0;` (cannot re-arm an in-flight recovery)
  - Updates: `pending_owner: new_owner`, `recovering: 1`; `owner` and the three
    guardian keys carried unchanged.
- **`finalize(sig auth_x, sig auth_y)`** — any 2-of-3 guardians (same threshold)
  confirm the staged rotation. Guards:
  - `requires checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_b) || checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_c) || checkSig(auth_x, guardian_b) && checkSig(auth_y, guardian_c);`
  - `requires recovering == 1;` (cannot finalize what was never proposed)
  - Updates: `owner: pending_owner` (rotate to the staged nominee),
    `recovering: 0` (disarm); guardian keys and `pending_owner` carried.

Lifecycle: `live -> live via account.propose_recovery;` and
`live -> live via account.finalize;`.

**Invariants** (declared): `authorized`, `multisig_threshold`,
`no_undeclared_state`.

**Honest scope.** `multisig_threshold` is a structural count of ≥2 distinct
committed guardian-key `checkSig` operands, not a proof the boolean combination
is a true k-of-n threshold; the covenant rotates authority only and reads no UTXO
values.

## MultisigTreasury

Source: `library/governance/treasury/MultisigTreasury.portrait` (emits one
`.sil`: `MultisigTreasury.sil`).

**Purpose.** A 2-of-2 multisignature treasury covenant: a treasury UTXO whose
`balance` may only be moved when **both** committed signers authorise the spend
in the same transaction. Spend authority is checked against the committed signer
keys, never caller-supplied pubkeys.

**State** (role `treasury`):

- `signer_a: pubkey` — first committed signer (spend authority)
- `signer_b: pubkey` — second committed signer (spend authority)
- `balance: int` — treasury balance (value-conserved by field name)

**Transitions:**

- **`spend(sig auth_a, sig auth_b, int amount)`** — move `amount` out of the
  treasury; requires both committed signers (2-of-2). Guards:
  - `requires checkSig(auth_a, signer_a);` (signer A authorises, committed key)
  - `requires checkSig(auth_b, signer_b);` (signer B authorises, committed key)
  - `requires amount >= 0;` (non-negative spend)
  - `requires amount <= balance;` (cannot overspend the treasury)
  - Updates: `balance: balance - amount` (single additive subtraction); both
    signer keys carried unchanged.

Lifecycle: `live -> live via treasury.spend;`.

**Invariants** (declared): `value_conserved`, `authorized`, `non_negative_amount`,
`multisig_threshold`, `no_undeclared_state`.

**Honest scope.** `value_conserved` is N-field additive-delta cancellation
(single subtraction of `amount` from `balance`), not an SMT proof;
`multisig_threshold` is a structural count of the two committed-key `checkSig`
operands (2-of-2, an n-of-n conjunction), not a true k-of-n threshold proof.
`amount` is caller-asserted and conserved against committed state — the covenant
does not itself read UTXO coin values; actual coin movement is the wallet's
responsibility.
