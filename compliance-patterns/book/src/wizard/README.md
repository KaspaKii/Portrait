# kcp scaffold — generate a covenant project

The `kcp` CLI generates ready-to-compile Kaspa covenant projects from templates.

**Pre-production, unaudited.** Generated projects are testnet-only scaffolds.

## Usage

```sh
cargo run -p kcp -- scaffold <PATTERN> --workspace-path /path/to/kaspa-compliance-patterns [OPTIONS]
```

## Patterns

| Pattern | Command | What it generates |
|---------|---------|-------------------|
| P2SH multisig vault | `kcp scaffold vault` | k-of-n multisig lock+spend |
| DAA-height timelock | `kcp scaffold timelock` | Single-key CLTV covenant |
| Composite All | `kcp scaffold composite` | Timelock AND multisig combined |
| KCC20-shape token | `kcp scaffold ktt-token` | Mint → transfer → burn demo |
| Sealed lineage | `kcp scaffold sealed-lineage` | Append-only evidence chain |
| Transferable record | `kcp scaffold transferable-record` | Ownership-transfer chain |
| Paired attestation | `kcp scaffold paired-attestation` | Two-party mate-proof demo |

Until the library is published on crates.io (v0.2+), `--workspace-path` is
required so the generated `Cargo.toml` can reference the library via path
dependencies.

See the per-pattern pages for flags and generated-project details.
