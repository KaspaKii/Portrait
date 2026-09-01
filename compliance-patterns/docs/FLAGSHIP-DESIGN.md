# FLAGSHIP-DESIGN — Kii Covenant-Settled Compliance Instrument (CSCI)

**Status:** Design draft — pre-production, unaudited, testnet-only.
Stichting Kii Foundation. MIT licence.
Engine pin: rusty-kaspa v2.0.0 (`90dbf07`). Do not bump.
Stichting Kii Foundation. 2026-06-27.

---

## 1. Instrument Name and Description

**Instrument: Covenant-Settled Compliance Instrument (CSCI)**

A CSCI is a KTT-denominated financial instrument whose lifecycle — issuance,
compliance checks, and transfer — is split across two layers in the way the
Kaspa two-layer model intends: a silverscript covenant on L1 enforces value and
custody rules unconditionally; a vProg running on the Kii-compatible vProgs
runtime computes the compliance logic (eligibility, limits, rule predicates);
and the vProg's state transition is settled on L1 via a RISC Zero succinct STARK
posted as a KIP-16 tag-0x21 payload. The covenant binds to the settled vProg
state via the KIP-20 covenant ID mechanism — it will not release value unless the
ZK-settled journal matches the expected transition hash.

**Architecture summary:** the CSCI combines (a) covenant-enforced custody
on Kaspa L1, (b) off-chain vProg computation settled by a RISC Zero succinct
STARK via KIP-16 tag-0x21 (PQ-safe, no pairings), and (c) covenant-to-vProg
binding via KIP-20 covenant IDs.

---

## 2. Architecture Diagram

```
CSCI — end-to-end architecture (TN10 PoC target)

  ┌─────────────────────────────────────────────────────────────┐
  │  OFF-CHAIN (operator / compliance engine)                   │
  │                                                             │
  │  Input: transfer intent                                     │
  │    - from_owner : [u8;32]  (KTT owner_identifier)          │
  │    - to_owner   : [u8;32]  (recipient identifier)          │
  │    - amount     : u64                                       │
  │    - rule_hash  : [u8;32]  (sha256 of the rule set)        │
  │    - seq        : u64      (monotonic transition counter)   │
  │                                                             │
  │         │                                                   │
  │         ▼                                                   │
  │  [vProg guest — RISC Zero zkVM]                             │
  │    - reads: current KTT state (owner, amount, rules)        │
  │    - evaluates: eligibility predicate(s) against rule_hash  │
  │    - asserts: amount <= limit, recipient eligible           │
  │    - computes: new KTT state (post-transfer fields)         │
  │    - commits to journal:                                    │
  │        covenant_id (32 B) || new_state_hash (32 B) ||       │
  │        rule_hash   (32 B) || seq             (8 B LE)       │
  │         │                                                   │
  │         ▼                                                   │
  │  RISC Zero prove() → composite receipt                      │
  │    → compress(ProverOpts::succinct())                       │
  │    → succinct STARK seal (~222–223 KB)                      │
  │                                                             │
  └──────────────────────────────┬──────────────────────────────┘
                                 │  proof fields:
                                 │  seal, journal_hash, image_id,
                                 │  control_id, control_digests,
                                 │  control_index, claim
                                 │  (hashfn = Poseidon2 = OP_1)
                                 │
  ┌──────────────────────────────▼──────────────────────────────┐
  │  KASPA L1 — settlement transaction (TN10)                   │
  │                                                             │
  │  Unlocking script pushes (in order):                        │
  │    [1] journal_bytes (104 B, clear) — so covenant can read  │
  │    [2] claim | control_index | control_digests | seal |     │
  │        sha256(journal_bytes) | image_id | control_id |      │
  │        OP_1 (hashfn=Poseidon2) | 0x21 | OP_0               │
  │    → OpZkPrecompile(0xa6) pops 8 proof fields,              │
  │        verifies STARK, pushes true/false                    │
  │    → OP_VERIFY: abort if ZK check failed                    │
  │                                                             │
  │         │  ZK verified — journal_bytes still on stack       │
  │         ▼                                                   │
  │  [Covenant locking script — state-continuity checks]        │
  │    - OP_SHA256(journal_bytes) → computed_hash               │
  │    - asserts computed_hash == sha256 arg used in ZK call    │
  │      (binding clear journal to the proof)                   │
  │    - reads journal_bytes[0..32] → assert == expected        │
  │      covenant_id                                            │
  │    - reads journal_bytes[32..64] → new_state_hash           │
  │    - reads journal_bytes[64..96] → assert == rule_hash      │
  │      constant committed at genesis                          │
  │    - reads journal_bytes[96..104] → assert seq = prev+1     │
  │    - inspects output UTXO state fields via SilverScript     │
  │      validateOutputState intrinsics (compiled to opcode     │
  │      sequence from a .sil source file — not yet authored)   │
  │    - asserts output state fields hash == new_state_hash     │
  │    - releases KTT UTXO to new_owner only if all hold        │
  │                                                             │
  │         │  covenant_id (KIP-20)                             │
  │         ▼                                                   │
  │  New KTT UTXO — gated by same covenant family               │
  │    owner_identifier = to_owner                              │
  │    amount           = transferred amount                    │
  │    is_minter        = false                                 │
  │    seq              = prev_seq + 1                          │
  │                                                             │
  └─────────────────────────────────────────────────────────────┘

Legend:
  vProg guest = off-chain RISC Zero guest program (Kii vProgs runtime)
  KIP-16 tag-0x21 = OpZkPrecompile RISC Zero succinct STARK verifier
                    NOTE: pushes true/false only; journal is NOT re-pushed.
                    journal_bytes must be passed in clear in the unlocking script.
  KIP-20 = covenant ID binding (genesis + continuation handoff)
  validateOutputState = SilverScript-compiled construct (not a raw opcode);
                        covenant locking script must be authored as a .sil file.
  sigOpCount = 255 (required for ~25 M script-unit STARK verification budget)
```

