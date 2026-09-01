# Independent toolchain cross-check — Kaspa Python SDK silverscript bindings

*Status: offline compile-only cross-check, 2026-06-29. Pre-production, unaudited,
testnet-only. The Kaspa Python SDK silverscript bindings are **experimental**
(build-from-source; not yet on PyPi). Nothing here was broadcast; no wallet/key/RPC
was touched; our repositories were not modified (the SDK was built in a scratch dir).*

## Why this matters

Kaspanet shipped experimental **silverscript bindings in the Kaspa Python SDK**
(compile silverscript + build signature scripts natively in Python). That gives us an
**independent second implementation** of the silverscript compiler — a chance to test
whether our covenants are toolchain-portable and whether two independently-maintained
compilers agree on the exact locking-script bytes. "An independent implementation
agrees" is precisely the cypherpunk-grade evidence this library trades on.

## Result — byte-for-byte agreement

Built the SDK's `silverscript` extension from source (git rev `d2d03b3`, pkg `2.0.1`;
pins silverscript-lang `faaa074`) and compiled representative covenants through **both**
toolchains with identical deterministic constructor inputs. Our `silverc` is
silverscript-lang `2c46231`.

| Covenant | Locking-script bytes | silverc vs SDK |
|---|---|---|
| counter (SDK's own example) | 60 | **identical** |
| `state/CsciInstrument` (flagship; KIP-20 binding) | 524 | **identical** |
| `finance/Escrow` | 1133 | **identical** |
| `governance/MultisigTreasury` | 479 | **identical** |
| `finance/RoyaltySplit` | 650 | **identical** |
| `vprog/ComplianceCredential` (covenant side) | 631 | **identical** |

**6 covenants, byte-for-byte identical, 0 divergences**; both report
`compiler_version 0.1.0`.

## Honest findings (not failures)

1. **Revision skew (bounded).** The SDK pins silverscript-lang `faaa074`; our `silverc`
   is `2c46231` — verified to be **exactly one commit behind**. The single intervening
   commit (PR #131, "allow contract state to include different types") touches the
   compiler, so byte divergence is *theoretically possible only* for contracts that
   exercise **mixed-type output state** — none of our covenants do, hence the identical
   output above.
2. **Acceptance gap in the experimental bindings — KTT.** Our `kcp-ktt-token` covenant
   (`ktt.sil`) does **not** compile through the SDK: a scalar `byte` constructor
   parameter is rejected for every Python value type. Reproduced on a minimal
   `contract T(byte x)`; `byte[N]` array constructors are accepted fine. This is a
   limitation of the **experimental SDK bindings**, not a defect in our covenant
   (`silverc` compiles it). Recorded as an interoperability finding.
3. **Build note.** The SDK's official `./build-release` currently fails on a stale
   transitive `cc 1.2.61` pin under Rust 1.96 (`E0583`/`E0599`); `cargo update -p cc`
   (→ 1.2.65) fixes it. This is in the SDK's lockfile, not our covenants.

## Reproduce

```sh
git clone https://github.com/kaspanet/kaspa-python-sdk.git && cd kaspa-python-sdk
python3 -m venv env && . env/bin/activate && pip install maturin
cargo update -p cc            # fixes the cc 1.2.61 vs Rust 1.96 build break
export PYO3_PYTHON="$PWD/env/bin/python"
export RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup"   # macOS
cargo build -p kaspa-python-sdk-silverscript --lib --features extension-module --release
# import the built libsilverscript as silverscript<EXT_SUFFIX>, then:
#   silverscript.compile(open('CsciInstrument.sil').read(), <native ctor args>).script.hex()
# compare against:  silverc CsciInstrument.sil --ctor CsciInstrument_ctor.json -c   (the "script" bytes)
```

## Scope

Offline compile-only. The SDK's `RpcClient.submit_transaction` (broadcast) path was
**not** exercised — consistent with this project's discipline (nothing to kaspanet; any
tx work is TN10-testnet only). The byte-identity covers **code generation**, the part
that determines the on-chain covenant address; it does not by itself re-prove the live
settlements (those are in `examples/portrait-settlement/PROVENANCE.json`).
