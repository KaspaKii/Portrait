# Tier 3 Cross-Layer Binding — Design Document

**Status:** Design (pre-production, unaudited, testnet-only)  
**Date:** 2026-06-28  
**Author:** Kii Foundation  

---

## Problem

A Kaspa covenant enforces UTXO state transitions on L1. A RISC Zero vProg proves
off-chain computation correctness (compliance checks, complex business logic) and
produces a STARK. How do we *bind* the proof to a specific covenant instance so
the L1 covenant can trustlessly verify the computation?

The naive approach — put the proof script-pubkey in a `CHECKSIGFROMSTACK` — would
work for interactive spends, but we need a *non-interactive*, *composable* anchor
that:

1. Uniquely identifies the covenant type (not the instance UTXO).
2. Survives UTXO respending (the covenant ID is reborn each generation).
3. Can be included in the RISC Zero journal without knowing the current UTXO ID in advance.

KIP-20 covenant IDs solve (1) and (2). This document specifies how to use them
as the cross-layer anchor.

---

## Background: KIP-20 Covenant ID

KIP-20 (Kaspa Improvement Proposal 20) specifies a *covenant identity* field for
script-pubkeys. A covenant ID (`KovId`) is a 32-byte hash that identifies a
covenant *program* independently of which UTXO carries it. Two UTXOs with the
same covenant program (same silverscript bytecode, same constructor args) share
the same covenant ID.

The ID is stable across UTXO generations because it is derived from the compiled
bytecode and constructor parameters, not from the UTXO outpoint.

**Implementation note:** In rusty-kaspa v2.0.0 (`90dbf07`), `OpZkPrecompile` uses
`tag 0x21` for STARK verification. KIP-20's `covenant_id` field is populated in
the script-pubkey and is accessible inside covenant scripts via the
`COVENANT_ID` opcode (added in Toccata).

---

## Binding Architecture

```
  ┌──────────────────────────┐
  │  Portrait Source          │
  │  ComplianceToken.portrait │
  └──────────┬───────────────┘
             │ portrait engrave            portrait atelier-build
             ▼                                       ▼
  ┌──────────────────┐               ┌───────────────────────────┐
  │ ComplianceToken   │               │ compliancetoken_guest_main │
  │ .sil + CTOR.json  │               │ .rs (RISC Zero guest)     │
  └──────────┬────────┘               └───────────┬───────────────┘
             │ silverc                             │ cargo build (guest)
             ▼                                     ▼
  ┌──────────────────┐               ┌───────────────────────────┐
  │ KovId            │               │ CSCI Journal (104 bytes)  │
  │ = sha256(bytecode│◄──covenant_id─│ covenant_id[32]           │
  │   + ctor_args)   │               │ new_state_hash[32]        │
  │                  │               │ rule_hash[32]             │
  └──────────┬────────┘               │ seq[8 LE]                │
             │                        └───────────┬───────────────┘
             │  L1 covenant script                │ STARK proof
             │  (UTXO script-pubkey)              │ (Groth16 / SNARK)
             ▼                                    ▼
  ┌──────────────────────────────────────────────────────┐
  │  OpZkPrecompile (tag 0x21)                           │
  │  Verifies: proof commits to covenant_id known on L1 │
  │  i.e. journal.covenant_id == COVENANT_ID opcode val  │
  └──────────────────────────────────────────────────────┘
```

### Journal field: `covenant_id`

The RISC Zero guest writes the KovId into `journal[0..32]`. The L1 covenant
script reads its own covenant ID via `COVENANT_ID` and checks:

```silverscript
// Pseudocode — Toccata OpZkPrecompile verification
let journal_covenant_id = zk_proof.journal[0..32];
assert(journal_covenant_id == COVENANT_ID);
```

This makes the proof useless for a different covenant program — it is cryptographically
bound to the specific ComplianceToken bytecode + constructor args.

### Journal field: `new_state_hash`

`new_state_hash` (journal[32..64]) is the SHA-256 of the encoded new state.
The L1 covenant stores the current state hash in its UTXO state and checks
that the next UTXO carries the hash declared in the proof. This prevents
replay of old proofs.

### Journal field: `rule_hash`

`rule_hash` (journal[64..96]) is `sha256(entry_point_name_utf8)`. The L1
covenant can optionally enforce which rule was executed (e.g. only allow
`verify_compliance`, not `verify_anything`).

### Journal field: `seq`

`seq` (journal[96..104]) is a monotonic counter (u64 LE). The L1 covenant
stores the last accepted seq and rejects proofs where `journal.seq <= last_seq`.
Prevents replay of old valid proofs within the same covenant instance.

---

## Rust Types (kcp-csci)

```rust
/// A KIP-20 covenant identity — 32 bytes derived from bytecode + constructor args.
pub struct KovId(pub [u8; 32]);

/// Cross-layer binding: ties a CSCI vProg journal to a specific covenant program.
pub struct CovIdBinding {
    /// The KIP-20 covenant identity of the target covenant program.
    pub kov_id: KovId,
    /// The seq value at which the proof was generated.
    pub seq: u64,
    /// The rule hash (sha256 of the entry point name).
    pub rule_hash: [u8; 32],
    /// The new state hash committed to in the journal.
    pub new_state_hash: [u8; 32],
}

impl CovIdBinding {
    /// Construct from a raw 104-byte CSCI journal.
    pub fn from_journal(journal: &[u8; 104]) -> Self { ... }
    
    /// Validate that this binding matches the covenant program at `kov_id`.
    pub fn verify_kov_id(&self, expected: &KovId) -> bool { ... }
    
    /// Validate that the seq is strictly greater than the last accepted seq.
    pub fn verify_seq_advance(&self, last_seq: u64) -> bool { ... }
}
```

---

## Security Properties

| Property | Mechanism |
|---|---|
| Program binding | journal.covenant_id == KovId (bytecode + ctor hash) |
| State binding | journal.new_state_hash must match next UTXO state |
| Rule binding | journal.rule_hash == sha256(entry_name) |
| Replay resistance | journal.seq > last_accepted_seq |
| Proof soundness | RISC Zero STARK (Groth16 / inner snark) |

---

## M1 Limitations

- KovId computation is not yet wired into `portrait atelier-build`. The guest
  currently reads `covenant_id` from the host as an input (the host must supply
  it from the deployed bytecode). A later milestone will derive it automatically
  from the CTOR JSON.
- L1 silverscript covenant verification of the proof is not yet implemented.
  The covenant script stub in `crates/kcp-csci/` models the check but does not
  yet emit the `OpZkPrecompile` call.
- The covenant ID is accessed in SilverScript via the `OpInputCovenantId(input_index)`
  builtin (not a bare `COVENANT_ID` stack opcode). The pseudocode above uses
  `COVENANT_ID` as shorthand; the actual SilverScript ABI is `OpInputCovenantId`.
  This builtin is Toccata+ only (mainnet activates 2026-06-30 at DAA 474,165,565).

---

## References

- [KCP-CSCI architecture](FLAGSHIP-DESIGN.md)
- [CSCI provenance](CSCI-PROVENANCE.json)
- [kcp-csci crate](../crates/kcp-csci/)
- [Portrait Tier 3 demo](../../kii-portrait/portrait/examples/tier3-demo/)
