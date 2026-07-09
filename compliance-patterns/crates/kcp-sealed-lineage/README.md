# kcp-sealed-lineage

> **v0 — unaudited — testnet first.**

A Kaspa compliance pattern: an append-only sealed evidence lineage anchored
to a dedicated UTXO chain. Each step in the lineage commits to an off-chain
record via a blinded SHA-256 seal.

Part of the `kaspa-compliance-patterns` workspace, targeting the
[Toccata](https://github.com/kaspanet/rusty-kaspa) hardfork (~30 Jun 2026,
DAA 474,165,565).

---

## What is a sealed evidence lineage?

A sealed evidence lineage is a **dedicated UTXO chain**. The publisher creates
a genesis event by locking a small amount of KAS to their own address with a
`seq = 0` payload. Each subsequent evidence step spends the lineage UTXO and
re-locks the value (minus fee) back to the same publisher address, embedding a
structured payload that carries a cryptographic commitment to an off-chain
record body.

The result is a publicly auditable, append-only evidence log anchored to real
on-chain transactions. An observer can verify the chain of events by reading
the transaction payloads; the off-chain record bodies remain private unless
explicitly disclosed, because each commitment is blinded.

---

## v0 enforcement: honest accounting

### What IS enforced on-chain in v0

| Property | Mechanism |
|---|---|
| Only the publisher can append | Pay-to-address locking script; requires the publisher's Schnorr signature to spend |
| Each event creates a new on-chain record | Real Kaspa transaction, visible on any block explorer |

### What is carried in the payload (NOT enforced by consensus in v0)

| Property | Where it lives | Notes |
|---|---|---|
| Sequence numbers (L-1) | Payload `seq` field | Verified off-chain by `invariants::validate_chain` |
| Lineage identity (L-2) | Payload `lineage_id` field | Verified off-chain by `invariants::validate_chain` |
| Event-class rules (L-3) | Payload `event_class` field | Verified off-chain by `invariants::validate_chain` |
| Temporal envelope (L-4) | Payload `t_bucket` field | Verified off-chain by `invariants::validate_chain` |

**Consensus does not inspect the payload or reject malformed successors in v0.**
A party who obtains the publisher key can submit an append with an arbitrary
payload; the invariant library will detect the violation off-chain, but
consensus will not prevent the spend.

### Next step (not in v0)

The documented next step is expressing the invariants as a **covenant
declaration** on the upstream covenant-declaration system, so that consensus
rejects bad successors. This requires `validateOutputState` / KIP-20 covenant
introspection opcodes available on Toccata. Until that work is done, L-invariant
enforcement is application-layer only.

### Value-carry note

The donor lineage system also enforces a fifth invariant (V-5): the successor
transaction must carry the full lineage UTXO value forward (minus fee). In v0
this is ensured by the single-output shape of `append_lineage_tx` — consensus
does not enforce it directly.

---

## Payload format

The on-chain payload is exactly **87 bytes**:

| Offset | Len | Field | Value |
|---|---|---|---|
| 0 | 5 | magic | `b"KCPSL"` |
| 5 | 1 | version | `0x01` |
| 6 | 32 | lineage_id | SHA-256 of the canonical genesis identity body |
| 38 | 8 | seq | event sequence number, u64 little-endian |
| 46 | 1 | event_class | `0x00` = Genesis, `0x01` = Append, `0x02` = Close |
| 47 | 8 | t_bucket | publisher-supplied seconds since Unix epoch, u64 LE |
| 55 | 32 | commitment | blinded SHA-256 commitment to the off-chain record |

The genesis transaction uses `seq = 0` and `event_class = 0x00`. The first
append uses `seq = 1`; each subsequent append increments by exactly 1. A
Close event (`0x02`) is terminal: no event may follow it.

---

## Commitment construction (SHA-256 vs. Poseidon)

The commitment is a **blinded SHA-256 seal**:

```text
commitment = SHA-256(canonical_json(record_body) || blind)
```

1. `record_body` is serialised to canonical JSON (sorted object keys).
2. The 32-byte `blind` is appended directly.
3. SHA-256 is computed over the concatenation.

The `blind` must be kept secret by the publisher and stored off-chain alongside
the record body. Without the blind, an observer cannot verify a hypothesised
body against the on-chain commitment.

**Divergence from donor:** the Kii SCL donor system uses a BN254 Poseidon
commitment (ZK-friendly). v0 of this pattern deliberately uses SHA-256 — it
is simpler, has no elliptic-curve dependencies, and can be reviewed by any
cryptographer without ZK tooling. The Poseidon upgrade path is tied to future
ZK-circuit work scheduled for the KIP-16 era; it is out of scope for v0.

---

## Lineage invariants

Validated by [`invariants::validate_chain`]:

- **L-1** — `seq` starts at `0` (Genesis) and increments by exactly `1`.
- **L-2** — every event carries the same `lineage_id` as the first event.
- **L-3** — Genesis (`0x00`) is only valid at `seq = 0`; Append (`0x01`) is
  valid at any `seq ≥ 1`; Close (`0x02`) is terminal (nothing may follow);
  unknown class values are rejected.
- **L-4** — `t_bucket` is non-decreasing and the step between consecutive
  events does not exceed 90 days (`7 776 000` seconds). This follows the donor
  system's evidence-cadence envelope, which requires publishers to refresh or
  close a lineage at least once per quarter.

---

## Usage

### Pure (no node required)

