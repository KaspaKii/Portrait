# governance/SocialRecovery

A guardian-based 2-of-3 account-recovery covenant. An account UTXO is owned by
`owner`, but the owner key can be **rotated without the owner's participation**
when any two of three committed guardians cooperate — the canonical "lost my key"
recovery flow. A two-step propose-then-finalize shape mirrors real social-recovery
wallets: a proposal arms a `pending_owner` that a confirming 2-of-3 quorum must
then ratify, so a single malformed proposal cannot silently hand over the account.

**Status:** 🟡 drafted — pre-red-team, testnet-only, not audited, not mainnet-safe.

## Parameters / State

One constructor param per state field, in field order:

| Field | Type | Meaning |
|---|---|---|
| `owner` | `pubkey` | Current account owner (rotated by recovery). |
| `guardian_a` | `pubkey` | Committed guardian 1 (recovery authority). |
| `guardian_b` | `pubkey` | Committed guardian 2 (recovery authority). |
| `guardian_c` | `pubkey` | Committed guardian 3 (recovery authority). |
| `pending_owner` | `pubkey` | Nominee staged by a proposal. |
| `recovering` | `int` | Arm/disarm flag (0 = idle, 1 = armed). |

## Lifecycle

```
idle      --propose_recovery(2-of-3 guardianSigs, new_owner)  [recovering==0] --> armed  (pending_owner := new_owner; recovering := 1)
armed     --finalize(2-of-3 guardianSigs)                     [recovering==1] --> idle   (owner := pending_owner; recovering := 0)
```

## Why it's safe by shape

- **Committed-key authorisation (C2).** Every `checkSig` authorises against a
  COMMITTED guardian key carried in state, never a caller-supplied pubkey. An
  attacker holding one guardian key cannot satisfy any of the three conjunctive
  pairs. The `owner` never signs a recovery — that is the point.
- **Two-step ratification.** `finalize` re-checks the 2-of-3 quorum before the
  owner key is rotated, so a single proposal cannot enact a takeover on its own.

## Authoring convention: the 2-of-3 quorum (&& / || precedence)

The 2-of-3 quorum is expressed as a **disjunction of the three conjunctive
guardian pairs**, written without parentheses:

```
requires checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_b)
      || checkSig(auth_x, guardian_a) && checkSig(auth_y, guardian_c)
      || checkSig(auth_x, guardian_b) && checkSig(auth_y, guardian_c);
```

This relies on the authoring convention that `&&` binds tighter than `||` in
both the Portrait Pratt parser and silverscript, so it groups unambiguously as
`(a&&b) || (a&&c) || (b&&c)`. This is the same convention `ArbiterEscrow` uses for
its buyer/seller/arbiter 2-of-3. Note: no `require` may begin with `(`, since a
leading paren is read as the optional require-wrapper.

## Honest scope

- **Authority, not value.** This covenant rotates a key; it moves no coin, so no
  value invariant applies (cf. EvidenceLineage / CsciInstrument).
- **Guardian-key custody is the trust assumption.** Two cooperating guardians can
  rotate the owner key by design; the security rests on guardians being chosen and
  kept independent.
- **Semantic checks are structural/relational, not an SMT solver** (per-field, no
  cross-field flow proof).
- Pre-production, unaudited, testnet-only.

## Files

- `SocialRecovery.portrait` — the canonical covenant source. `portrait engrave`
  lowers it to `.sil` + CTOR JSON that `silverc` accepts (exit 0).
- `SocialRecovery.sil` — the emitted Silverscript component.
- `SocialRecovery_ctor.json` — the emitted CTOR JSON consumed by `silverc --ctor`.
- `SocialRecovery.json` — the `silverc`-compiled script.

## Reproduce

```sh
cd portrait
cargo run --bin portrait -- check   ../library/governance/social-recovery/SocialRecovery.portrait
cargo run --bin portrait -- engrave ../library/governance/social-recovery/SocialRecovery.portrait
```
