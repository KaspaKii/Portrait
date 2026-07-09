# hello-vault

The shortest path from a fresh `kaspa-compliance-patterns` checkout to
"the real Kaspa script engine accepted my covenant spend."

Locks a synthetic UTXO under a **2-of-2 multisig P2SH covenant script**, then
spends it back by satisfying the script. Runs **entirely offline** — no live
node, no funds, no network — but uses the **real `rusty-kaspa` script engine**
via `kcp_common::p2sh::verify_p2sh_spend_offline`.

## Run

```sh
# from the repo root
cargo run --manifest-path examples/hello-vault/Cargo.toml
```

Expected output:

```
hello-vault — offline P2SH multisig covenant demo
(no live node, no funds, no network — real rusty-kaspa engine)

[1/5] built 2-of-2 multisig spending condition
[2/5] compiled to N-byte redeem script
[3/5] built synthetic spend tx + P2SH-locked UTXO (200000000 sompi)
[4/5] signed with both keys + built P2SH satisfier
[5/5] ✓ PASSED — real rusty-kaspa script engine accepted the spend

You just ran the same engine path that produced [KCP-VT-002]
on testnet-10. See crates/kcp-vault/README.md for variants:
  - MultiSig (k-of-n)
  - TimelockHeight / TimelockUnixSeconds (CLTV)
  - Composite Any/All (branch-selected P2SH)
```

## What it actually does

1. **Builds a 2-of-2 multisig spending condition** from two deterministic
   throwaway keypairs (`0x11…`, `0x22…`). These are NOT secrets — they
   would never be used for real funds.
2. **Compiles the condition to a real Kaspa redeem script** via
   `kcp_vault::script::compile_condition`. The bytes that come out are the
   same bytes the library would produce for a live vault.
3. **Constructs a synthetic spend transaction + a corresponding P2SH-locked
   UTXO entry.** The transaction is well-formed but never broadcast; the
   outpoint references a UTXO that does not exist on any network.
4. **Signs with both keys + builds the satisfier** in key order. (Kaspa's
   `OP_CHECKMULTISIG` does NOT consume a dummy element — unlike Bitcoin —
   so the satisfier is just `[sig_for_pk1, sig_for_pk2]`.)
5. **Runs the spend through the real `rusty-kaspa` script engine** via
   `verify_p2sh_spend_offline`. The engine performs genuine signature
   verification + multisig threshold check + resource-meter accounting.
   A pass means the same code that runs on testnet-10 accepted the spend.

## Why this is a useful starting point

- **No environment setup beyond Rust + cargo.** No node, no wallet, no testnet
  faucet. The example is reproducible on a fresh laptop in under 10 minutes.
- **Same engine as live.** The `verify_p2sh_spend_offline` function is the
  exact preflight every live vault submission runs through before any RPC.
  An offline pass means the live path would accept it too.
- **Forkable starter.** Copy `examples/hello-vault/` into a new directory,
  point `kcp-common` and `kcp-vault` at crates.io once they publish (v0.2+),
  and you have a standalone project. Until then, path-deps inside this
  repo work.

## Next steps

After this runs, the natural progression:

1. **Read [`crates/kcp-vault/README.md`](../../crates/kcp-vault/README.md)** —
   variants (timelock, composite Any/All, branch-selected P2SH).
2. **Try the live testnet example** —
   `crates/kcp-vault/examples/onchain_evidence.rs` — same flow against a
   real testnet-10 node + a funded wallet. See
   [`docs/ENVIRONMENT.md`](../../docs/ENVIRONMENT.md) for the env vars.
3. **Read other patterns** — `kcp-paired-attestation` (two-party datasig
   via CSFS), `kcp-sealed-lineage` (append-only state-continuity covenant
   live as `[KCP-SL-003]`), `kcp-ktt-token` (KCC20-shape token live as
   `[KCP-KTT-003]`), `kcp-transferable-record` (live as `[KCP-TR-003]`).

## Provenance

This example is adapted directly from the kcp-vault unit test
`multisig_2of2_lock_spend_executes_on_engine` in
[`crates/kcp-vault/src/onchain.rs`](../../crates/kcp-vault/src/onchain.rs#L915).
Same engine path, same assertion, exposed as a forkable example.

## Status

**v0 — unaudited — testnet first.** This is a *demonstration* of the engine
preflight path; it is not a security review of your covenant. See
[`SECURITY.md`](../../SECURITY.md) and the library `KNOWN-ISSUES.md` for the
full caveats catalogue.
