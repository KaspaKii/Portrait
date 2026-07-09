# kcp-paired-attestation

`kcp-paired-attestation` provides two-party mutual attestation: both parties
commit to the same record under their own blinding factors, then produce a
`MateProof` demonstrating they committed to identical content. It is the
bilateral escrow equivalent for Kaspa compliance use cases.

## Constructing an attestation

```rust
use kcp_paired_attestation::{
    mate::{build_mate_proof, verify_mate},
    record::{attestation_id, commit, AttestationRecord},
};

let record = AttestationRecord::new(subject, terms_hash, nonce);
let att_id = attestation_id(&record)?;

// Each party generates a CSPRNG blind — never reuse blinds across sessions.
let blind_a: [u8; 32] = secure_random_bytes();
let blind_b: [u8; 32] = secure_random_bytes();

let commit_a = commit(&record, &blind_a)?;
let commit_b = commit(&record, &blind_b)?;

let proof = build_mate_proof(&record, blind_a, blind_b, commit_a, commit_b)?;
verify_mate(&proof)?; // both parties committed to the same record
```

**Never reuse a blind across sessions.** A reused blind allows an observer to
correlate commitments across sessions. Draw blinds from a CSPRNG
(`rand::thread_rng().fill_bytes(&mut blind)`).

## A note on v0 vs. v1 enforcement

In v0, `verify_mate` is an off-chain check only — consensus does not enforce
it. In v1, the `MateProof` is submitted to a `OP_CHECKSIGFROMSTACK` (CSFS)
spend that enforces both commitments on-chain at the consensus layer
`[KCP-PA-002]`. The API is identical in both versions; only the spend path changes.

## Extensions

- **PQ upgrade** — replace Schnorr CSFS with an ML-DSA-44 proof via `kcp-pq-anchor`. See [PQ Anchor](./pq-anchor.md).
- **Compliance workflow** — chain with `kcp-sealed-lineage` to anchor the `attestation_id` into a tamper-evident log. See `examples/compliance-workflow`.

→ API reference: [`AttestationRecord`], [`attestation_id`], [`commit`], [`build_mate_proof`], [`verify_mate`], [`negotiate_blind`]
