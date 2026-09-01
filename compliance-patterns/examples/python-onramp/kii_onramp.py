"""kii_onramp — a Rosetta-style Python on-ramp for Kaspa covenants.

Three thin layers over the **experimental** Kaspa Python SDK silverscript
bindings (cross-checked byte-for-byte against our `silverc` toolchain — see
``docs/SDK-CROSSCHECK.md``):

  (a) scaffold(name, template|src)  -> a .sil source string
  (b) compile_covenant(sil, ctor)   -> {script, abi, state_layout, p2sh_address}
  (c) build_sig(compiled, fn, args) -> the signature (unlocking) script bytes

OFFLINE BY DESIGN. This module NEVER touches the network, a wallet, a private
key, or an RPC endpoint. It only turns source text + native Python constructor
values into deterministic script bytes and a P2SH address string. Broadcasting
lives elsewhere (``reproduce_settlement.py --broadcast``), gated off by default.

Status: pre-production, unaudited, testnet-only. The SDK silverscript bindings
are experimental (build-from-source; not on PyPI). See README.md.
"""

from __future__ import annotations

import importlib
import json
import os
import pathlib
import sys
from dataclasses import dataclass
from typing import Any

import p2sh  # local, pure-Python, offline P2SH derivation

# ── locate & import the experimental SDK silverscript extension ──────────────
#
# The built cdylib is imported as the module ``silverscript``. By default we
# look in a sibling ``sdk_pkg/`` dir or honour KCP_SDK_DIR (the scratch build
# from docs/SDK-CROSSCHECK.md). We do NOT vendor the SDK into this repo.

_DEFAULT_SDK_DIRS = [
    os.environ.get("KCP_SDK_DIR", ""),
    str(pathlib.Path(__file__).parent / "sdk_pkg"),
]


def _import_silverscript():
    last = None
    for d in _DEFAULT_SDK_DIRS:
        if d and d not in sys.path and pathlib.Path(d).is_dir():
            sys.path.insert(0, d)
        try:
            return importlib.import_module("silverscript")
        except Exception as e:  # noqa: BLE001 - report the original below
            last = e
    raise ImportError(
        "Could not import the experimental Kaspa SDK `silverscript` extension. "
        "Build it from source (see README.md / docs/SDK-CROSSCHECK.md) and put "
        "the built `silverscript.<ext>.so` on a dir named by $KCP_SDK_DIR or in "
        f"./sdk_pkg. Last error: {last!r}"
    )


silverscript = _import_silverscript()


# ── (a) scaffold ────────────────────────────────────────────────────────────

# A minimal, self-contained covenant template. `{name}` is substituted in.
_MINIMAL_TEMPLATE = """\
pragma silverscript ^0.1.0;

// Scaffolded by kii_onramp.scaffold() — a minimal owner-authorised covenant.
contract {name}(int max_ins, int max_outs, pubkey owner, int seq) {{
    pubkey owner = owner;
    int seq = seq;

    #[covenant(binding = cov, from = max_ins, to = 1, mode = transition)]
    function step(State[] prev_states, sig auth) : (State) {{
        require(checkSig(auth, prev_states[0].owner));
        return({{ owner: prev_states[0].owner, seq: prev_states[0].seq + 1 }});
    }}
}}
"""


def scaffold(name: str, template: str | None = None, src: str | None = None) -> str:
    """Return SilverScript (.sil) source for a covenant called ``name``.

    Precedence:
      * ``src`` — an explicit path to an existing ``.sil`` file (e.g. a covenant
        from the portrait library); its contents are returned verbatim.
      * ``template`` — a template string containing ``{name}`` to substitute,
        OR the literal ``"minimal"`` to use the built-in minimal template.
      * neither — the built-in minimal template.

    This is pure text generation; nothing is compiled or broadcast here.
    """
    if src is not None:
        path = pathlib.Path(src)
        if not path.is_file():
            raise FileNotFoundError(f"scaffold(src=...): no such .sil file: {src}")
        return path.read_text()
    if template is None or template == "minimal":
        return _MINIMAL_TEMPLATE.format(name=name)
    return template.format(name=name)


# ── (b) compile_covenant ────────────────────────────────────────────────────


@dataclass
class Compiled:
    """A compiled covenant + its on-chain identity."""

    name: str
    compiler_version: str
    script: bytes
    abi: list  # FunctionAbiEntry objects from the SDK
    state_layout: Any  # (start, len)
    p2sh_address: str
    _cc: Any  # underlying SDK CompiledContract (for build_sig)

    def as_dict(self) -> dict:
        return {
            "name": self.name,
            "compiler_version": self.compiler_version,
            "script": self.script,
            "script_hex": self.script.hex(),
            "abi": [e.name for e in self.abi],
            "state_layout": self.state_layout,
            "p2sh_address": self.p2sh_address,
        }