---

## 3. L1 Binding Design

### 3.1 Covenant ID (KIP-20)

The CSCI covenant is a state-continuity covenant: each UTXO in the chain carries
a `covenant_id` that was set at genesis and is enforced by KCC20 introspection
opcodes. Every spend must produce a successor output whose covenant state matches
what the covenant script expects. This is engine-enforced on rusty-kaspa v2.0.0
— not a soft assertion.

**`validateOutputState` is a SilverScript-compiled construct, not a raw opcode.**
It compiles to a sequence of raw introspection opcodes from a `.sil` source file.
The CSCI covenant locking script must be authored as such a `.sil` file and
compiled before the PoC genesis transaction can be created. This file does not
yet exist in this codebase. Authoring it is PoC step zero — a builder
prerequisite. The `reserve_covenant_live.rs` pattern in examples provides the
template.

The `covenant_id` is 32 bytes and is embedded in the vProg journal at byte
offset 0. The covenant locking script reads `journal_bytes` — passed in the
clear as the first item in the unlocking script — and checks
`journal_bytes[0..32] == expected_covenant_id`.

**Important engine constraint (rusty-kaspa v2.0.0):** `OpZkPrecompile` (0xa6)
pops all eight proof fields and pushes a boolean (`true`/`false`). It does NOT
re-push the `journal_hash`. The CSCI design handles this with a two-path structure:
1. The unlocking script includes `journal_bytes` (104 bytes) as a clear data push.
2. The covenant locking script independently computes `sha256(journal_bytes)` via
   `OP_SHA256` and checks it equals the `journal_hash` argument used in the ZK call
   (binding the clear-text journal to the proof). Only then does it read individual
   journal fields for the covenant_id / new_state_hash / rule_hash / seq checks.

A tampered `journal_bytes` will not match the sha256 the valid proof was made
against, so the sha256-binding step is consensus-enforced.

A proof whose journal references a different covenant ID fails on-chain because
both the ZK verifier (wrong journal hash for the seal) and the covenant's sha256
binding step (journal_bytes sha256 ≠ proof argument) reject it independently.

### 3.2 Journal Payload Schema

