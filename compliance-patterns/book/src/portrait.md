# Portrait — the covenant compiler & cross-layer catalogue

> **Maturity:** pre-production, unaudited, testnet-only. On-chain evidence is
> perishable by design (testnets reset). The flagship `CsciInstrument` path and
> **five of the ten** cross-layer (vProg) patterns have been settled live on
> Kaspa testnet-10 (real STARKs verified in-consensus via the KIP-16 tag-0x21
> precompile); the other five vProg patterns are **emit-verified only** (they
> compile and emit a guest, but are **not** settled live), and the remaining
> covenant patterns on this page are compile-verified (silverc exit 0), not
> settled live.

**Portrait** is a high-level surface language and Rust toolchain that compiles one
source program down to **both** Kaspa layers: a **SilverScript covenant**
(`.sil`, the L1 spending policy) and, for cross-layer patterns, a **RISC Zero
vProg** (an off-L1 guest whose succinct STARK is verified in-consensus via the
KIP-16 **tag-0x21** precompile).

The pipeline is:

```
source.portrait
  → portrait-syntax    (parse)
  → portrait-sema      (typed / structural checks)
  → portrait-ir        (lower to Cartoon IR)
  → portrait-project   (Pounce — projection: covenant vs vProg)
  → portrait-emit      (Engraver — .sil + CTOR JSON; vProg guest for paired roles)
  → silverc            (the pinned compiler accepts .sil → JSON script)
```

Engine reference pinned throughout: **rusty-kaspa tag v2.0.0 (commit 90dbf07)**.

## What "compiles through the pipeline" means — and does not

Every covenant below parses, passes the typed semantic checks, and emits a `.sil`
the pinned `silverc` accepts (exit 0). That is evidence the source is well-formed
and self-consistent. It is **not** a security audit, and the semantic checks are
**structural / per-field** — they are not a full SMT solver and do not prove
cross-field value flow.

### Refinement invariants (seven, opt-in, checker-enforced)

`value_conserved` · `conservation_split` · `monotonic_seq` ·
`non_negative_amount` · `bounded_supply` · `spending_cap` ·
`multisig_threshold` · `temporal_guard`.

`conservation_split` asserts an **N-field structural split** (value drawn from one
committed field arrives across N destination fields, the added deltas netting the
subtracted deltas) — generalised from the original two-field form and exercised by
`InternalSplit` / `RoyaltySplit` on a fan-out (>2-leg) shape. It is structural
N-field additive-delta arithmetic, **not** a general cross-field flow proof and
**not** an SMT proof.

### Real hash builtin

The language has a real `blake2b` builtin that lowers to the engine intrinsic
(`silverc blake2b(_) → OpBlake2b, 0xaa`). The `Htlc` and `SealedBidAuction`
patterns use it for a **true** on-chain `blake2b(preimage) == hashlock` digest
lock — a hashlock computed on-chain by the covenant, not a committed-value
equality placeholder.

## Covenant catalogue (35 sources)

The library carries **35 `.portrait` covenant sources** (counted under
`library/`: finance 18, custody 3, governance 2, attestation 1, state 1, and
**ten** cross-layer vProg). An engrave sweep over the sources runs `silverc` to
exit 0; `DigitalReit.portrait` emits 2 covenants (DigitalReitToken +
DigitalReitSplitter), every other source emits one. The **ten** cross-layer
(vProg) patterns under `library/vprog/` are a **subset** of the 35, not
additional; the table below lists representative non-vProg sources, with the
vProg catalogue in its own section.

