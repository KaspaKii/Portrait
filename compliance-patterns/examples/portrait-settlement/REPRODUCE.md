# Reproducing the Portrait Settlement flagship (Phase A)

This documents how a third party rebuilds the CSCI vProg guest and gets the
**same `image_id`**, then confirms the on-chain covenant lock commits exactly
that program — so the covenant accepts a spend only if a proof for *that* vProg
verifies in-consensus.

**Pre-production · unaudited · testnet-only · MIT · Stichting Kii Foundation.**

## Pinned artifacts (observed 2026-06-28)

| Field | Value |
|---|---|
| Engine | `rusty-kaspa` tag `v2.0.0` (commit `90dbf07`) |
| vProg `image_id` (CSCI guest) | `c6ce0eda6084608aa529dafc06503f21b956f023d62ed1b7c88a278eeb0c5832` |
| `control_id` (recursion program; toolchain-stable) | `c9b08054994f542a6310b00d9b6fc6528ed7bb6f4ca5476a686847127cdfdc5b` |
| `covenant_id` (KovId = `blake2b256(redeem)`) | `869bfb4d98a318f308775f6410b2bc0eaa80865e6f7d7b047e341ae9bcbf1663` |
| P2SH covenant address (testnet-10) | `kaspatest:pzrfh76dnz333ucgwa0kgy9jhs824qyxtehh67cy0c6p46duhutxx3eh0m94m` |
| P2SH scriptPubKey | `OP_BLAKE2B <869bfb4d…> OP_EQUAL` (= `aa20869bfb4d…87`) |

## RISC Zero toolchain (must match to reproduce `image_id`)

The `image_id` (RISC Zero `CSCI_GUEST_ID`) is a content hash of the compiled
guest ELF. It is deterministic **given the same guest source and the same
RISC Zero toolchain**. The pin used:

| Tool | Version |
|---|---|
| `rzup` | `0.5.0` |
| `cargo-risczero` / `r0vm` | `3.0.5` |
| RISC Zero rust toolchain | `v1.91.1` (`~/.risc0/toolchains/v1.91.1-rust-*`) |
| `risc0-zkvm` / `risc0-build` (host + guest) | `3.0.5` (see `kii-csci-prover/Cargo.lock`, `methods/csci-guest/Cargo.lock`) |
| Guest target | `riscv32im-risc0-zkvm-elf` |

Install/select the toolchain:

```sh
cargo install cargo-risczero --version 3.0.5
rzup install rust 1.91.1        # the RISC Zero rust fork the guest builds with
```

## Rebuild the guest and verify `image_id`

> **Companion component.** The RISC Zero prover (`kii-csci-prover`) is maintained
> as a separate repository, not vendored here. The steps below assume it is
> checked out alongside this repo; if you don't have it, the ZK-proof steps can be
> skipped — the covenant-layer reproduction above stands on its own.

The guest lives in `kii-csci-prover/methods/csci-guest`. The
host prints `CSCI_GUEST_ID` as `image id`:

```sh
cd kii-csci-prover
touch methods/csci-guest/src/main.rs            # force a clean guest rebuild
RISC0_DEV_MODE=1 cargo run --release -p csci-prover | grep "image id"
# expect: image id: c6ce0eda6084608aa529dafc06503f21b956f023d62ed1b7c88a278eeb0c5832
```

A clean rebuild reproduces the same `image_id` (verified 2026-06-28: rebuilt
across three independent runs — dev, real pass-1, real pass-2 — all
`c6ce0eda…`).

## Confirm the covenant lock commits exactly this `image_id`

The on-chain covenant is a P2SH whose **redeem script** is the engine's own
tag-0x21 split (mirrors `R0Fields::p2sh_scripts()` in the engine at `90dbf07`):

```
redeem = <image_id> <control_id> <hashfn=01> <tag=21> OpZkPrecompile(0xa6)
```

so the redeem (71 bytes) literally contains the 32-byte `image_id`. The P2SH
lock is `OP_BLAKE2B <blake2b256(redeem)> OP_EQUAL`, i.e. it commits
`blake2b256(redeem) = covenant_id = 869bfb4d…`. Derive and check it:

```sh
cd kaspa-compliance-patterns
KCP_PROOF_DIR=kii-csci-prover/proof-export/succinct \
  cargo run -p portrait-settlement --bin covid --release
# prints redeem hex (containing image_id c6ce0eda…) and
#   covenant_id (blake2b256(redeem)): 869bfb4d98a318f308775f6410b2bc0eaa80865e6f7d7b047e341ae9bcbf1663
```

Because the redeem embeds `image_id`, the covenant address (which commits
`blake2b256(redeem)`) changes if and only if the vProg changes. A spend can only
satisfy this covenant by presenting a tag-0x21 proof whose `image_id` equals the
committed one — i.e. **only the pinned vProg can settle**. The offline
demonstrator proves a wrong `image_id` is rejected by the real engine:

```sh
KCP_PROOF_DIR=kii-csci-prover/proof-export/succinct \
  cargo run -p portrait-settlement --bin portrait-settlement
# [1/3] VALID proof ... ACCEPT ✓
# [2/3] tampered journal ... REJECT ✓
# [3/3] wrong image id  ... REJECT ✓   (only the pinned vProg can settle)
```

## Full real-instrument flow (two-pass covenant_id binding)

`covenant_id` must equal `blake2b256(redeem)`, but `redeem` contains `image_id`
and `control_id` (both known only from a build/proof). `control_id` is
toolchain-stable (`c9b08054…`), and `image_id` is fixed by the guest. So:

```sh
# Pass 1 — discover image_id + control_id (sentinel covenant_id):
cd kii-csci-prover
RISC0_DEV_MODE=0 cargo run --release -p csci-prover

# Derive the real covenant_id = blake2b256(redeem):
cd kaspa-compliance-patterns
CID=$(KCP_PROOF_DIR=kii-csci-prover/proof-export/succinct \
       cargo run -q -p portrait-settlement --bin covid --release)

# Pass 2 — real proof bound to the real covenant_id (journal[0..32] == CID):
cd kii-csci-prover
RISC0_DEV_MODE=0 KCP_COVENANT_ID=$CID cargo run --release -p csci-prover

# Live settle + on-chain negative control on TN10:
cd kaspa-compliance-patterns
KCP_NODE_URL=ws://127.0.0.1:17210 KCP_KEY_FILE=.secrets/tn10-portrait.key \
KCP_PROOF_DIR=kii-csci-prover/proof-export/succinct KCP_NET_SUFFIX=10 \
  cargo run -p portrait-settlement --bin settle --release
```

The guest's `new_state_hash` / `rule_hash` are pinned against the library types
by `crates/kcp-csci/tests/csci_smoke.rs::guest_state_and_rule_hash_match_library`,
so the in-zkVM state transition cannot silently drift from `kcp_csci::CsciState`.
