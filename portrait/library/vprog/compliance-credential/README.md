# ComplianceCredential

**A cross-layer (vProg) privacy-preserving compliance pattern — Portrait vProg catalogue #1.**

*Pre-production, unaudited, testnet-only. Perishable evidence. Local-only —
nothing settled to kaspanet.*

> **STATUS — emit-verified template (M2 skeleton).** The covenant compiles via
> `silverc`; `portrait atelier-build` emits a RISC Zero guest **SKELETON** that
> builds the 104-byte journal and advances `seq`. The actual predicate — what the
> guest proves (here, the compliance check over the private credential) — is
> **authored by the developer into the guest, NOT synthesised by the compiler**.
> The emitted guest reads the proposed verdict but does not itself recompute it.
> **Not settled live** — only `CsciInstrument` has settled live on TN10.

A holder proves they satisfy a compliance predicate — `attribute >= threshold`,
`jurisdiction ∈ allowed set`, `not-on-a-list` — **without revealing the
credential**. The predicate is over private data the chain must not see, so the
rule lives in a RISC Zero zk-STARK guest (the vProg); the covenant settles a
"verified" transition **bound to that proof**, recording only a credential
*commitment* and a boolean *verdict*.

This is an instance of the proven CSCI cross-layer shape (`../../state/CsciInstrument`,
which settled live on TN10 with the STARK verified in consensus). It is built
**source-only** inside that proven shape.

## The pair

| Layer | Role | What it does |
|---|---|---|
| **L1 covenant** | `settle` (`#[covenant]`) | Settles one verified compliance transition on chain; self-enforces the structural state machine + the covenant-id binding. Carries the credential `commitment` forward unchanged so the chain never sees the credential. |
| **vProg guest** | `predicate` (NonCovenant) | The off-chain compliance predicate proven by the STARK; evaluates the predicate over the private credential and commits only the commitment + verdict into the journal's `new_state_hash`. |

Portrait emits both from this one source:
- `portrait engrave ComplianceCredential.portrait` → `ComplianceCredential.sil` (silverc exit 0)
- `portrait atelier-build ComplianceCredential.portrait` → `compliancecredential_guest_main.rs`

## What the guest proves

The holder satisfies the compliance predicate over a **private** credential. Only
a credential `commitment` and the boolean `verdict` enter the 104-byte CSCI
journal — the credential plaintext never does:

```text
covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]
```

`new_state_hash` commits the (private) credential commitment + verdict + seq.

## What the covenant enforces (on chain)

| Rule | Enforced by |
|---|---|
| Monotonic CSCI seq (`next.seq == prev.seq + 1`) | `seq + 1` in the transition (no replay / skip) |
| Covenant-id binding (journal's covenant_id == this covenant) | `require(proof_cov_id == OpInputCovenantId(0))` — emitted because the role has a vProg companion (`has_vprog`) |
| Owner authorization | `require(checkSig(auth, prev_states[0].owner))` against the committed owner key (never a caller arg) |
| Credential stays private (commitment continuity) | `commitment` carried from its own prior value; the chain only ever holds the commitment |
| Singleton, no fork / no burn | covenant transition mode (structural) |

## Files

- `ComplianceCredential.portrait` — the Portrait source (what a builder writes).
- `ComplianceCredential.sil` — the emitted silverscript (verify against the real silverc).
- `compliancecredential_guest_main.rs` — the emitted RISC Zero guest main (the vProg template).
- `README.md` — this file.

## Honest scope

- **Emit-verified template, not settled live.** The covenant compiles via silverc
  (exit 0) and the guest is emitted as well-formed Rust. Live settlement of this
  pattern reuses the *same* proven CSCI harness
  (`examples/portrait-settlement`); only CSCI itself has been settled live.
- **The covenant-side type checks are structural/relational (no SMT).** The
  *soundness of the predicate evaluation* rests on the RISC Zero guest + the
  tag-0x21 verifier, **not** on Portrait. Portrait carries the structural
  compliance state machine on chain.
- **The tag-0x21 STARK verification is an engine-level operation**, not a
  silverscript op. The settlement harness feeds the journal's covenant_id in as
  `proof_cov_id`; a single redeem that both verifies the STARK and binds its
  journal would need an upstream opcode that is **deliberately not pursued**.
- The guest's body-lowering (atelier M2) advances `seq` from the scalar return;
  the commitment + verdict are committed via the encoded state and the application
  predicate over the private credential is authored into the guest template. This
  is the catalogue template, not a fully synthesised prover.

## Red team (per Kii standing rule — resolve/flag CRITICAL & HIGH before final)

| Sev | Finding | Mitigation |
|---|---|---|
| **HIGH** | **Predicate soundness is entirely off-chain.** If the guest binds the wrong commitment, evaluates a weakened predicate, or the commitment is not collision-resistant, the on-chain "verified" transition is vacuous. | The *guest construction* (domain-separated commitment, predicate fidelity, binding the verdict to the committed credential) matters as much as the covenant. Make it part of the Hallmark scope. |
| **HIGH** | **Stale-credential acceptance.** A verdict proven against a credential that was later revoked still settles, because nothing on chain checks freshness. | Bind the predicate to a revocation-list root / issuer epoch and prove non-revocation as part of the guest; do not rely on a form without a freshness binding for regulated use. |
| **MEDIUM** | **Issuer trust root is implicit.** The pattern proves *the holder satisfies a predicate over a commitment*; it does not prove the credential was issued by a trusted authority. | Have the guest verify an issuer signature over the credential and commit the issuer id, so the verdict means "a trusted issuer's credential satisfies the predicate". |
| **MEDIUM** | **Owner-key compromise re-settles.** A single `owner` pubkey authorizes every transition; stealing it lets an attacker advance state (though not forge the off-chain verdict). | Offer an M-of-N owner variant + key rotation in the family. |
| **LOW** | Cross-instrument replay of a proof. | Prevented by the covenant-id binding (`proof_cov_id == OpInputCovenantId(0)`); the CSCI live run demonstrated the negative control. |

**Recommended form:** use the *issuer-bound, revocation-checked*
form for regulated use. The bare predicate-over-commitment form ships as a
teaching reference only.
