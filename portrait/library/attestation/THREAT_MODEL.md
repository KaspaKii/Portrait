# THREAT_MODEL — EvidenceLineage

Scope: the `EvidenceLineage` covenant emitted from `EvidenceLineage.portrait`
to `EvidenceLineage.sil` and accepted by `silverc` (exit 0). This document
covers ONLY what the on-chain covenant enforces. It is deliberately narrow:
claims about off-chain record integrity are out of scope and called out below.

Status: pre-production, unaudited, testnet-only. No on-chain (TN10) transaction
has been produced for this covenant; the only verified property here is that the
emitted script compiles. Anything beyond compilation is [UNVERIFIED].

## Assets

- **The lineage UTXO** — the single covenant-bound output that carries the
  attestation chain forward. Its spend authority is the protected asset.
- **`subject`** — the bound identity (CAGE / LEI / entity hash). Its continuity
  across the chain is the integrity property adopters rely on.
- **`commit`** — a hiding commitment to the off-chain canonical record for each
  attestation. The covenant treats it as an opaque 32-byte value.
- **`seq`** — the monotonic position of each attestation in the chain.

## Trust assumptions

- **Issuer key custody.** `issuer` is a single pubkey. Whoever holds the issuer
  private key can extend the chain arbitrarily (within the structural rules
  below). Key compromise = chain compromise. No multisig / rotation in this
  version.
- **Commitment soundness is OFF-CHAIN.** The covenant does not verify that
  `commit` actually binds to any record, nor that successive records are
  consistent. Binding + hiding are the responsibility of the off-chain
  commitment scheme (an off-chain responsibility, outside the covenant). A dishonest issuer
  can commit to a record that says anything; the covenant only enforces that
  *some* 32-byte value was carried.
- **Time buckets are caller-supplied.** `next_t_bucket` is an argument, not a
  consensus clock reading. The covenant enforces ordering/envelope on the value
  the spender provides; it does not prove the value reflects real elapsed time.
- **Engine pin.** Enforcement assumes the `rusty-kaspa` v2.0.0 (90dbf07)
  covenant semantics for singleton transitions and `checkSig`.

## Threats and the covenant's response (reject vectors)

The emitted `attest` transition REJECTS a spend when any of these hold:

1. **Forged extension (no issuer authority).**
   `require(checkSig(auth, issuer))` — a spend whose signature does not verify
   against `issuer` is rejected. An attacker without the issuer key cannot
   extend the chain.
2. **Out-of-range schema / event class.**
   `require(next_class >= 0)` — a negative `event_class` (malformed schema
   discriminator) is rejected (invariant I-3).
3. **Time going backwards.**
   `require(next_t_bucket >= prev.t_bucket)` — an attestation that claims an
   earlier time bucket than its predecessor is rejected (invariant I-4,
   monotonic time).
4. **Time jumping past the temporal envelope.**
   `require(next_t_bucket <= prev.t_bucket + window)` — an attestation that
   skips more than `window` buckets ahead is rejected (invariant I-4, envelope).
5. **Sequence tampering / non-monotonic seq.**
   The return object fixes `seq = prev.seq + 1`; the spender cannot set an
   arbitrary `seq`. A successor with any other sequence value is not a valid
   output of this transition (invariant I-1).
6. **Identity substitution.**
   The return object fixes `subject = prev.subject`; the spender cannot rebind
   the chain to a different identity (invariant I-2).
7. **Fork / burn / multi-successor (I-5).**
   The singleton-transition covenant mode (`from = max_ins, to = 1`) structurally
   permits exactly one successor UTXO, preventing the chain from forking into two
   competing successors or being silently burned. [UNVERIFIED on-chain — relies
   on engine covenant-binding semantics; confirm against TN10 before relying on
   it.]

## Explicit non-goals (NOT defended)

- **No off-chain record integrity.** See trust assumptions — the covenant cannot
  detect a dishonest or inconsistent committed record.
- **No replay across distinct chains.** Two independently deployed EvidenceLineage
  covenants with the same params are indistinguishable to this document; binding
  to a specific covenant instance is the deployer's responsibility (genesis tx /
  covenant id).
- **No issuer-key rotation or revocation.** A compromised issuer key cannot be
  rotated within this covenant.
- **No value-conservation proof here.** `invariant value_conserved` is declared
  in the portrait source but is enforced by the engine's covenant value rules,
  not re-checked in the emitted script; treat as [UNVERIFIED] until exercised on
  TN10.
