# Covenant reference

> **Maturity:** pre-production, unaudited, testnet-only; perishable evidence — facts
> below trace to the `.portrait` sources at the date of this build and may go stale.

This reference catalogues every covenant the Portrait library ships, derived
directly from the `.portrait` sources. Each family page lists, per covenant, its
purpose, committed state fields, transitions with their verbatim `requires` guards,
declared invariants, and an honest-scope note.

## Covenant vs vProg patterns

A **plain covenant** is a singleton/role covenant whose transitions are checked
**structurally / relationally** by `portrait check` (no SMT solver). Guards are the
literal `requires(...)` lines in the source; `value_conserved` /
`conservation_split` are N-field additive-delta cancellations (not solver-proved
arithmetic); temporal gates are caller-asserted coarse `now_bucket` comparisons
(the engine sequence rule enforces the real relative-timelock); `multisig_threshold`
is a structural count of committed-key `checkSig` operands.

A **vProg (cross-layer) pattern** pairs an on-chain settlement covenant with an
off-L1 `NonCovenant` companion that Atelier lowers to a RISC Zero guest. The covenant
compiles and engraves a covenant-id binding, and the guest emits the 104-byte CSCI
journal; the heavy off-chain `predicate(...)` is **developer-authored, not synthesised
by the compiler** (`atelier-build`'s emitted stub returns `true` by default). The
library ships **ten** vProg patterns: **five settled live on TN10** with a real STARK
over a real authored predicate (ProofOfReserves, ComplianceCredential,
ConfidentialTransfer, BatchRollup, PrivateVoting), alongside the **CsciInstrument**
reference (state family); the **other five are emit-verified only** (compile +
engrave + a RISC Zero guest, predicate stubs — **not** settled live).
Honest residuals: the live covenant is the tag-0x21 verifier P2SH (image-id-pinned),
not yet a silverscript state machine; inputs are fixed sample data over small fixed
sets; the audit key is a v1 symmetric pad. The non-vProg covenants here are
compile-and-engrave artifacts, not on-chain settlements.

## Families

| Family | Page | Covenant sources |
|---|---|---:|
| Finance | [finance.md](./finance.md) | 18 |
| Custody | [custody.md](./custody.md) | 3 |
| Governance | [governance.md](./governance.md) | 2 |
| Attestation | [attestation.md](./attestation.md) | 1 |
| State / CSCI | [state.md](./state.md) | 1 |
| Cross-layer (vProg) | [vprog.md](./vprog.md) | 10 (5 settled-live + 5 emit-verified) |
| **Library total** | | **35** |
| Example covenants | [examples.md](./examples.md) | 6 |

## Library totals

- **35 covenant sources** (`.portrait`). DigitalReit is the only multi-role covenant
  and emits **two** `.sil` (`DigitalReitToken.sil` + `DigitalReitSplitter.sil`); every
  other source emits one.
- **10 of the 35** are cross-layer **vProg** patterns (the Cross-layer family).
  **Five are settled live on TN10**: ConfidentialTransfer, ComplianceCredential,
  BatchRollup, PrivateVoting, ProofOfReserves — alongside CsciInstrument (state
  family), which additionally carries a vProg companion. The **other five
  (MerkleProofOfSolvency, PrivateOrderMatch, PrivateVickreyAuction,
  ZkAllowlistTransfer, ZkExecutionRollup) are emit-verified only** — they compile,
  engrave, and emit a RISC Zero guest, but are **not** settled live.
  The **35 library covenant sources** break down as 25 non-vProg (finance 18,
  custody 3, governance 2, attestation 1, state 1) + 10 cross-layer vProg
  (5 settled-live + 5 emit-verified).
- The **6 example covenants** under `portrait/examples` (Counter, SimpleToken,
  PausableToken, VestingWallet, ComplianceToken, PersonalVault) are illustrative
  and counted **separately** from the 35 library sources; 5 emit a `.sil`
  (PersonalVault is a non-covenant app-composition source).