```rust
use kcp_sealed_lineage::{
    record::{lineage_id, commitment},
    payload::Payload,
    invariants::{validate_chain, GENESIS, APPEND},
};
use serde_json::json;

// 1. Establish a lineage identity from the genesis body.
let genesis_body = json!({"name": "my-evidence-log"});
let lid = lineage_id(&genesis_body).unwrap();

// 2. Compute a blinded commitment (keep the blind off-chain!).
let blind = [0u8; 32]; // use a CSPRNG in production
let genesis_commitment = commitment(&genesis_body, &blind).unwrap();

// 3. Encode the genesis payload.
let now_secs = 1_700_000_000u64;
let genesis_payload = Payload {
    lineage_id: lid,
    seq: 0,
    event_class: GENESIS,
    t_bucket: now_secs,
    commitment: genesis_commitment,
}.encode();

// 4. After an append, validate the chain off-chain.
let append_body = json!({"action": "append", "ref": "doc-001"});
let append_blind = [1u8; 32];
let append_commitment = commitment(&append_body, &append_blind).unwrap();
let chain = vec![
    Payload { lineage_id: lid, seq: 0, event_class: GENESIS,
              t_bucket: now_secs, commitment: genesis_commitment },
    Payload { lineage_id: lid, seq: 1, event_class: APPEND,
              t_bucket: now_secs, commitment: append_commitment },
];
validate_chain(&chain).unwrap();
```

### With node (feature `wrpc`)

```rust
use kcp_common::{wallet::{Wallet, Prefix}, wrpc::{NodeClient, NodeConfig}};
use kcp_sealed_lineage::tx::{create_lineage_tx, append_lineage_tx, DEFAULT_LINEAGE_VALUE_SOMPI};
// ... see examples/testnet_evidence.rs for the full flow
```

---

## Testnet evidence

Recorded 2026-06-11 on **testnet-10** (local kaspad v2.0.0, synced, DAA ~488,321,927):

- lineage_id `df14572db7e61314ddef8c795ade894ebca2b4910f38cfaa95bb86eb3b61f777`
- genesis tx `50257d5d15a080914a70e56ab8445124f58e52996f188cd65f42e2f965e52095`
- append tx `be5c06245eb3cb1a9ab7e92de1d0bc211968e1f62d880e852d74b5e1db35c76a`
  (chain validated off-chain, L-1/L-2/L-3/L-4)

Testnet evidence is perishable — testnets reset by design. Record the network
and date with any claim, and refresh by re-running the example.

To reproduce or refresh: fund a testnet wallet for the target network, then
run against a synced node:

```sh
KCP_NODE_URL=ws://localhost:17210 \
KCP_KEY_FILE=/path/to/wallet.key \
cargo run -p kcp-sealed-lineage --example testnet_evidence --features wrpc
```

3. Record the printed evidence block alongside your own run (the maintainers
   track it in `docs/EVIDENCE.md`).

---

## v1 on-chain enforcement (introspection)

The `covenant/` directory contains the first engine-proven implementation of
L-1..L-4 as a **SilverScript covenant declaration**, targeting Kaspa Toccata
(KIP-20 covenant-declaration opcodes).

### What is in `covenant/`

| File | Description |
|---|---|
| `sealed-lineage.sil` | SilverScript source (810+ bytes compiled) |
| `sealed-lineage.script.hex` | Compiled script (812 bytes, hex) |
| `sealed-lineage.compiled.json` | Full silverc JSON artifact + ABI |
| `README.md` | Compilation provenance and engine-proof summary |

### Invariants enforced by the covenant (on-chain, consensus-layer)

| Invariant | Enforcement |
|---|---|
| L-1 — seq monotone | `require(newStates[0].seq == prevStates[0].seq + 1)` |
| L-2 — lineage identity | `require(newStates[0].lineage_id == prevStates[0].lineage_id)` |
| L-3 — event-class ordering | reject if `prevState.event_class == CLOSE`; reject if `newState.event_class == GENESIS` |
| L-4 — temporal envelope | `require(new.t_bucket >= prev.t_bucket && new.t_bucket <= prev.t_bucket + 7776000)` |
| Ownership | `require(checkSig(s, prevStates[0].publisherPk))` |

### Engine proof

The compiled script was executed against `TxScriptEngine::from_transaction_input`
with `covenants_enabled: true` (real engine, not a stub). 12 tests covering
all accept and reject cases ran against the `silverscript-lang` harness
(silverscript commit `2c46231`, validated against rusty-kaspa v2.0.0).

Full harness, test output, and reproduction instructions are kept as an
archived research artifact outside this published repo.

### Honesty note

This was **engine-level proof** (local harness); the covenant was then deployed
**live on testnet-10** `[KCP-SL-003]` (v0, unaudited, synthetic data). The library
itself stays on `tag=v2.0.0` and embeds the compiled script as **data only**
— no dependency on `silverscript-lang` is added to this crate. The covenant
is **covenant-id-bound** (not P2SH-wrapped), matching the KCC20/KTT
state-continuity model. "Built (pre-production, unaudited)" is an accurate
description of the current status.

---

## Caveats

- This crate does not mention KCC20, KRC-20, or KTT internally.
- The fee constant [`kcp_common::tx::CARRIER_FEE_SOMPI`] is fixed and
  conservative for today's testnet; adjust if mempool rules change at Toccata.
- `kaspa-bip32` and `kaspa-consensus-core` are pinned to `tag = "v2.0.0"` of
  `rusty-kaspa`. The API may change before mainnet activation.
- The SHA-256 commitment construction (not Poseidon) is documented in the
  section above; the Poseidon upgrade is deferred to KIP-16 era ZK work.
