# Compliance Credential Lifecycle

**Reference implementation** showing four `kaspa-compliance-patterns` crates
working together as a complete compliance workflow on the Kaspa BlockDAG.

Pre-production, unaudited, testnet-only.

## What this demonstrates

```
[1] Bilateral attestation     kcp-paired-attestation
    Two parties commit to a KYC credential via blinded mate proof.

[2] Evidence lineage          kcp-sealed-lineage
    The attestation is anchored into an append-only tamper-evident lineage.

[3] Record transfer           kcp-transferable-record
    The credential record is transferred to the subject controller.

[4] Regulated token           kcp-ktt-token
    A compliance token (KCC20-shape) is minted for the subject's entitlement.
```

## Run it

```sh
cargo run --manifest-path examples/compliance-workflow/Cargo.toml
```

## Run the smoke tests

```sh
cargo test --manifest-path examples/compliance-workflow/Cargo.toml
```

## Pattern selection

| Need | Pattern |
|------|---------|
| Two-party bilateral commitment | `kcp-paired-attestation` |
| Append-only tamper-evident log | `kcp-sealed-lineage` |
| Ownership transfer with provenance | `kcp-transferable-record` |
| Regulated token with minter guard | `kcp-ktt-token` |

## Before live use

**Replace all synthetic values:**
- `[0xAAu8; 32]` blinds → CSPRNG-derived `[u8; 32]` from `rand::thread_rng()`
- `[0xAAu8; 32]` keys → real Schnorr x-only pubkeys from your wallet
- `nonce` → monotonically increasing value per subject/session — never reuse (reuse breaks AttestationRecord binding uniqueness)

This example runs entirely offline. No live node, no funds, no network.

## Post-quantum upgrade

Once `kcp-pq-anchor` is integrated, each step in this workflow can be
upgraded to PQ-safe on-chain verification:

| Step | JournalSpec binding |
|------|---------------------|
| [1] Paired attestation | `JournalSpec::PairedAttestation { attestation_id, spend_outpoint }` |
| [3] Sealed lineage APPEND | `JournalSpec::SealedLineage { lineage_id, seq, t_bucket }` |
| [4] Record transfer | `JournalSpec::TransferableRecord { record_id, new_controller }` |

The `kcp-pq-anchor` crate assembles the KIP-16 tag-0x21 redeem script from
your RISC Zero guest's proof output. Proof generation requires RISC Zero v3.0.5;
the library handles the hardest part — canonical `hashfn = OP_1 (0x51)` enforcement.

See `crates/kcp-pq-anchor/` and `book/src/patterns/pq-anchor.md`.
