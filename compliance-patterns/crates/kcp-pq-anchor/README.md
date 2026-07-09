# kcp-pq-anchor

> **Pre-production, unaudited, testnet-only.**

KIP-16 tag-0x21 post-quantum credential anchor for the Kaspa BlockDAG.
Script assembly helpers for RISC Zero succinct STARK proofs — the first
post-quantum credential anchor in any Kaspa library.

This crate assembles the verifiable redeem script. Proof generation requires
RISC Zero v3.0.5 running your own guest program. The library solves the hardest
part: correct KIP-16 tag-0x21 opcode assembly.

## Quick start

```rust
use kcp_pq_anchor::{
    anchor_script::{build_pq_anchor_redeem, PqAnchorScriptFields},
    journal_spec::JournalSpec,
    sigop::sigop_count_for_pq_verify,
};

let journal = JournalSpec::PairedAttestation {
    attestation_id: my_attestation_id,
    spend_outpoint: my_outpoint_bytes,
}.journal_hash();

let script = build_pq_anchor_redeem(&PqAnchorScriptFields {
    claim: my_claim_bytes,
    control_index: 0,
    control_digests: my_control_digests_concat,
    seal: my_stark_seal,
    journal,
    image_id: MY_IMAGE_ID,
    control_id: MY_CONTROL_ID,
})?;

// Set sigOpCount = 255 when submitting the spending transaction
let sig_ops = sigop_count_for_pq_verify(); // 255
```

## Key invariant: canonical hashfn push

The `hashfn` field (Poseidon2 = integer 1) **must** be pushed as `OP_1` (0x51),
never as a 1-byte data push `[0x01, 0x01]`. Consensus rejects non-canonical
integer pushes. `build_pq_anchor_redeem` enforces this internally.

## Licence

MIT — Stichting Kii Foundation
