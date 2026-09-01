# State / CSCI covenants

> **Maturity:** Pre-production, unaudited, testnet-only, perishable evidence. Covenant
> type-checks are structural/relational (no SMT solver). The heavy vProg predicate is
> developer-authored, not synthesised by the compiler (`atelier-build`'s emitted stub
> returns `true` by default). `CsciInstrument` is the covenant in **this** family
> settled LIVE on TN10; the five [cross-layer (vProg) patterns](./vprog.md) (a separate
> family) have **also** been settled live, each with a real STARK over a real authored
> predicate.

This family covers covenants that lift a CSCI (Committed-State Continuity Instrument)
state machine on-chain. The structural transition rules — sequence monotonicity,
committed-owner authorization, value conservation, covenant-id binding — live in the
emitted silverscript; the ZK proof itself is verified at settle time by the engine
(tag-0x21 `OpZkPrecompile` / `kcp-pq-anchor`), not by silverscript.

Source: `library/state/` in the Portrait repository.

## CsciInstrument

A covenant that self-enforces the CSCI state machine on-chain (silverscript), rather
than relying on the vProg alone. It lifts the structural rules of the CSCI transition
on-chain so that the rules no longer live only in the off-L1 vProg guest.

Source files:
- `CsciInstrument.portrait` — covenant source
- `csciinstrument_guest_main.rs` — RISC Zero vProg guest companion (M3)

This covenant has a vProg companion (the `csci_rules` entrypoint carries no
`#[covenant]` attribute), so it emits **one `.sil`** (the on-chain `settle`
covenant, `CsciInstrument.sil`) plus the guest main for the off-L1 predicate. The
presence of the vProg flips `has_vprog`, which causes the covenant-id binding
`require()` to be emitted into `settle`.

### State

| Field | Type |
|---|---|
| `owner` | `pubkey` — committed owner key (the settle authority) |
| `amount` | `int` — value carried by the instrument (conserved) |
| `seq` | `int` — monotonic CSCI sequence number (genesis = 0) |
| `state_hash` | `bytes32` — CSCI content/state hash (`new_state_hash` in the journal) |

Genesis is constructed from one `param` per state field, in field order: `owner`,
`amount`, `seq`, `state_hash`.

### Transitions

- **`settle(sig auth, bytes32 next_state_hash)`** — `#[covenant(mode = transition)]`,
  the on-chain CSCI transition. Lifecycle: `live -> live via instrument.settle`.
  - Guard: `requires checkSig(auth, owner)` — authorization against the **committed**
    owner key from prior state, never a caller argument.
  - Guard (emitted, not in source): a covenant-id binding `require()` is emitted
    automatically because the role has a vProg companion (`has_vprog`); it binds the
    STARK journal's `covenant_id` to this covenant's own on-chain id
    (`proof_cov_id == OpInputCovenantId(0)`).
  - Field updates (returns `CsciInstrument`):
    - `owner: owner` — owner key carried unchanged
    - `amount: amount` — value conserved (carry field:field)
    - `seq: seq + 1` — CSCI sequence advances by exactly one
    - `state_hash: next_state_hash` — adopt the new committed state hash

- **`csci_rules(bytes32 next_state_hash)`** — no `#[covenant]` attribute → lowered as
  a NonCovenant (vProg) transition; Atelier emits its RISC Zero guest main, the
  Engraver ignores it. Its body mirrors `settle` (consumes `next_state_hash`,
  advances `state_hash`/`seq`, carries `owner`/`amount`), so the guest hashes the NEW
  state into the journal's `new_state_hash`.
  - Field updates (returns `CsciInstrument`):
    - `owner: owner`
    - `amount: amount`
    - `seq: seq + 1`
    - `state_hash: next_state_hash`
  - Guest journal (104 bytes): `covenant_id[32] || new_state_hash[32] || rule_hash[32] || seq[8 LE]`,
    where `new_state_hash = sha256(encode_state(owner, amount, seq+1, next_state_hash))`,
    `rule_hash = sha256("csci_rules")`, and `seq = prev_seq + 1`.
  - **Predicate hook is a stub:** `predicate(...)` returns `true` by default — the
    guest does **not** yet prove the heavy off-chain claim.

### Invariants

Declared in the source:
- `invariant value_conserved;` — C1: `amount` must derive from its own prior value.
- `invariant monotonic_seq;` — C3: `seq` must advance by exactly one.
- `invariant no_undeclared_state;`

### Honest scope

silverscript enforces the structural state-machine rules + covenant-id binding; the
tag-0x21 ZK verification of the STARK proof is an engine-level op composed at settle
time, and the vProg's heavy predicate is a developer stub returning `true`.
