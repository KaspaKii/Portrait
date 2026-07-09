# Attestation covenants

> Maturity: pre-production, unaudited, testnet-only, perishable evidence. Covenant
> type-checks here are structural/relational (no SMT). The covenants in this family
> are emitted to silverscript and accepted by `silverc`; they have not been settled
> on-chain. Derived from the actual `.portrait` sources in
> `portrait/library/attestation/`.

The attestation family covers covenant-anchored evidence chains: each spend records
one attestation, carrying a bound subject forward under a hiding commitment. The
covenant enforces the structural invariants only — the soundness of the off-chain
commitment scheme is an external responsibility, not something this layer proves.

This family currently contains one covenant. It is a plain transition covenant
(not a vProg pattern), so there are no `_guest_main.rs` guests and no
EMIT-VERIFIED predicate stubs. It emits a single `.sil`
(`EvidenceLineage.sil`).

## EvidenceLineage

**Purpose.** The canonical covenant-anchored attestation chain — the factored-out
form of the I-1..I-5 lineage pattern (SCL, PLA, Kastract, KiiWORKS, AssetMint).
One spend = one attestation: each successor UTXO commits to an off-chain canonical
record under a hiding commitment and carries the bound subject forward. The
covenant enforces structural invariants; the commitment scheme's soundness is an
off-chain responsibility.

**State** (each field initialised from the constructor param in the same position;
genesis state is declared first in the constructor, then the policy params):

- `seq: int` — monotonic sequence number (genesis = 0)
- `subject: bytes32` — bound identity (CAGE / LEI / entity hash)
- `commit: bytes32` — hiding commitment of the canonical record
- `t_bucket: int` — coarse timestamp bucket
- `event_class: int` — schema / event discriminator

Policy params carried in the covenant body (not used for state init):
`issuer: pubkey` (trust root permitted to extend the chain) and
`window: int` (max seconds between attestations — the temporal envelope).

**Transitions.**

- `attest(auth: sig, next_commit: bytes32, next_class: int, next_t_bucket: int)` —
  extends the chain in place (`live -> live`). Guards:
  - `requires checkSig(auth, issuer);` — authorised extender
  - `requires next_class >= 0;` — I-3 schema envelope
  - `requires next_t_bucket >= t_bucket;` — I-4 monotonic time
  - `requires next_t_bucket <= t_bucket + window;` — I-4 temporal envelope

  Field updates in the returned state:
  - `seq: seq + 1` — I-1 monotonic sequence
  - `subject: subject` — I-2 identity binding (carried unchanged)
  - `commit: next_commit`
  - `t_bucket: next_t_bucket`
  - `event_class: next_class`

**Invariants** (declared in the source):

- `invariant value_conserved;`
- `invariant no_undeclared_state;`

Additionally, I-5 (exactly one successor UTXO; dust carried; no fork / no burn) is
noted in the source as enforced *structurally* by the singleton-transition covenant
mode rather than by a declared `invariant` line; I-1 is enforced by `seq + 1` and
I-2 by carrying `subject` unchanged.

**Honest scope.** Structural transition covenant only; the hiding-commitment scheme's
soundness is off-chain, not proven by the covenant — pre-production,
unaudited, testnet-only.
