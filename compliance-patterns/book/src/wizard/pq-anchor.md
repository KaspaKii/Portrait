# kcp scaffold pq-anchor

Generates a KIP-16 tag-0x21 post-quantum credential anchor script assembly demo.
This scaffold assembles the redeem script only — proof generation requires RISC Zero v3.0.5.
Runs entirely offline.

**Pre-production, unaudited, testnet-only.**

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--out` | `./kii-covenants-out/my-pq-anchor` | Output directory |
| `--workspace-path` | *(required)* | Absolute path to `kaspa-compliance-patterns` workspace root |

## Example

```sh
cargo run -p kcp -- scaffold pq-anchor \
  --workspace-path $PWD \
  --out /tmp/my-pq
cd /tmp/my-pq && cargo test
```

## What is generated

- `Cargo.toml` — path dep on `kcp-pq-anchor`; `publish = false`; no RISC Zero dependency
- `src/main.rs` — synthetic proof field setup, `build_pq_anchor_redeem` call, hex script output, sigOpCount display
- `tests/pq_anchor_smoke.rs` — script assembles, `0x51` (OP_1) present for hashfn, sigOpCount=255, JournalSpec determinism
- `README.md` — before-live-use callouts

## Before live use

**`hashfn` MUST be pushed as `OP_1` (opcode `0x51`), never as a data push `[0x01, 0x01]`.**
The library enforces this internally; do not hand-assemble the field.
Replace all synthetic `[0xNNu8; 32]` fields with real RISC Zero guest output.
The real `seal` is approximately 222 KB — the 128-byte seal in the scaffold is synthetic.
Set `sigOpCount = 255` on the spending transaction.
Use the `JournalSpec` variant matching your pattern:
`PairedAttestation`, `SealedLineage`, `TransferableRecord`, or `Custom`.
