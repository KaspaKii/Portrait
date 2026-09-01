# Cross-layer (vProg) patterns

> **Maturity:** Pre-production, unaudited, testnet-only, perishable evidence. Every
> covenant on this page compiles via `silverc` (engrave → exit 0) and emits a
> RISC Zero guest (`atelier-build`). The heavy off-chain predicate is
> **developer-authored** (not synthesised by the compiler). The library ships **ten**
> vProg patterns; the **five documented in detail below have been settled live on
> TN10** with a real RISC Zero STARK over that authored predicate, verified
> in-consensus via the KIP-16 tag-0x21
> precompile (PoR `3d7c4a16…`, ZK-KYC `52616496…`, ConfidentialTransfer
> `2d8166f9…`, BatchRollup `0237d2de…`, PrivateVoting `112780ec…`; each
> `is_accepted=true`, REST-confirmed), with per-pattern negative controls rejected
> by the live node. **Honest residuals (all five):** the live covenant is the
> tag-0x21 verifier P2SH (image-id-pinned), **not yet** a silverscript state
> machine the way `CsciInstrument`'s seq/auth/cov-id rules are; inputs are fixed
> sample data over small fixed sets (not Merkle-rooted registries; no persistent
> nullifier set); commitments are `sha256(value‖blinding)`, not Pedersen; the
> audit key is a v1 symmetric pad. These patterns reuse the same proven CSCI
> settlement harness as `CsciInstrument`. The **remaining five vProg patterns**
> (`MerkleProofOfSolvency`, `PrivateOrderMatch`, `PrivateVickreyAuction`,
> `ZkAllowlistTransfer`, `ZkExecutionRollup`) are **emit-verified only** — they
> compile, engrave, and emit a RISC Zero guest, but are **not** settled live.

The cross-layer family holds **ten** patterns; the **five settled-live** ones
documented here share the **cross-layer (vProg) shape** proven by the CSCI
instrument: an on-chain `#[covenant]` settlement role that enforces the structural
state machine, paired with an off-L1 NonCovenant companion entrypoint whose body is
lowered into a RISC Zero zk-STARK guest. Each pattern exists because its core rule is
a predicate over **private / hidden data** that a pure covenant cannot recompute
without breaking confidentiality, so the predicate is discharged in the guest and the
covenant settles a transition **bound to that proof**.

## Shared mechanics (all five patterns)

**The 104-byte CSCI journal.** Every emitted guest commits a fixed-layout RISC Zero
STARK journal:

```
covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]
```

- `covenant_id` — the auto-derived KovId (sha256 of the silverc-compiled script
  bytes) of the covenant this proof binds to.
- `new_state_hash` — `sha256(encode_state(NEW fields))`, computed over the new
  state the covenant adopts on chain.
