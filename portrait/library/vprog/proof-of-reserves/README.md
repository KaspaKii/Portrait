# ProofOfReserves

**A cross-layer (vProg) solvency-attestation pattern — Portrait vProg catalogue #2.**

*Pre-production, unaudited, testnet-only. Perishable evidence. Local-only —
nothing settled to kaspanet.*

> **STATUS — emit-verified template (M2 skeleton).** The covenant compiles via
> `silverc`; `portrait atelier-build` emits a RISC Zero guest **SKELETON** that
> builds the 104-byte journal and advances `seq`. The actual predicate — what the
> guest proves (here, `sum(reserves) >= sum(liabilities)` over the private account
> set) — is **authored by the developer into the guest, NOT synthesised by the
> compiler**. The emitted guest reads the proposed verdict + accounts root but
> does not itself fold the balances. **Not settled live** — only `CsciInstrument`
> has settled live on TN10.

A custodian periodically attests that it is solvent — `sum(reserves) >=
sum(liabilities)` — over a **private** set of account balances. Summation over a
large private set is infeasible on chain, so the rule lives in a RISC Zero
zk-STARK guest (the vProg); the covenant records a monotonic-seq attestation
transition **bound to that proof**.

This is the second instance of the proven CSCI cross-layer shape (the first being
`../../state/CsciInstrument`, which settled live on TN10 with the STARK verified
in consensus). It is built **source-only** inside that proven shape.

## The pair

| Layer | Role | What it does |
|---|---|---|
| **L1 covenant** | `attest` (`#[covenant]`) | Records one attestation transition on chain; self-enforces the structural state machine + the covenant-id binding. |
| **vProg guest** | `solvency_rule` (NonCovenant) | The off-chain predicate proven by the STARK; folds the private balances, asserts solvency, commits epoch + verdict + accounts root into the journal's `new_state_hash`. |

Portrait emits both from this one source:
- `portrait engrave ProofOfReserves.portrait` → `ProofOfReserves.sil` (silverc exit 0)
- `portrait atelier-build ProofOfReserves.portrait` → `proofofreserves_guest_main.rs`

## What the guest proves

`sum(reserves) >= sum(liabilities)` over a private per-account balance set. The
journal commits the attested `epoch`, the boolean `solvent` verdict, and a Merkle
root of the included accounts (`accounts_root`) inside the 104-byte CSCI journal:

```text
covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]
```

## What the covenant enforces (on chain)

| Rule | Enforced by |
|---|---|
| Monotonic reporting epoch (`next.epoch == prev.epoch + 1`) | `epoch + 1` in the transition |
| Monotonic CSCI seq (`next.seq == prev.seq + 1`) | `seq + 1` in the transition |
| Covenant-id binding (journal's covenant_id == this covenant) | `require(proof_cov_id == OpInputCovenantId(0))` — emitted because the role has a vProg companion (`has_vprog`) |
| Attestor authorization | `require(checkSig(auth, prev_states[0].attestor))` against the committed attestor key |
| Singleton, no fork / no burn | covenant transition mode (structural) |

## Files

- `ProofOfReserves.portrait` — the Portrait source (what a builder writes).
- `ProofOfReserves.sil` — the emitted silverscript (verify against the real silverc).
- `proofofreserves_guest_main.rs` — the emitted RISC Zero guest main (the vProg template).
- `README.md` — this file.

## Honest scope

- **Emit-verified template, not settled live.** The covenant compiles via silverc
  (exit 0) and the guest is emitted as well-formed Rust. Live settlement of this
  pattern reuses the *same* proven CSCI harness
  (`examples/portrait-settlement`); only CSCI itself has been settled live.
- **The covenant-side type checks are structural/relational (no SMT).** The
  *soundness of the solvency computation* rests on the RISC Zero guest + the
  tag-0x21 verifier, **not** on Portrait. Portrait carries the structural
  attestation state machine on chain.
- **The tag-0x21 STARK verification is an engine-level operation**, not a
  silverscript op. The settlement harness feeds the journal's covenant_id in as
  `proof_cov_id`; a single redeem that both verifies the STARK and binds its
  journal would need an upstream opcode that is **deliberately not pursued**.
- The guest's body-lowering (atelier M2) advances `epoch` from the scalar return;
  the verdict + accounts root are committed via the encoded state and the
  application sum-comparison over private balances is authored into the guest
  template. This is the catalogue template, not a fully synthesised prover.

## Red team (per Kii standing rule — resolve/flag CRITICAL & HIGH before final)

| Sev | Finding | Mitigation |
|---|---|---|
| **HIGH** | **Attestor-key compromise forges solvency.** A single `attestor` pubkey is the whole trust root; stealing it lets an attacker attest `solvent = true` with any root. | Ship an M-of-N attestor variant + key rotation as part of the family; do not rely on the single-key form for high-value reserves. |
| **HIGH** | **Solvency soundness is entirely off-chain.** If the guest's balance set is incomplete, double-counts, or the Merkle root is not binding, the on-chain attestation is vacuous. | The *guest construction* (account-set membership, conservation, domain-separated commitment) matters as much as the covenant. Make it part of the Hallmark scope. |
| **MEDIUM** | **`accounts_root` membership is asserted, not proven inclusive.** Nothing on chain proves the root covers *all* liabilities. | Bind to an independently published liability commitment (e.g. an exchange's signed customer-liability root) and prove the reserve root dominates it. |
| **MEDIUM** | **Epoch is self-asserted, not clock-bound.** Monotonic epoch prevents replay but not stale attestations presented as current. | Bind `epoch` to DAA score / a published period oracle for true time anchoring. |
| **LOW** | Cross-instrument replay of a proof. | Prevented by the KIP-20 covenant-id binding (`proof_cov_id == OpInputCovenantId(0)`); the CSCI live run demonstrated the negative control. |

**Recommended form:** use the *multi-attestor, clock-bound, with a
proven-inclusive liability root* form for high-value use. The bare single-key form
ships as a teaching reference only.
