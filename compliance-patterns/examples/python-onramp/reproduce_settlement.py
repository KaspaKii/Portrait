#!/usr/bin/env python3
"""reproduce_settlement.py — OFFLINE reproduction of the *covenant side* of a
TN10 settlement, driven entirely from Python via the experimental Kaspa SDK
silverscript bindings.

What it does (DEFAULT, offline, no network / no wallet / no key):
  1. Compile a real library covenant through the SDK (CsciInstrument by default,
     also Escrow / MultisigTreasury).
  2. Print the locking (redeem) script + its P2SH covenant address.
  3. Build a covenant spend signature (unlocking) script — the delegate path,
     which needs no private key, so it builds fully offline.
  4. ASSERT the SDK locking-script bytes are **byte-identical** to our
     `silverc` reference output (the toolchain we ship). This is the load-
     bearing claim: two independent compilers agree on the exact on-chain
     covenant bytes, hence the same P2SH address / covenant identity.

What it does NOT do by default: broadcast. There is a clearly-marked
`--broadcast` path that *would* lock + spend a real UTXO on Kaspa testnet-10
via the SDK's RpcClient — but it is GATED OFF, testnet-only, never mainnet, and
reads the signing key from the user environment (never hardcoded, never printed).

  >>> LIVE BROADCAST IS CURRENTLY DEFERRED <<<
  The settlement path is blocked by a transient server-side rate-limit on our
  TN10 endpoint. The compile + sig layers below are real and verifiable now;
  the on-chain submit is NOT-YET-RUN. We do not fabricate a txid.

Usage:
  python3 reproduce_settlement.py                 # offline, all covenants
  python3 reproduce_settlement.py CsciInstrument  # offline, one covenant
  KCP_TESTNET=1 python3 reproduce_settlement.py --broadcast   # gated; deferred

Status: pre-production, unaudited, testnet-only.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys

import kii_onramp

HERE = pathlib.Path(__file__).parent
COV = HERE / "covenants"

COVENANTS = ["CsciInstrument", "Escrow", "MultisigTreasury"]

# Each covenant's offline-buildable spend: (bare fn name, is_leader). The
# delegate path takes no `sig` arg, so it builds without any key material.
SPEND = {
    "CsciInstrument": ("settle", False),
    "Escrow": ("release", False),
    "MultisigTreasury": ("spend", False),
}


def _silverc_reference_hex(name: str, sil_path: pathlib.Path, ctor_path: pathlib.Path) -> str:
    """Return the silverc locking-script hex for `name`.

    Prefer the committed `<name>.silverc.hex` reference next to the covenant.
    If absent and `silverc` is on PATH, regenerate it from source so the
    byte-identity check is against a freshly-built silverc artifact.
    """
    ref = COV / f"{name}.silverc.hex"
    if ref.is_file():
        return ref.read_text().strip().lower()
    # Fallback: call silverc directly (still offline — pure compilation).
    if not _have_silverc():
        raise FileNotFoundError(
            f"no {ref.name} and `silverc` not on PATH — cannot get the reference "
            "bytes to compare against."
        )
    import json

    out = subprocess.run(
        ["silverc", str(sil_path), "--ctor", str(ctor_path), "-c"],
        capture_output=True,
        text=True,
        check=True,
    )
    doc = json.loads(out.stdout)
    return bytes(doc["script"]).hex().lower()


def _have_silverc() -> bool:
    from shutil import which

    return which("silverc") is not None


def reproduce_offline(names: list[str]) -> bool:
    all_ok = True
    print("=" * 74)
    print("OFFLINE covenant-side settlement reproduction (Python SDK silverscript)")
    print("No network · no wallet · no key. Compile + sig layers only.")
    print("=" * 74)

    for name in names:
        sil = COV / f"{name}.sil"
        ctor = COV / f"{name}_ctor.json"
        if not sil.is_file():
            print(f"\n[{name}] SKIP — {sil} not found")
            continue

        ctor_args = kii_onramp.native_ctor_from_silverc_json(ctor)
        comp = kii_onramp.compile_covenant(sil.read_text(), ctor_args)

        print(f"\n── {name} ─────────────────────────────────────────────")
        print(f"  compiler_version : {comp.compiler_version}")
        print(f"  locking script   : {len(comp.script)} bytes")
        print(f"  script_hex[:64]  : {comp.script.hex()[:64]}…")
        print(f"  p2sh address     : {comp.p2sh_address}")
        print(f"  abi entrypoints  : {[e.name for e in comp.abi]}")

        # Build a covenant spend sig script (delegate path — no key needed).
        fn, is_leader = SPEND.get(name, (None, False))
        if fn:
            try:
                sig = kii_onramp.build_sig(comp, fn, None, is_leader=is_leader)
                print(
                    f"  spend sig script : fn={fn!r} delegate -> {len(sig)} bytes "
                    f"({sig.hex()})"
                )
            except ValueError as e:
                print(f"  spend sig script : (delegate build skipped: {e})")

        # The load-bearing assertion: SDK bytes == silverc bytes.
        ref = _silverc_reference_hex(name, sil, ctor)
        sdk = comp.script.hex().lower()
        identical = sdk == ref
        print(
            f"  BYTE-IDENTICAL   : silverc={len(ref) // 2}B  sdk={len(sdk) // 2}B  "
            f"=> {'IDENTICAL ✓' if identical else 'DIVERGENT ✗'}"
        )
        if not identical:
            all_ok = False
            n = min(len(sdk), len(ref))
            d = next((i for i in range(n) if sdk[i] != ref[i]), n)
            print(
                f"    first diff at byte {d // 2}: "
                f"silverc={ref[max(0, d - 4):d + 12]} sdk={sdk[max(0, d - 4):d + 12]}"
            )
        assert identical, f"{name}: SDK script bytes are NOT byte-identical to silverc!"

    print("\n" + "=" * 74)
    print(f"RESULT: {'ALL byte-identical to silverc ✓' if all_ok else 'DIVERGENCE FOUND ✗'}")
    print("Live broadcast: DEFERRED (transient TN10 rate-limit) — NOT run, no txid.")
    print("=" * 74)
    return all_ok


def broadcast_gated(names: list[str]) -> int:
    """Gated, testnet-only broadcast scaffold. DEFERRED — intentionally does NOT
    submit. This documents exactly where the live path would attach, while
    keeping the committed example from ever touching kaspanet."""
    print("=" * 74)
    print(">>> --broadcast requested <<<")
    if os.environ.get("KCP_TESTNET") != "1":
        print("REFUSED: set KCP_TESTNET=1 to acknowledge testnet-only operation.")
        return 2
    # Hard guard: never mainnet.
    network = os.environ.get("KCP_NETWORK", "testnet-10").lower()
    if "main" in network:
        print("REFUSED: this on-ramp is TESTNET-ONLY; mainnet is never used.")
        return 2

    print(f"network              : {network} (testnet only)")
    print("key source           : $KCP_WALLET_KEY_FILE (read at spend time; never printed)")
    key_file = os.environ.get("KCP_WALLET_KEY_FILE")
    print(f"key file configured  : {'yes' if key_file else 'NO (would be required)'}")
    print()
    print("STATUS: DEFERRED — live submit is blocked by a transient server-side")
    print("rate-limit on our TN10 settlement endpoint. The compile + sig layers")
    print("above are real and verifiable now; the on-chain submit is NOT-YET-RUN.")
    print("We do NOT fabricate a txid.")
    print()
    print("When the rate-limit clears, the live path would, via the SDK RpcClient:")
    print("  1. connect to the TN10 node (KCP_NODE_URL), load the wallet key file,")
    print("  2. lock value to comp.p2sh_address (a normal output to the P2SH spk),")
    print("  3. build the spend tx, compute the input sighash,")
    print("  4. produce the Schnorr `sig` for the LEADER entrypoint,")
    print("     sig_script = build_sig(comp, fn, [schnorr_sig, ...], is_leader=True),")
    print("  5. submit_transaction(tx) and record the returned txid as evidence.")
    print()
    print("No transaction was built or submitted. Exiting without broadcasting.")
    print("=" * 74)
    return 0


def main(argv: list[str]) -> int:
    args = [a for a in argv if not a.startswith("-")]
    flags = {a for a in argv if a.startswith("-")}
    names = args or COVENANTS

    if "--broadcast" in flags:
        # Still prove the offline layers first, then hit the gated (deferred) path.
        ok = reproduce_offline(names)
        rc = broadcast_gated(names)
        return rc if rc else (0 if ok else 1)

    return 0 if reproduce_offline(names) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
