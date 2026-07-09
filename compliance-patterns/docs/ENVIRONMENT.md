# Environment variables

These variables configure the **testnet examples** of `kaspa-compliance-patterns`
(`crates/*/examples/*.rs`). All examples are **testnet-first, v0, unaudited** — they
refuse to run against a non-testnet network, use no hardcoded private keys, and
automate no faucet funding. Set the variables inline on the `cargo run` line, e.g.

```text
KCP_NODE_URL=ws://localhost:17210 KCP_KEY_FILE=/path/wallet.key \
  cargo run -p kcp-vault --example testnet_evidence --features wrpc
```

> Examples that talk to a node are gated behind the `--features wrpc` flag; without
> it they compile to a stub that prints `requires --features wrpc` (or simply omit
> the node path). The two offline examples — `kcp-sealed-lineage --example
> auditor_verify` and `kcp-common --example covenant_auditor`'s read-only checks —
> read no keys and move no funds.

## Reference

| Variable | Used by (examples) | Required? | Default | Meaning |
|---|---|---|---|---|
| `KCP_NODE_URL` | every node example: `kcp-common`(`covenant_auditor`, `node_status`, `p2sh_roundtrip`, `reserve_covenant_live`), `kcp-ktt-token`(`testnet_evidence`), `kcp-paired-attestation`(`onchain_evidence`, `testnet_evidence`), `kcp-sealed-lineage`(`testnet_evidence`), `kcp-transferable-record`(`testnet_evidence`), `kcp-vault`(`testnet_evidence`, `onchain_evidence`, `composite_evidence`) | No | `ws://localhost:17210` | wRPC (Borsh) URL of the Kaspa testnet node to connect to. Several examples print a notice to stderr when falling back to the default. |
| `KCP_KEY_FILE` | `kcp-common`(`p2sh_roundtrip`, `reserve_covenant_live`), `kcp-ktt-token`(`testnet_evidence`), `kcp-paired-attestation`(`onchain_evidence`, `testnet_evidence`), `kcp-sealed-lineage`(`testnet_evidence`), `kcp-transferable-record`(`testnet_evidence`), `kcp-vault`(`testnet_evidence`, `onchain_evidence`, `composite_evidence`) | **Yes** (where used) | none | Path to the funded testnet wallet key file (mnemonic or raw-hex) that signs/funds the transactions. Hard error if unset — except in `reserve_covenant_live`, where it is not required when `KCP_DRY_RUN` is set. In `onchain_evidence`/`composite_evidence`/`reserve_covenant_live`/`p2sh_roundtrip` it is index 0 of a 2-of-2 or the sole signer. |
| `KCP_NEXT_KEY_FILE` | `kcp-paired-attestation`(`onchain_evidence`), `kcp-vault`(`onchain_evidence`, `composite_evidence`), `kcp-transferable-record`(`testnet_evidence`) | Mixed | none | Path to a **second, distinct** key file. **Required** in paired-attestation `onchain_evidence` (oracle B's key — needs no funds). **Optional** elsewhere: in vault `onchain_evidence`/`composite_evidence` it supplies the second 2-of-2 signer, falling back to BIP-44 index 1 of `KCP_KEY_FILE` when unset (which only differs for a mnemonic key file); in transferable `testnet_evidence` it is the "next controller", likewise falling back to index 1. |
| `KCP_NET_SUFFIX` | every node example (same set as `KCP_NODE_URL`) | No | `10` | Testnet numeric suffix passed to `NodeConfig::testnet` (e.g. `10` → testnet-10). Parsed as `u32`; an unparseable value silently falls back to the default. |
| `KCP_CAPTURE_JSON` | `kcp-common` (`covenant_auditor`, `reserve_covenant_live`) | **Yes** | none | Path to the silverc byte-capture JSON for the covenant (disclosed `state0`/`state1` scripts + the append sigscript template). Both examples hard-error if unset. The captures live **outside this repo** (they hold a testnet secret key); they are not shipped in the public tree. |
| `KCP_DRY_RUN` | `kcp-common`(`reserve_covenant_live`) | No | unset → live submit | Presence-checked only (any value, even empty, counts). When set, runs the **offline** proof — builds and engine-preflights the covenant transactions with no node and no funds, then exits before any live submit. |
| `KCP_COVENANT_ID` | `kcp-common`(`covenant_auditor`) | **Yes** | none | Hex-encoded consensus `covenant_id` of the lineage head to audit. Hex-decoded into a 32-byte `Hash` (whitespace trimmed); hard error if unset. |
| `KCP_TS` | `kcp-ktt-token`(`testnet_evidence`), `kcp-paired-attestation`(`testnet_evidence`), `kcp-sealed-lineage`(`testnet_evidence`), `kcp-transferable-record`(`testnet_evidence`), `kcp-vault`(`testnet_evidence`) | No | none (omitted) | ISO-8601 timestamp for reproducibility across runs. In paired-attestation, sealed-lineage, and transferable-record it is embedded into the record/genesis body when set, and omitted otherwise. In `kcp-ktt-token` and `kcp-vault` `testnet_evidence` it is currently read but not yet wired into the payload. |

## Notes

- **Defaults are read from code, not inferred.** Where a default is shown it is the
  value the example actually substitutes when the variable is unset.
- **"Required" means a hard error if absent** in the example(s) listed, unless a
  fallback is described in the Meaning column.
- The offline-only example `kcp-sealed-lineage --example auditor_verify` and the
  read-only checks in `kcp-common --example covenant_auditor` consume no key file —
  see each example's rustdoc header for its exact safety profile.
