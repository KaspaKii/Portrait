"""Pure-Python Kaspa P2SH address derivation.

Mirrors the pinned engine (rusty-kaspa v2.0.0 = 90dbf07) exactly:

  * redeem-script hash = BLAKE2b-256 of the redeem (locking) script bytes,
    with the library default parameters (no key, no salt, no personalization).
    This matches `blake2b_simd::Params::new().hash_length(32)` used by
    `kaspa_txscript::pay_to_script_hash_script` (see
    kaspa-txscript .../src/standard.rs) and Python's stdlib
    `hashlib.blake2b(..., digest_size=32)`.

  * the address is the Kaspa cashaddr-style bech32 of
    `version_byte(=8 for ScriptHash) || 32-byte hash`, prefixed `kaspatest:`
    on testnet. Polymod constants + CHARSET copied verbatim from
    kaspa-addresses .../src/bech32.rs.

OFFLINE ONLY. No network, no key material. This file only computes an address
string from script bytes.
"""

from __future__ import annotations

import hashlib

# bech32 charset + polymod — verbatim from kaspa-addresses src/bech32.rs
_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
_SCRIPT_HASH_VERSION = 8  # kaspa-addresses Version::ScriptHash = 8


def _polymod(values) -> int:
    c = 1
    for d in values:
        c0 = c >> 35
        c = ((c & 0x07FFFFFFFF) << 5) ^ d
        if c0 & 0x01:
            c ^= 0x98F2BC8E61
        if c0 & 0x02:
            c ^= 0x79B76D99E2
        if c0 & 0x04:
            c ^= 0xF33E5FB3C4
        if c0 & 0x08:
            c ^= 0xAE2EABE2A8
        if c0 & 0x10:
            c ^= 0x1E4F43E470
    return c ^ 1


def _conv8to5(data: bytes) -> list:
    five = []
    buff = 0
    bits = 0
    for b in data:
        buff = (buff << 8) | b
        bits += 8
        while bits >= 5:
            bits -= 5
            five.append((buff >> bits) & 0x1F)
            buff &= (1 << bits) - 1
    if bits > 0:
        five.append((buff << (5 - bits)) & 0x1F)
    return five


def redeem_script_hash(redeem_script: bytes) -> bytes:
    """BLAKE2b-256 of the redeem (locking) script — the value the P2SH lock
    commits to. Identical to the engine's `pay_to_script_hash_script` hash."""
    return hashlib.blake2b(redeem_script, digest_size=32).digest()


def p2sh_address(redeem_script: bytes, prefix: str = "kaspatest") -> str:
    """Derive the Kaspa P2SH address that locks value to `redeem_script`.

    `prefix` is `kaspatest` (testnet, the default) or `kaspa` (mainnet). This
    on-ramp is testnet-only; mainnet is never used by the example drivers.
    """
    h = redeem_script_hash(redeem_script)
    payload5 = _conv8to5(bytes([_SCRIPT_HASH_VERSION]) + h)
    prefix5 = [ord(ch) & 0x1F for ch in prefix]
    chk = _polymod(prefix5 + [0] + payload5 + [0] * 8)
    chk5 = _conv8to5(chk.to_bytes(8, "big")[3:])
    body = "".join(_CHARSET[c] for c in payload5 + chk5)
    return f"{prefix}:{body}"
