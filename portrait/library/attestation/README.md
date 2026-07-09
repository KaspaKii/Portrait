# EvidenceLineage

**The canonical covenant-anchored attestation chain — Portrait library component #1.**

This is not a demo. It is the factored-out form of a covenant pattern the Kii
federation already hand-rolls in at least five production codebases:

| Repo | Domain | The same pattern, spelled differently |
|---|---|---|
| **SCL** | CMMC / DFARS defense compliance | `I-1..I-5` invariants, Poseidon commitment, lineage UTXO |
| **PLA** | Post-trade reconciliation | identical `I-1..I-5`, domain-separated Poseidon tag |
| **Kastract** | FIDIC construction contracts | IPA → IPC → payment lineage, KIP-20 covenant-ID walk |
| **KiiWORKS** | Digital Product Passports | DPP lifecycle anchoring + selective disclosure |
| **AssetMint** | RWA identity / claims | claims transfer lineage under a clawback covenant |

Each writes the same thing by hand. `EvidenceLineage` is the abstraction. It is
a strong reference component: the most-reused, smallest, most-verifiable
surface in the whole estate, and the one whose correctness underwrites the
others.

## What it is

One spend = one attestation. The chain is a sequence of singleton-covenant
UTXOs. Each successor commits to an off-chain *canonical record* under a hiding
commitment (`commit`), carries the bound `subject` forward, and increments a
monotonic `seq`. Verifiers check artifacts against the on-chain commitment
without ever seeing the underlying data on the public ledger.

## The five invariants

| ID | Rule | Enforced by |
|---|---|---|
| **I-1** | `next.seq == prev.seq + 1` | `seq + 1` in the transition |
| **I-2** | `next.subject == prev.subject` | `subject` carried unchanged |
| **I-3** | well-formed payload; `event_class` in range | `require(next_class >= 0)` (+ range in domain refinement) |
| **I-4** | `prev.t_bucket ≤ next.t_bucket ≤ prev.t_bucket + window` | two `require`s |
| **I-5** | exactly one successor UTXO, dust carried, no fork / no burn | singleton-transition covenant mode (structural) |

## Files

- `EvidenceLineage.portrait` — the Portrait source (what a builder writes).
- `EvidenceLineage.sil` — the intended silverscript emission (verify against the real compiler).
- `README.md` — this file.

## Red team (per Kii standing rule — resolve/flag CRITICAL & HIGH before final)

| Sev | Finding | Mitigation |
|---|---|---|
| **HIGH** | **Issuer-key compromise forges the chain.** A single `issuer` pubkey is the whole trust root; stealing it lets an attacker extend or rewrite forward. | Ship a `MultiSigIssuer` variant (M-of-N) and a key-rotation entrypoint as part of the component family. SCL's FFRDC-signed genesis is the reference. Do **not** rely on the single-key form for high-value lineages. |
| **HIGH** | **Commitment soundness is off-chain.** If `commit` is not binding + hiding (wrong construction, no domain separation, reused blinds), the on-chain guarantees are vacuous. | The *commitment construction* matters as much as the covenant — use domain-separated Poseidon/blake2b, per-record blind, as SCL/PLA already do. Make the construction part of the Hallmark scope. |
| **MEDIUM** | **`t_bucket` is self-asserted**, not a real clock; the temporal envelope bounds drift, not absolute time. | For lineages needing true time, bind to DAA score / `this.age` instead of a passed bucket. Offer a `ClockBound` variant. |
| **MEDIUM** | **Genesis trust is external.** Nothing on-chain proves `subject` was legitimately bound at genesis. | Require a genesis-authority signature (FFRDC / LEI authority / GLEIF), enforced in a separate `genesis` entrypoint. Document the off-chain trust assumption explicitly. |
| **LOW** | Cross-lineage replay. | Prevented by the KIP-20 covenant-ID; a dedicated test asserts the lineage id is load-bearing. |

**Recommended form:** use the *multisig, clock-bound, genesis-authorised* form for anything of value. The bare single-key form ships as a teaching reference only. Be conservative here — a single drained lineage would undermine trust in the pattern.
