# kcp-pq-anchor

> **Pre-production, unaudited, testnet-only.**

`kcp-pq-anchor` provides KIP-16 tag-0x21 post-quantum credential anchor script
assembly for the Kaspa BlockDAG — the first post-quantum credential anchor in
any Kaspa library.

The Kaspa VM includes `OpZkPrecompile`, a consensus-level RISC Zero verifier.
Tag `0x21` identifies the RISC Zero Groth16/STARK variant. This crate assembles
the 8-field script that invokes the precompile. Proof generation requires RISC
Zero v3.0.5 running your own guest program; this library solves the hardest
part: canonical opcode assembly.

**Why post-quantum matters**: secp256k1 Schnorr signatures are not post-quantum
safe. A sufficiently large fault-tolerant quantum computer could break ECDLP and
forge Schnorr signatures. `kcp-pq-anchor` provides a path to ML-DSA-44 (or any
RISC Zero guest) verified at consensus before Kaspa mainnet reaches that threat
horizon.

## Constructing a PQ anchor script

```rust
use kcp_pq_anchor::{
    anchor_script::{build_pq_anchor_redeem, PqAnchorScriptFields},
    journal_spec::JournalSpec,
    sigop::sigop_count_for_pq_verify,
};

// Derive the journal hash from the pattern-specific binding
let journal = JournalSpec::PairedAttestation {
    attestation_id: my_att_id,
    spend_outpoint: my_outpoint,
}.journal_hash();

// Assemble the KIP-16 tag-0x21 redeem script
let script = build_pq_anchor_redeem(&PqAnchorScriptFields {
    claim: my_claim_bytes,
    control_index: 0,
    control_digests: my_path_digests_concat, // multiple of 32 bytes
    seal: my_stark_seal,                     // ~222 KB from RISC Zero output
    journal,
    image_id: MY_IMAGE_ID,
    control_id: MY_CONTROL_ID,
})?;

// Set sigOpCount = 255 when submitting the spending transaction
let sig_ops = sigop_count_for_pq_verify(); // 255
```

## A note on the 8-field tag-0x21 stack

The KIP-16 tag-0x21 script pushes eight fields in this exact order:

| # | Field | Type | Notes |
|---|-------|------|-------|
| 1 | `claim` | bytes | Arbitrary claim payload |
| 2 | `control_index` | u32 LE | Merkle path index |
| 3 | `control_digests` | bytes | Concatenated 32-byte path digests |
| 4 | `seal` | bytes | RISC Zero STARK seal (~222 KB) |
| 5 | `journal` | [u8; 32] | `sha256(journal_bytes)` — caller pre-computes |
| 6 | `image_id` | [u8; 32] | Guest image hash |
| 7 | `control_id` | [u8; 32] | Control root hash |
| 8 | `hashfn` | opcode | **Must be `OP_1` (0x51), never data push** |

**It is critical** that `hashfn` is pushed as `OP_1` (opcode 0x51), not as a
1-byte data push `[0x01, 0x01]`. Consensus rejects non-canonical integer pushes.
`build_pq_anchor_redeem` enforces this internally.

**`journal` is `sha256(journal_bytes)`**, not the raw journal. The pattern-specific
`JournalSpec` enum computes this hash for you given the structured inputs.

## A note on proof generation

This crate assembles the script. Proof generation requires a separate pipeline:

1. Write a RISC Zero guest program that verifies your credential logic
2. Run the guest with `risc0-zkvm v3.0.5` to produce the seal and journal
3. Pass the proof fields to `build_pq_anchor_redeem`
4. Submit the spending transaction with `sigOpCount = 255`

The `kii-ml-dsa` repository (forthcoming) provides a worked ML-DSA-44 example.

## Extensions — per-pattern JournalSpec bindings

Each pattern has a defined `journal_bytes` encoding:

| Pattern | JournalSpec | Encoding |
|---------|-------------|----------|
| `kcp-paired-attestation` | `PairedAttestation` | `attestation_id (32) ‖ spend_outpoint (36)` |
| `kcp-sealed-lineage` | `SealedLineage` | `lineage_id (32) ‖ seq (8 LE) ‖ t_bucket (8 LE)` |
| `kcp-transferable-record` | `TransferableRecord` | `record_id (32) ‖ new_controller_xonly (32)` |
| Any | `Custom([u8; 32])` | Caller provides pre-hashed journal |

→ API reference: [`PqAnchorScriptFields`], [`build_pq_anchor_redeem`], [`JournalSpec`], [`sigop_count_for_pq_verify`]