```
journal_bytes (104 bytes total):
  [0..32]  covenant_id     — the KIP-20 covenant ID for this CSCI instance
  [32..64] new_state_hash  — sha256(abi-encode(new KTT state fields))
  [64..96] rule_hash       — sha256(canonical rule set bytes)
  [96..104] seq            — u64 little-endian, monotonic transfer counter
```

`journal` field in the KIP-16 tag-0x21 script = `sha256(journal_bytes)` (32 B),
pre-computed off-chain by the operator before constructing the spending tx. This
will use the `kcp-pq-anchor` `JournalSpec` infrastructure. The CSCI will add a
`JournalSpec::CsciTransition` variant as part of the builder's implementation;
this variant does not yet exist (current variants: `PairedAttestation`,
`SealedLineage`, `TransferableRecord`, `Custom`).

### 3.3 State Hash Schema

The CSCI state encoding is a superset of `KttState::encode()` (defined in
`kcp-ktt-token/src/state.rs`, `STATE_LEN = 42`) with `seq` appended:

```
new_state_fields (fixed 50 bytes total):
  identifier_type  : u8         offset 0    (0x00 = pubkey, 0x01 = script-hash,
                                              0x02 = covenant-id)
  owner_identifier : [u8; 32]   offsets 1–32
  amount           : u64 LE     offsets 33–40   (8 bytes)
  is_minter        : u8         offset 41       (0x00 / 0x01)
  seq              : u64 LE     offsets 42–49   (8 bytes — CSCI extension)
```

`new_state_hash = sha256(new_state_fields)`. This is NOT EVM ABI-encoding
(which would produce 160 bytes via 32-byte slot padding). The field order
matches `KttState::encode()` exactly; `seq` is appended as 8 LE bytes.

The base `KttState::decode()` remains intact; the CSCI layer reads the first
42 bytes as the base KTT state and the remaining 8 bytes as `seq`.

The covenant script inspects the declared output's state fields via the
SilverScript `validateOutputState` intrinsics (compiled from a `.sil` source
file — see §3.1 note on `validateOutputState`), then independently computes
`sha256(new_state_fields)` and checks it equals `journal_bytes[32..64]`. Both
assertions must hold for the spend to be valid.

### 3.4 vProg Image ID

The RISC Zero `image_id` is a content-addressed hash of the guest ELF binary.
It is hardcoded as a constant in the Kii crate (following `kcp-pq-anchor`
convention) and committed to git. Changing the guest program changes the
`image_id`, breaking the on-chain check. The `image_id` is the root of the
guest's soundness: the covenant trusts computation only from the program whose
image was declared at genesis, for the lifetime of that covenant.

---

## 4. The PoC Shape

**Target network:** TN10 (rusty-kaspa v2.0.0 Toccata, covenants active).

**What must be demonstrable:**

A0. **Prerequisite: author and compile the CSCI covenant locking script** — write
   the `.sil` SilverScript source for the state-continuity covenant (journal
   sha256 binding + covenant_id check + new_state_hash check + rule_hash check +
   seq enforcement + output state validation). Compile to embedded script bytes.
   This step must complete before any of A–E.

A. **Genesis transaction** — create the CSCI UTXO: a KTT genesis carrier tx that
   establishes a covenant-id-bound KTT state (owner, amount, rule_hash, seq=0).
   Evidence: txid accepted on TN10, `is_accepted: true` from the REST API.

B. **Compliance vProg run** — off-chain: run the RISC Zero guest against a
   transfer intent (from_owner, to_owner, amount=50, valid rule set). Guest
   outputs a succinct STARK seal and journal. Log: guest execution time, seal
   size, journal bytes.

C. **Settlement transaction (happy path)** — spend the CSCI UTXO via the KIP-16
   tag-0x21 redeem script. The covenant checks: STARK valid + journal matches +
   state transition valid + seq increments. Evidence: txid accepted on TN10,
   `is_accepted: true`. The new CSCI UTXO (seq=1) is visible on-chain.

D. **Negative control** — see Section 5.

E. **Provenance file** — a `CSCI-PROVENANCE.json` to the `kii-ml-dsa` /
   `kcp-pq-anchor` standard: genesis txid, settlement txid, negative-control
   txid + rejection evidence, image_id constant, rule_hash, node version,
   verification date, verifier identity (independent verification pass), REST fetch
   receipts for each.

