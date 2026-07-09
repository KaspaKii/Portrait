# Next step: post-quantum credential anchors via KIP-16

This note scopes a concrete post-quantum extension path for the library, grounded
in a separately-validated Kii project (`kii-ml-dsa`). It is a design note, not
a claim of completed work; no PQ anchor code is shipped with this library today.

## The gap

`kcp-common::cryptography` currently exposes Schnorr-based signing and SHA-256
hashing only. Every credential anchor in this library (vault, paired-attestation,
sealed-lineage, transferable-record) ultimately roots in a secp256k1 Schnorr
signature. That signing path is **not post-quantum-safe**: Shor's algorithm
running on a sufficiently capable quantum computer would break it.

The on-chain verification side has a well-defined PQ upgrade path via KIP-16,
which is live on testnet-10 today and scheduled for Kaspa mainnet activation at
DAA 474,165,565 (≈ 30 Jun 2026). This note scopes what that path looks like.

## The KIP-16 / OpZkPrecompile building block

**`OpZkPrecompile` (`0xa6`)** verifies a zero-knowledge proof on-chain. Two
tags are defined:

| Tag | Scheme | PQ-safe? |
|-----|--------|----------|
| `0x20` | Groth16 (BN254) | No — relies on pairings |
| `0x21` | RISC Zero Succinct STARK | **Yes** — FRI + Poseidon2, no pairings |

For PQ work, tag `0x21` is the correct choice. A succinct STARK proof from RISC
Zero `v3.0.5` verifies on-chain using ~25M script units; this requires
`sigOpCount = 255` (the u8 maximum) in the spending transaction.

### Tag-0x21 stack format (deployed TN10 master, bottom→top)

```
[claim, control_index, control_digests, seal, journal, image_id, control_id, hashfn]
tag (0x21)
OpZkPrecompile (0xa6)
```

Field notes:
- `hashfn` MUST be `1` (poseidon2) — pushed as `OP_1` (canonical; NOT `OP_DATA_1 0x01`)
- `journal` = `sha256(journal_bytes)` (the host precomputes this)
- `seal` ≈ 222–223 KB for a succinct STARK
- `control_id` is required on the deployed master (earlier 7-field drafts omit it)

Source: verified against `rusty-kaspa` master (`github.com/kaspanet/rusty-kaspa`)
and confirmed by multiple on-chain executions in `kii-ml-dsa`.

## What Kii has validated externally (kii-ml-dsa, testnet-10)

A separate Kii project (`kii-ml-dsa`) has demonstrated the full
RISC Zero → KIP-16 pipeline live on testnet-10. The following on-chain receipts
confirm the pipeline is real and functional:

| Scheme | Standard | TN10 txid |
|--------|----------|-----------|
| ML-DSA-44 / Dilithium | FIPS-204 | `b7add3df69d54ca96b171771d92cad300d231504ed80ea55a9873edb08094eca` |
| SLH-DSA / SPHINCS+ | FIPS-205 | `01f747bc0e559bb7080d3e77dae8a4c1902545da1d97b55a9b37be90870f2342` |
| FN-DSA / Falcon-512 | FIPS-206 draft | `aec459a84600caa13f6ce2c13104285c1b55c8cb8a986340bf5c135b65f15bc3` |
| ML-DSA-44 PQ-bound spend | — | `e33a1332e72266f361b6276449c6b3273bbb03a6fe1caff4d7db71f56806ff90` |

These txids are from the `kii-ml-dsa` project, **not** from this library.
Testnet-10 evidence is perishable; the txids may not resolve if the testnet
resets. They are cited here as design provenance only.

**Scope note:** native hash-based PQ on Kaspa (XMSS-in-script) was pioneered
independently by others; Kii's lane is the complementary lattice/ZK route. We do
not claim "first PQ on Kaspa."

## The PQ anchor pattern

A **PQ-secured credential** in this library's context would work as follows:

1. **Off-chain**: generate a NIST PQ keypair (ML-DSA-44 is the recommended
   first choice: lattice-based, FIPS-204, shortest key+sig in the lattice family).
2. **RISC Zero guest**: the guest verifies the ML-DSA signature over
   `sha256(tx_commit)`, where `tx_commit` is reconstructed from the KIP-10
   introspection opcodes inside the guest.
3. **Prove**: `default_prover().prove(env, ELF)` → composite receipt →
   `.compress(&ProverOpts::succinct())` → succinct STARK.
4. **On-chain**: construct a KIP-16 tag-0x21 redeem script, push the 8 proof
   fields, verify on-chain. The covenant rebuilds `tx_commit` via introspection
   (e.g. `OpTxOutputAmount 0xc2`, `OpTxOutputSpk 0xc3`, `OpCat 0x7e`,
   `OpSHA256 0xa8`) and checks the ZK journal matches.

**Important**: SilverScript has no ZK-verify builtin (confirmed in the
upstream source). This pattern requires hand-rolling the raw KIP-16 opcode
sequence — it is NOT a SilverScript one-liner.

## Per-pattern upgrade scope

| Pattern | Current anchor | PQ upgrade |
|---------|---------------|------------|
| `kcp-vault` | secp256k1 Schnorr multisig | ML-DSA multisig inside zkVM; covenant checks ZK journal |
| `kcp-paired-attestation` | CSFS (secp256k1) | ML-DSA paired sig inside zkVM; journal carries both attestation_ids |
| `kcp-sealed-lineage` | SHA-256 blinded commitment | Add ML-DSA append-authorization inside zkVM; lineage_id in journal |
| `kcp-transferable-record` | SHA-256 unblinded commitment | Add ML-DSA transfer-authorization inside zkVM; record_id in journal |

## Integration point in this library

The natural home for PQ anchor helpers is a new `kcp-common::pq_anchor` module
(or a dedicated `kcp-pq-anchor` crate if the proof-pipeline surface is large).
It would expose:

- A `PqAnchorScript` builder: takes RISC Zero proof fields → produces the
  KIP-16 tag-0x21 redeem script bytes.
- A `JournalSpec` type: canonically encodes the per-pattern binding (e.g.
  `subject_id || spend_outpoint` for paired-attestation).
- Helper for `sigOpCount = 255` covering the ~25M script-unit budget.

The guest programs themselves would live in a `guests/` directory, compiled
by `risc0-build`, with image IDs hardcoded as constants (for soundness).

## What unblocks this

1. **Toccata mainnet activation** (DAA 474,165,565, ≈ 30 Jun 2026) — KIP-16 will
   be live on mainnet; testnet evidence becomes mainnet-backed.
2. **RISC Zero v3.0.5 pinned** — the proof pipeline is version-sensitive;
   pin `risc0-zkvm` + `risc0-build` to `3.0.5` in any future `kcp-pq-anchor` crate.
3. **No SilverScript dependency** — hand-rolled opcode sequence; no `silverc`
   compile step required.

## Status of this note

This is a design note grounded in verified external evidence. No PQ anchor code
is in this library today. The path is real, the opcodes are live, and the
pipeline design is validated. Authoring a `kcp-pq-anchor` crate is a possible
future addition to the library.

See also:
- [`docs/NEXT-STEPS-introspection-enforcement.md`](NEXT-STEPS-introspection-enforcement.md) — KIP-10 state-continuity enforcement
- [`docs/NEXT-STEPS-covenant-live-deploy.md`](NEXT-STEPS-covenant-live-deploy.md) — live covenant-id-bound deployment
- [`KNOWN-ISSUES.md`](../KNOWN-ISSUES.md) §PQ gap for the explicit library limitation record
