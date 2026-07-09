# python-onramp — a Python on-ramp for Kaspa covenants

A small, Rosetta-style Python wrapper over the **experimental** Kaspa Python SDK
silverscript bindings. It lets you scaffold a covenant, compile it to locking-
script bytes + a P2SH address, and build the spend (unlocking) script — all from
Python, all offline.

The point: the SDK's silverscript compiler is an **independent second
implementation** of the one we ship (`silverc`). This example proves that for
real covenants the two toolchains emit **byte-for-byte identical** locking
scripts — and therefore the same on-chain covenant identity / P2SH address.

> **Scope & honesty box**
> - **Compile + sig layers: real and verifiable now.** The SDK locking-script
>   bytes are asserted **byte-identical** to our `silverc` output for every
>   bundled covenant, and the derived P2SH address matches the pinned engine
>   (`rusty-kaspa v2.0.0` = `90dbf07`) exactly.
> - **Live broadcast: DEFERRED.** The TN10 settlement submit is currently
>   blocked by a transient server-side rate-limit on our endpoint. It is
>   **not-yet-run**; we do **not** fabricate a txid. The `--broadcast` path is a
>   gated scaffold that documents where the live submit attaches and exits
>   without submitting.
> - **Experimental SDK.** The Kaspa Python SDK silverscript bindings are
>   build-from-source and not on PyPI; their API and compiler output may change.
> - **KTT scalar-byte gap.** The experimental bindings reject a scalar `byte`
>   constructor parameter (e.g. our `kcp-ktt-token`); `byte[N]` arrays are fine.
>   `compile_covenant` detects this and raises a clear, actionable error rather
>   than a cryptic one. Compile KTT-shaped covenants with `silverc` instead.
> - **Testnet-only, never mainnet, no key handling.** Nothing here touches
>   kaspanet. The committed code never broadcasts on import or default run. The
>   gated broadcast path is testnet-only, refuses mainnet, and reads the signing
>   key from the user environment — it is never hardcoded or printed.
>
> Pre-production · unaudited · testnet-first. See `docs/SDK-CROSSCHECK.md`.

## Layout

```
python-onramp/
  kii_onramp.py            # (a) scaffold (b) compile_covenant (c) build_sig — NO broadcast
  p2sh.py                  # pure-Python Kaspa P2SH address derivation (matches the engine)
  reproduce_settlement.py  # offline covenant-side reproduction + gated/deferred --broadcast
  covenants/               # representative covenants + silverc reference bytes + ctor args
    CsciInstrument.sil  CsciInstrument.silverc.hex  CsciInstrument_ctor.json
    Escrow.sil          Escrow.silverc.hex          Escrow_ctor.json
    MultisigTreasury.sil MultisigTreasury.silverc.hex MultisigTreasury_ctor.json
```

## Install — build the experimental SDK silverscript extension from source

The bindings are not on PyPI. Build the `silverscript` extension from source
(this is the same procedure recorded in `docs/SDK-CROSSCHECK.md`):

```sh
git clone https://github.com/kaspanet/kaspa-python-sdk.git
cd kaspa-python-sdk
python3 -m venv env && . env/bin/activate && pip install maturin

cargo update -p cc                 # fixes the stale cc 1.2.61 vs Rust 1.96 build break

export PYO3_PYTHON="$PWD/env/bin/python"
# macOS only — let the extension resolve Python symbols at load time:
export RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup"

cargo build -p kaspa-python-sdk-silverscript --lib --features extension-module --release
```

Then make the built library importable as `silverscript`. Copy/symlink the built
`target/release/libsilverscript.dylib` (Linux: `.so`) to a directory as
`silverscript<EXT_SUFFIX>.so` (the suffix from
`python3 -c "import sysconfig;print(sysconfig.get_config_var('EXT_SUFFIX'))"`,
e.g. `silverscript.cpython-314-darwin.so`) and point the on-ramp at it:

```sh
export KCP_SDK_DIR=/path/to/dir/containing/silverscript.<ext>.so
```

`kii_onramp` looks for the extension in `$KCP_SDK_DIR`, then a sibling
`./sdk_pkg/`, then the documented scratch-build location.

`silverc` (our reference toolchain) only matters if you want
`reproduce_settlement.py` to regenerate the reference bytes instead of using the
committed `*.silverc.hex` files — the bundled references mean it runs without
`silverc` installed.

## Usage

### Offline reproduction (default — no network, no wallet, no key)

```sh
python3 reproduce_settlement.py                 # all bundled covenants
python3 reproduce_settlement.py CsciInstrument  # one covenant
```

It compiles each covenant via the SDK, prints the locking script + P2SH address,
builds the (delegate-path) spend sig script, and **asserts** the SDK bytes equal
the `silverc` reference bytes.

### Library use

```python
import kii_onramp

src  = kii_onramp.scaffold("MyCovenant")                 # or src="/path/to/lib/Foo.sil"
comp = kii_onramp.compile_covenant(src, [1, 1, b"\\x00"*32, 0])
print(comp.p2sh_address, len(comp.script), "bytes")
sig  = kii_onramp.build_sig(comp, "step", None, is_leader=False)  # delegate path, offline
```

`compile_covenant` returns a `Compiled` with `.script` (bytes), `.abi`,
`.state_layout`, and `.p2sh_address`. The leader spend path takes a real Schnorr
`sig` (built only while signing an actual spend), so offline reproduction uses
the keyless delegate path.

### Gated broadcast (DEFERRED — does not submit)

```sh
KCP_TESTNET=1 python3 reproduce_settlement.py CsciInstrument --broadcast
```

Without `KCP_TESTNET=1` it refuses. With it, it prints the live-path plan and
exits **without** building or submitting a transaction (deferred — rate-limit).
`KCP_NETWORK` containing `main` is rejected. The signing key would be read from
`$KCP_WALLET_KEY_FILE` at spend time and is never printed.

## Verified

Run on `python-onramp/covenants` (engine `rusty-kaspa v2.0.0` = `90dbf07`):

| Covenant | locking-script bytes | SDK vs silverc | P2SH vs engine |
|---|---|---|---|
| CsciInstrument | 524 | identical | identical |
| Escrow | 1133 | identical | identical |
| MultisigTreasury | 479 | identical | identical |

License: MIT (Stichting Kii Foundation).
