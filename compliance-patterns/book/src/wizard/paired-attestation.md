# kcp scaffold paired-attestation

Generates a two-party mutual attestation mate-proof demo. Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--subject-label` | `MyAttestation` | Subject label |
| `--out` | `./kii-covenants-out/my-paired-attestation` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold paired-attestation \
  --subject-label "ServiceAgreement" \
  --workspace-path $PWD \
  --out /tmp/my-pa
cd /tmp/my-pa && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-paired-attestation`; `publish = false`
- `src/main.rs` — two-party commit + mate proof + verify demo
- `tests/attestation_smoke.rs` — valid proof, tamper-A, tamper-B, determinism tests
- `README.md` — before-live-use callouts

## Before live use

Replace `[0x11u8; 32]` blind shares with CSPRNG-derived bytes. Never reuse a blind.