**Claims being proven by the PoC:**

1. A vProg can compute a compliance decision off-chain and commit its output to
   a RISC Zero journal.
2. The journal can be verified on Kaspa L1 via KIP-16 tag-0x21
   (OpZkPrecompile / succinct STARK) at the existing sigOpCount budget.
3. A silverscript covenant can be made to bind its spend path to the contents
   of a verified journal (covenant_id + state hash + rule_hash + seq).
4. Together, these three facts constitute a complete covenant-enforced +
   vProg-computed + ZK-settled instrument running on L1.

---

## 5. Negative Control

The negative control is a mandatory PoC deliverable — it is not optional.

**Tampered proof test:** construct a spending transaction identical to the happy
path, but substitute a journal where `rule_hash` does not match the rule set the
vProg actually ran against (i.e. claim a different, more permissive rule set was
applied). Because the STARK seal is bound to the original journal bytes by the
RISC Zero protocol, the seal does not verify against the tampered journal hash.

**Expected consensus outcome:** `OpZkPrecompile` (0xa6) with tag 0x21 rejects
the script execution — the STARK verifier fails, the script returns false, and
the transaction is invalid. The transaction will be rejected by every validating
node before it enters any block.

**Evidence required:** attempt to submit the tampered transaction to TN10; record
the node rejection (error response from the wRPC submit call, or confirm the txid
is not accepted via the REST API returning `is_accepted: false` or a 404). This
rejection receipt is the negative-control evidence — it must appear in the
provenance file.

**Additional negative control (covenant-side):** separately, construct a
spending transaction with a valid STARK proof but whose output state does not
match `journal[32..64]` (the new_state_hash the vProg committed to). The
covenant's `validateOutputState` check fails independently of the ZK path.
Record this rejection as a second negative control, distinguishing it from the
STARK-level rejection.

---

## 6. Threat Model

### What the covenant enforces (consensus-final)

- **Custody:** the KTT UTXO cannot be spent unless the KIP-16 tag-0x21 script
  succeeds. No off-chain override, no operator key escape hatch in the covenant
  itself.
- **State continuity:** the covenant enforces that the successor output's state
  fields match what the ZK journal committed to. An operator cannot post a valid
  proof but then route the value to a different recipient — the covenant checks
  the output independently.
- **Covenant identity:** `covenant_id` in the journal is checked by the script.
  A valid proof from a different CSCI instance (different covenant_id) is not
  accepted.
- **Sequence monotonicity:** `seq` must increment exactly by 1. Replay attacks
  (re-submitting a valid old proof) are rejected because the predecessor UTXO
  has already been spent.

### What the vProg enforces (ZK-settled, not consensus-native)

- **Rule logic:** the eligibility predicates, transfer limits, and KYC/AML
  conditions are evaluated inside the zkVM. The covenant trusts only that the
  proof is valid for the declared image_id — it does not re-execute the rule
  logic. A bug in the guest program produces an incorrect but validly proved
  result; the covenant cannot detect this.
- **Rule set identity:** `rule_hash` in the journal binds the proof to a
  specific rule set. The covenant checks that `rule_hash` in the journal matches
  the expected constant (set at genesis). An operator cannot silently swap to a
  more permissive rule set — they would need a new proof from the declared guest
  against the declared rule_hash.

### Trust boundary

```
                    CONSENSUS TRUST
                   (can't be overridden)
  ┌───────────────────────────────────────────────────────┐
  │  - proof is a valid RISC Zero STARK for image_id      │
  │  - sha256(journal_bytes) == hash in ZK call           │
  │  - journal_bytes encodes covenant_id + new_state_hash │
  │    + rule_hash + seq (all four fields enforced)       │
  │  - output state fields hash == new_state_hash         │
  │  - seq = prev + 1                                     │
  └───────────────────────────────────────────────────────┘

                    ZKVM TRUST
            (sound if guest is correct)
  ┌───────────────────────────────────────────────────────┐
  │  - rule predicate logic is correct                    │
  │  - input data (owner, amount, rule set) is authentic  │
  │  - the guest commits the right new_state_hash         │
  └───────────────────────────────────────────────────────┘

                    OPERATOR TRUST
               (off-chain, not attested)
  ┌───────────────────────────────────────────────────────┐
  │  - the rule_hash was the correct version at genesis   │
  │  - the guest ELF was reviewed before image_id commit  │
  │  - input data supplied to the prover is authentic     │
  └───────────────────────────────────────────────────────┘
```

