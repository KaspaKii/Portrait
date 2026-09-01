# Build your first covenant in 10 minutes

The quick-start lives in the top-level [README](./README.md#build-your-first-covenant-in-10-minutes).

The shortest path from a fresh checkout to "the real Kaspa script engine
accepted my covenant spend" is the **`examples/hello-vault`** standalone
project, which:

1. Builds a 2-of-2 multisig spending condition from deterministic throwaway keys.
2. Compiles it to a real Kaspa redeem script.
3. Constructs a synthetic spend transaction + signed satisfier.
4. Runs the spend through the real `rusty-kaspa` script engine via
   `kcp_common::p2sh::verify_p2sh_spend_offline`.

No live node, no funds, no network — but the same engine path that
produced `[KCP-VT-002]` on testnet-10.

Run from the repo root:

```sh
cargo run --manifest-path examples/hello-vault/Cargo.toml
```

See `examples/hello-vault/README.md` for the full walkthrough.