def _scalar_byte_guard(ctor_args, sil_src: str) -> None:
    """Raise a clear, actionable error if the covenant has a scalar ``byte``
    constructor parameter — the known acceptance gap in the experimental SDK
    bindings (``byte[N]`` arrays are fine; a bare ``byte`` is rejected for every
    Python value type). See docs/SDK-CROSSCHECK.md finding #2 (KTT)."""
    import re

    # Find the contract/covenant constructor parameter list and scan its params.
    m = re.search(r"\b(?:contract|covenant)\s+\w+\s*\(([^)]*)\)", sil_src)
    if not m:
        return
    params = m.group(1)
    # A scalar byte param looks like `byte name` but NOT `byte[...] name`.
    if re.search(r"\bbyte\s+(?!\[)\w+", params):
        raise ValueError(
            "scalar `byte` constructor parameter is unsupported by the "
            "experimental Kaspa SDK silverscript bindings (a bare `byte` is "
            "rejected for every Python value type). Use a `byte[N]` array, or "
            "compile this covenant with `silverc` instead. "
            "See docs/SDK-CROSSCHECK.md (KTT scalar-byte gap)."
        )


def compile_covenant(sil_src: str, ctor_args=None, *, prefix: str = "kaspatest") -> Compiled:
    """Compile SilverScript source into a :class:`Compiled` (script + abi +
    state_layout + P2SH address). ``ctor_args`` are **native** Python values
    (int / bool / bytes / list / dict) in constructor-parameter order; pass
    ``None`` for a no-arg constructor.

    Wraps ``silverscript.compile``. Pure / offline. ``prefix`` selects the
    address network — ``kaspatest`` (default) or ``kaspa``. This on-ramp is
    testnet-first; mainnet is never used by the bundled drivers.
    """
    _scalar_byte_guard(ctor_args, sil_src)
    try:
        cc = silverscript.compile(sil_src, ctor_args)
    except silverscript.SilverScriptError as e:
        raise ValueError(f"silverscript compile failed: {e}") from e
    script = bytes(cc.script)
    return Compiled(
        name=cc.contract_name,
        compiler_version=cc.compiler_version,
        script=script,
        abi=list(cc.abi),
        state_layout=cc.state_layout,
        p2sh_address=p2sh.p2sh_address(script, prefix=prefix),
        _cc=cc,
    )


# ── (c) build_sig ───────────────────────────────────────────────────────────


def build_sig(compiled: Compiled, fn: str, args=None, *, is_leader: bool = False) -> bytes:
    """Build the signature (unlocking) script for a covenant entrypoint.

    ``fn`` is the bare covenant function name (e.g. ``"release"``); ``is_leader``
    selects the leader vs. delegate variant. ``args`` are native Python values
    matching the entrypoint's ABI inputs (omit / ``None`` for the delegate path,
    which takes no arguments).

    NOTE: the *leader* path takes a real ``sig`` value (a Schnorr signature over
    the transaction sighash) which can only be produced with a private key while
    building an actual spend — that belongs to the gated broadcast path, not
    here. The *delegate* path needs no signature and builds fully offline, which
    is what the offline reproduction exercises.

    Pure / offline: this only assembles script bytes.
    """
    cc = compiled._cc
    try:
        return bytes(cc.build_sig_script_for_covenant_decl(fn, args, is_leader=is_leader))
    except silverscript.SilverScriptError as e:
        raise ValueError(f"build_sig failed for {fn!r} (is_leader={is_leader}): {e}") from e


# ── ctor-args helper ────────────────────────────────────────────────────────


def native_ctor_from_silverc_json(path) -> list:
    """Translate a ``silverc`` ``--ctor`` tagged-JSON file into the native
    Python values that the SDK ``compile()`` expects. Lets the same constructor
    inputs drive both toolchains for a byte-identity check."""
    raw = json.loads(pathlib.Path(path).read_text())
    out = []
    for node in raw:
        k = node["kind"]
        if k == "array" and all(x.get("kind") == "byte" for x in node["data"]):
            out.append(bytes(int(x["data"]) for x in node["data"]))
        elif k in ("int", "byte"):
            out.append(int(node["data"]))
        elif k == "bool":
            out.append(bool(node["data"]))
        elif k == "string":
            out.append(node["data"])
        elif k == "array":
            out.append([int(x["data"]) for x in node["data"]])
        else:
            raise ValueError(f"unknown ctor kind {k!r}")
    return out


# ── tiny self-demo (offline) ────────────────────────────────────────────────

if __name__ == "__main__":
    here = pathlib.Path(__file__).parent
    src = scaffold("DemoCovenant")  # minimal template
    print("scaffold(DemoCovenant) ->", len(src), "chars of .sil")
    # minimal template ctor: (max_ins, max_outs, pubkey owner=byte[32], seq)
    z32 = bytes(32)
    comp = compile_covenant(src, [1, 1, z32, 0])
    d = comp.as_dict()
    print("compile_covenant ->")
    print("  name           :", d["name"])
    print("  compiler_version:", d["compiler_version"])
    print("  script         :", len(d["script"]), "bytes")
    print("  abi            :", d["abi"])
    print("  state_layout   :", d["state_layout"])
    print("  p2sh_address   :", d["p2sh_address"])
    sig = build_sig(comp, "step", None, is_leader=False)  # delegate path, offline
    print("build_sig(step, delegate) ->", len(sig), "bytes:", sig.hex())
    print("\nOFFLINE OK — no network, no wallet, no key touched.")