**What can go wrong and where:**

- **Malicious guest:** if the operator deploys a guest program that approves
  ineligible transfers, the covenant accepts the resulting proofs. Mitigation:
  the `image_id` is hardcoded as a constant in the crate and committed to git;
  anyone can audit the guest ELF against the image_id. The CSCI PoC includes
  the guest source so the image_id can be independently reproduced.
- **Input data forgery:** the prover controls what data is fed to the guest.
  The covenant does not verify that `from_owner` was the actual UTXO owner in
  the prior state — it verifies the journal. If the prover feeds false input
  data, the proof is valid but the computation is wrong. This is the residual
  operator-trust exposure; it is honest to name it.
- **Rule set staleness:** if `rule_hash` was computed over an outdated rule set
  at genesis, every proof will apply the wrong rules. Updating the rule set
  requires a new covenant genesis (new image_id or new rule_hash constant).
- **vProgs standalone vs synchronous:** the PoC uses standalone-based ZK
  settlement — the vProg executes off-chain and settles its output on-chain via
  STARK. It does NOT use synchronous vProg composition (the core-team-gated path
  that would allow vProgs to compose in-protocol). The trust model for standalone
  settlement is as above; synchronous composition would shift more logic into
  consensus, but is not on the PoC critical path.
- **Testnet perishability:** TN10 testnets reset. All txids cited in provenance
  are evidence at a point in time; the examples must be re-run to refresh
  evidence. This is the same discipline as `kcp-pq-anchor`.

---

## 7. Pre-Production Disclaimers

**Maturity stamp — mandatory, non-negotiable:**

This is a pre-production design document for an instrument that does not yet
exist in code. Nothing described here is audited, reviewed by a security
professional, or deployed on mainnet. All PoC work targets testnet-10 (TN10)
using synthetic data and no real-world value.

Specific limitations that must appear on every public artifact derived from
this design:

- **Unaudited.** No external security audit of `kcp-pq-anchor`, `kcp-ktt-token`,
  or the proposed CSCI covenant has been performed. Do not use with real value.
- **Testnet evidence is perishable.** TN10 resets by design. Cited txids are
  evidence at a point in time; the PoC must be re-run to refresh evidence before
  any public claim.
- **Standalone-based ZK settlement, not synchronous composition.** The vProg
  executes off-chain; its output is settled on-chain via STARK. Synchronous
  vProg composition (core-team-gated) is not used and is not on the critical path.
- **Operator trust residual.** The covenant enforces proof validity and state
  continuity. It does not enforce that the input data supplied to the prover was
  authentic. The guest program must be independently audited against its image_id.
- **Engine pin.** All code runs against rusty-kaspa v2.0.0 (`90dbf07`). No
  engine changes are proposed or permitted by this design.
- **No KIPs.** This design does not draft, propose, or depend on any new Kaspa
  Improvement Proposal. It uses KIP-16 tag-0x21 and KIP-20 covenant IDs as
  deployed on TN10 in rusty-kaspa v2.0.0.
- **The CSCI covenant script does not yet exist as code.** The SilverScript
  covenant binding the ZK journal to the KTT output state (§3.1) has not been
  authored. Authoring, compiling, and testing the `.sil` source file is a
  prerequisite for PoC steps A (genesis tx) and C (settlement tx). The
  `kcp-pq-anchor` script assembly helpers are reused; the covenant logic is new.
- **No mainnet deployment until explicit Foundation sign-off + fresh txid
  re-verification.** Toccata mainnet activates 30 Jun 2026 (DAA 474,165,565).
  The PoC targets TN10 only. No public artifact is posted without a fresh txid
  verification and explicit sign-off.

---

*This is a design-only artifact. No code was written or modified. The
implementation works from this document; independent verification checks
every on-chain claim.*