- `rule_hash` — `sha256(entrypoint name)` (the vProg companion's name).
- `seq` — `prev_seq + 1` (u64 LE). Per Atelier's CSCI convention the **journal** seq
  always advances by one, even where the on-chain seq advances differently (see
  BatchRollup).

**The covenant binding.** Because each role carries a NonCovenant (vProg) companion
entrypoint, `has_vprog` is set, which causes the emitted `.sil` to include the
covenant-id binding require: `proof_cov_id == OpInputCovenantId(0)`. This ties this
specific instrument to the proof that produced its new state-commitment.

**The developer predicate.** The emitted guest contains a
`predicate(...) -> bool` developer hook over the typed inputs. As emitted by
`atelier-build` it is a stub returning `true` — the substantive claim (fold /
commitment open / membership / policy circuit) is **developer-authored, not
synthesised by the compiler**. For the live settlements on this page, a **real**
predicate body was authored into each guest and a real STARK generated over it
(`RISC0_DEV_MODE=0`); the guest `assert!`s the predicate before committing the
journal, so an unsatisfied predicate panics and no proof can exist.

**Honest scope (all five).** Covenant type-checks are **structural/relational (no SMT
solver)**. The tag-0x21 ZK *verification* of the STARK proof is an **engine-level
operation** (`OpZkPrecompile` / `kcp-pq-anchor`), NOT a silverscript op — it is
layered by the settlement harness (`examples/portrait-settlement`), which feeds the
journal's `covenant_id` in as `proof_cov_id`. silverscript here enforces only the
on-chain state-machine rules + the covenant-id binding; the confidentiality/soundness
of each claim rests on the RISC Zero guest + the verifier, not on Portrait.

Sources: `library/vprog/<pattern>/<Name>.portrait` and the emitted
`<name>_guest_main.rs` alongside each.

---

## ConfidentialTransfer

Source: `library/vprog/confidential-transfer/ConfidentialTransfer.portrait` (role
`instrument`) · guest: `confidentialtransfer_guest_main.rs`.

**Purpose.** A hidden-amount transfer: the UTXO carries only a `commitment` to a
hidden balance/transfer state plus a monotonic `seq`; transfer amounts are never on
chain. The guest proves OFF-L1 that a transfer is valid over those hidden amounts.

**State.**
- `owner: pubkey` — committed owner key (the settle authority).
- `commitment: bytes32` — hiding commitment to the balance/transfer state.
- `seq: int` — monotonic CSCI sequence number (genesis = 0).

**Transitions.**
- `settle(sig auth, bytes32 next_commitment)` — the L1 covenant
  (`#[covenant(mode = transition)]`).
  - Guard: `requires checkSig(auth, owner);` (committed-owner authorization).
  - Updates: `owner: owner` (unchanged), `commitment: next_commitment` (adopt new
    state-commitment), `seq: seq + 1`.
  - Covenant-id binding require emitted by `has_vprog`.
- `transfer_rules(bytes32 next_commitment)` — the vProg (NonCovenant) companion. No
  guard. Mirrors `settle`: `commitment: next_commitment`, `seq: seq + 1`. Manual
  guest predicate: opening the Pedersen commitment over the hidden amount and proving
  conservation (`in == out`, `amount >= 0`).

**Invariants.** `monotonic_seq` (seq advances by exactly one); `no_undeclared_state`.

**Honest scope.** **Settled live on TN10** — settle `2d8166f9…` (`is_accepted`):
a real in-zkVM predicate (range + conservation + audit-binding) over hidden amounts,
plus a positive audit-decrypt. Residuals: the audit key is a v1 symmetric pad (not
ElGamal/Pedersen); the live covenant is the tag-0x21 verifier P2SH, not yet a
silverscript state machine.

---

## ComplianceCredential

Source: `library/vprog/compliance-credential/ComplianceCredential.portrait` (role
`credential`) · guest: `compliancecredential_guest_main.rs`.

**Purpose.** A privacy-preserving compliance settlement: the guest proves the holder
satisfies a compliance predicate (e.g. attribute >= threshold, jurisdiction in
allowed set, not-on-a-list) WITHOUT revealing the credential; only a credential
commitment + the boolean verdict enter the journal.

**State.**
- `owner: pubkey` — committed owner key (the settle authority).
- `commitment: bytes32` — credential commitment (chain never sees the credential).
- `verdict: int` — boolean verdict from the predicate (0 = fail, 1 = pass).
- `seq: int` — monotonic CSCI sequence number (genesis = 0).
- `state_hash: bytes32` — CSCI content/state hash (new_state_hash in the journal).

**Transitions.**
- `settle(sig auth, bytes32 next_state_hash)` — the L1 covenant.
  - Guard: `requires checkSig(auth, owner);`.
  - Updates: `owner: owner`, `commitment: commitment` (carried — credential stays
    private), `verdict: verdict` (carried), `seq: seq + 1`,
    `state_hash: next_state_hash`.
  - Covenant-id binding require emitted by `has_vprog`.
- `predicate(bytes32 next_state_hash)` — the vProg companion. No guard. Mirrors
  `settle`. Manual guest predicate: opening the commitment, evaluating the arbitrary
  compliance policy over the private credential, deriving `verdict`.

**Invariants.** `monotonic_seq`; `no_undeclared_state`.

**Honest scope.** **Settled live on TN10** — settle `52616496…` (`is_accepted`):
a real selective-disclosure predicate over private attributes (accredited ∧ allowed
jurisdiction ∧ not-sanctioned), attributes never leaving the zkVM, four negative
controls rejected. Residuals: inputs are fixed sample data over small fixed sets (the
sanctions/jurisdiction sets are fixed arrays, not Merkle-rooted); the live covenant is
the tag-0x21 verifier P2SH, not yet a silverscript state machine.

---

## BatchRollup

Source: `library/vprog/batch-rollup/BatchRollup.portrait` (role `rollup`) · guest:
`batchrollup_guest_main.rs`.

**Purpose.** A rollup-style fold: aggregate N state transitions into ONE on-chain
settlement. The guest proves that folding N transitions over `prev_root` yields
`next_root`; the covenant settles the new root in a single spend bound to that proof.

**State.**
- `operator: pubkey` — committed operator key (the settle authority).
- `root: bytes32` — committed rollup state root (the aggregate).
- `seq: int` — monotonic batch sequence (genesis = 0).

**Transitions.**
- `settle(sig auth, bytes32 next_root, int batch_count)` — the L1 covenant.
  - Guards: `requires checkSig(auth, operator);` (committed-operator auth);
    `requires batch_count >= 1;` (forward batch progress).
  - Updates: `operator: operator`, `root: next_root` (adopt proven next root),
    `seq: seq + batch_count` (sequence advances by the batch size).
  - Covenant-id binding require emitted by `has_vprog`.
- `apply_batch(bytes32 next_root, int batch_count)` — the vProg companion.
  - Guard: `requires batch_count >= 1;` (lowered into an in-guest `assert!`).
  - Updates: `root: next_root`, `seq: seq + batch_count`. Manual guest predicate: the
    fold itself — proving applying `batch_count` private transitions to `prev_root`
    yields `next_root`.

**Invariants.** `no_undeclared_state`. Note: `monotonic_seq` is **deliberately NOT
declared** — the on-chain `seq` advances by `batch_count` (N), not by one. The
**journal** seq still advances by one per Atelier's CSCI convention.

**Honest scope.** **Settled live on TN10** — settle `0237d2de…` (`is_accepted`):
a real in-zkVM N=5 fold (`root = sha256(root‖delta)`), three negative controls + a
tampered-journal reject + a positive seq-advanced-by-N=5 check. Residuals: deltas are
fixed sample data; the live covenant is the tag-0x21 verifier P2SH, not yet a
silverscript state machine.

---

## PrivateVoting

Source: `library/vprog/private-voting/PrivateVoting.portrait` (role `ballotbox`) ·
guest: `privatevoting_guest_main.rs`.

**Purpose.** An anonymous-ballot pattern: an electorate casts ballots into a running
tally without revealing who voted for what. Each accepted ballot must prove
eligibility (member of the committed registry) and one-vote (a fresh nullifier),
both WITHOUT unmasking the voter; the new tally is a fold of the prior tally with the
ballot.

**State.**
- `owner: pubkey` — committed owner key (the settle authority).
- `registrar: pubkey` — committed registrar key (authority over eligibility).
- `tally_root: bytes32` — committed commitment to the running tally.
- `seq: int` — monotonic CSCI sequence number (genesis = 0).

**Transitions.**
- `settle(sig auth, bytes32 next_tally_root)` — the L1 covenant.
  - Guard: `requires checkSig(auth, owner);` (committed-owner auth).
  - Updates: `owner: owner`, `registrar: registrar` (carried unchanged),
    `tally_root: next_tally_root` (adopt proven new tally-commitment), `seq: seq + 1`.
  - Covenant-id binding require emitted by `has_vprog`.
- `vote_rule(bytes32 next_tally_root)` — the vProg companion. No guard. Mirrors
  `settle`. Manual guest predicate: eligibility membership in the committed registry,
  nullifier freshness (exactly one vote), and folding the hidden ballot into the prior
  tally to justify `next_tally_root` — all without revealing identity or choice.

**Invariants.** `monotonic_seq`; `no_undeclared_state`.

**Honest scope.** **Settled live on TN10** — settle `112780ec…` (`is_accepted`):
a real in-zkVM predicate (eligibility membership + ballot validity + correct tally
update + a one-vote nullifier), the ballot never leaving the zkVM, with a positive
tally-advance check and negative controls rejected. Residuals: the electorate is a
fixed array (not Merkle-rooted) and there is **no persistent nullifier set across
settlements**; the live covenant is the tag-0x21 verifier P2SH, not yet a silverscript
state machine.

---

## ProofOfReserves

Source: `library/vprog/proof-of-reserves/ProofOfReserves.portrait` (role
`custodian`) · guest: `proofofreserves_guest_main.rs`.

**Purpose.** A solvency-attestation pattern: a custodian periodically attests
`sum(reserves) >= sum(liabilities)` over a PRIVATE set of account balances.
Summation over the large private set cannot be recomputed on chain, so the rule lives
in the guest; the covenant records a monotonic attestation transition bound to that
proof.

**State.**
- `attestor: pubkey` — committed attestor key (the attest authority).
- `epoch: int` — monotonic reporting epoch (genesis = 0).
- `solvent: bool` — attested verdict for `epoch`.
- `accounts_root: bytes32` — Merkle root of the attested account set.
- `seq: int` — monotonic CSCI sequence number (genesis = 0).

**Transitions.**
- `attest(sig auth, bool next_solvent, bytes32 next_accounts_root)` — the L1 covenant
  (the lifecycle settles via `custodian.attest`).
  - Guard: `requires checkSig(auth, attestor);` (committed-attestor auth).
  - Updates: `attestor: attestor`, `epoch: epoch + 1` (reporting epoch advances by
    one), `solvent: next_solvent` (adopt verdict),
    `accounts_root: next_accounts_root` (adopt account-set root), `seq: seq + 1`.
  - Covenant-id binding require emitted by `has_vprog`.
- `solvency_rule(bool next_solvent, bytes32 next_accounts_root)` — the vProg
  companion. No guard. Mirrors `attest`. Manual guest predicate: folding the private
  per-account balances and proving `sum(reserves) >= sum(liabilities)` to justify
  `next_solvent`, and proving `next_accounts_root` is the Merkle root of the included
  accounts.

**Invariants.** `monotonic_seq`; `no_undeclared_state`. (The epoch also advances by
exactly one per the `attest` body.)

**Honest scope.** **Settled live on TN10** — settle `3d7c4a16…` (`is_accepted`):
a real in-zkVM solvency predicate (`sum(reserves) >= sum(liabilities)`, overflow-checked;
insolvency panics so no proof can exist), with an insolvency negative control and a
tampered-journal reject. Residuals: balances are fixed sample data; `accounts_root` is
not a production Merkle accumulator; the live covenant is the tag-0x21 verifier P2SH,
not yet a silverscript state machine.