| Domain | Pattern | Honest scope note |
|---|---|---|
| Finance | Escrow, StreamingVesting, DigitalReit (Token + Splitter) | DigitalReit emits 2 covenants; child binds parent by covenant-id lineage. |
| Finance | Htlc | true `blake2b(preimage) == hashlock` digest lock + timeout refund. |
| Finance | ArbiterEscrow | 2-of-3 written `(a&&b) \|\| (a&&c) \|\| (b&&c)` (see authoring convention below). |
| Finance | InternalTransfer | `conservation_split` two-field structural split. |
| Finance | **InternalSplit** *(new)* | N-field `conservation_split` — one source split additively across several legs. |
| Finance | **RoyaltySplit** *(new)* | one-source, three-payee royalty fan-out; N-field `conservation_split`. Structural additive-delta arithmetic, **not** an SMT proof. |
| Finance | **CollateralVault** *(new)* | CDP-style deposit/borrow/repay; ratio guard is a committed-state integer comparison — **no division, no oracle, no liquidation-safety proof**. |
| Finance | **TokenAllowance** *(new)* | ERC-20 approve/transferFrom; single owner→spender pair, **not** an allowances mapping. |
| Finance | **SealedBidAuction** *(round-3)* | commit-reveal via a true `blake2b` hashlock; "highest wins" is a **monotone guard**, **not a global-max proof**. |
| Finance | **Subscription** *(round-3)* | periodic pulls gated by a per-period `temporal_guard`. |
| Governance | MultisigTreasury | authorized-capability invariant. |
| Governance | **SocialRecovery** *(round-3)* | guardian 2-of-3 recovery; same `&&`/`\|\|` convention. |
| Custody | TimeVault, SpendingLimitVault, DeadMansSwitch | `spending_cap` / `temporal_guard` invariants. |
| Attestation | EvidenceLineage | append-only lineage. |
| State | CsciInstrument | **settled LIVE on TN10** (seq + auth + cov-id binding). |
| Examples | Counter, SimpleToken, PausableToken, VestingWallet, ComplianceToken | worked examples; ComplianceToken pairs a covenant + a vProg guest. |

### Authoring convention: 2-of-3 quorums

`SocialRecovery` and `ArbiterEscrow` express a 2-of-3 quorum as a disjunction of
the three conjunctive pairs — `(a&&b) || (a&&c) || (b&&c)` — **without
parentheses**, relying on the silverscript convention that `&&` binds tighter
than `||` (the Portrait Pratt parser and silverscript agree on this precedence).
This is an authoring convention; the grouping is `(a&&b) || (a&&c) || (b&&c)`.

## Cross-layer (vProg) catalogue (10 patterns: 5 settled live, 5 emit-verified)

Each pairs an on-chain covenant with an off-L1 RISC Zero zk guest, following the
same proven shape as `CsciInstrument`. For each, `portrait engrave` → silverc
exit 0 and `portrait atelier-build` emits the guest. **Five of the ten are
settled live on TN10** (table below) with a real RISC Zero STARK over an in-zkVM
predicate, verified in-consensus via the tag-0x21 precompile (each
`is_accepted=true`, REST-confirmed), with per-pattern negative controls rejected
by the live node. The **other five — MerkleProofOfSolvency, PrivateOrderMatch,
PrivateVickreyAuction, ZkAllowlistTransfer, ZkExecutionRollup — are emit-verified
only**: they compile, engrave, and emit a RISC Zero guest, but are **not**
settled live (their predicates remain stubs).

| Pattern | What the guest proves (off-L1) | Live settle (TN10) |
|---|---|---|
| ProofOfReserves | `sum(reserves) >= sum(liabilities)` over a private account set (overflow-checked; insolvency panics). | `3d7c4a16…` |
| ComplianceCredential | ZK-KYC selective disclosure over private attributes (accredited ∧ allowed jurisdiction ∧ not-sanctioned); attributes never leave the zkVM. | `52616496…` |
| ConfidentialTransfer | hidden-amount transfer: range + conservation + audit-binding; amounts never on chain. | `2d8166f9…` |
| BatchRollup | folding N=5 deltas over `prev_root` yields `next_root`; the on-chain seq advances by N in one settlement. | `0237d2de…` |
| **PrivateVoting** *(round-3)* | voter is eligible (membership) and votes once (nullifier); a tally update is adopted without revealing the ballot. | `112780ec…` |

Plus a combined two-input cross-layer tx `abc2d13f…` (`is_accepted`) whose
acceptance required both the silverscript covenant input and the tag-0x21
verifier input.

> **Honest residuals for the live vProgs (all five).** The heavy predicate (the
> substantive claim each guest proves) is **authored by the developer into the
> guest, not synthesised by the compiler**; the live STARKs run those authored
> predicates. The residuals: (1) the live covenant is the **tag-0x21 verifier
> P2SH** (image-id-pinned) — the pattern-specific state-machine rules are **not
> yet** lifted into a silverscript covenant the way `CsciInstrument`'s seq/auth/
> cov-id rules are; (2) inputs are **fixed sample data over small fixed sets**
> (electorate / sanctions / allowlist are fixed arrays, not Merkle-rooted
> registries; no persistent nullifier set across settlements for PrivateVoting);
> commitments are `sha256(value‖blinding)`, not Pedersen; the audit key is a v1
> symmetric pad, not ElGamal. The covenant type-checks themselves remain
> **structural / per-field**, not an SMT proof.

See `KNOWN-ISSUES.md` for the scope caveats and per-pattern residuals.
