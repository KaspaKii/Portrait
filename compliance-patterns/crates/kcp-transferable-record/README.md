# kcp-transferable-record

> **v0 — unaudited — testnet first.**

A Kaspa compliance pattern: a registry record whose ownership can be transferred
between parties through a verifiable on-chain UTXO chain.

Part of the `kaspa-compliance-patterns` workspace, targeting the
[Toccata](https://github.com/kaspanet/rusty-kaspa) hardfork (~30 Jun 2026,
DAA 474,165,565).

---

## What is a transferable record?

A transferable record is a **dedicated UTXO chain**. The record is created by
locking a small amount of KAS to the initial controller's address. Each transfer
spends the current record UTXO (proving control with the holder's Schnorr key)
and re-locks the value to the next controller's address, embedding a structured
payload in the transaction.

The result is a publicly auditable chain of custody for an off-chain asset or
identity claim, anchored to real on-chain transactions.

---

## v0 enforcement: honest accounting

### What IS enforced on-chain in v0

| Property | Mechanism |
|---|---|
| Only the current controller can transfer | Pay-to-address locking script; requires the controller's Schnorr signature to spend |
| Each transfer creates a new on-chain record | Real Kaspa transaction, visible on any block explorer |

### What is carried in the payload (NOT enforced by consensus in v0)

| Property | Where it lives | Notes |
|---|---|---|
| Sequence numbers (TR-1) | Payload `seq` field | Verified off-chain by `lineage::validate_chain` |
| Record identity (TR-2) | Payload `record_id` field | Verified off-chain by `lineage::validate_chain` |
| Event commitments (TR-3) | Payload `commitment` field | Verified off-chain by `lineage::validate_chain` |

**Consensus does not inspect the payload or reject malformed successors in v0.**
A malicious party who obtains the current controller key can submit a transfer
with an arbitrary payload; the lineage library will detect the violation
off-chain, but consensus will not prevent the spend.

### v1 on-chain enforcement (introspection)

A covenant script (`covenant/transferable-record.sil`) enforces TR-1, TR-2, and
ownership (TR-3) at the **consensus layer** using the `validateOutputState` /
KIP-20 covenant introspection opcodes available on Toccata.

**State shape:** `record_id` (byte[32]), `seq` (int), `controllerPk` (byte[32]).

**Invariants enforced on-chain:**

| Invariant | Covenant check |
|---|---|
| TR-1 — seq monotone | `require(newStates[0].seq == prevStates[0].seq + 1)` |
| TR-2 — record identity | `require(newStates[0].record_id == prevStates[0].record_id)` |
| TR-3 — ownership | `require(checkSig(s, prevStates[0].controllerPk))` |
| Structural — single live record | `from=1, to=1` covenant binding (fan-out precluded) |

**Provenance:**

- Source: `covenant/transferable-record.sil`
- Compiled script: `covenant/transferable-record.script.hex` (548 bytes)
- Compiled with `silverc` (silverscript@2c46231), validated against rusty-kaspa v2.0.0
- Engine-proven via `TxScriptEngine` with `covenants_enabled: true`:
  8/8 tests pass (3 ACCEPT, 5 REJECT — archived outside this published repo, available on request)
- Library stays on `tag=v2.0.0`; compiled script is embedded as data only
- `silverscript-lang` is NOT a dependency of this crate

This was engine-level proof under a controlled local harness; the covenant was
then deployed **live on testnet-10** `[KCP-TR-003]` (v0, unaudited, synthetic
data). The covenant is covenant-id-bound.

---

## Payload format

The on-chain payload is exactly **78 bytes**:

| Offset | Len | Field | Value |
|---|---|---|---|
| 0 | 5 | magic | `b"KCPTR"` |
| 5 | 1 | version | `0x01` |
| 6 | 32 | record_id | SHA-256 of the canonical genesis body |
| 38 | 8 | seq | transfer sequence number, u64 little-endian |
| 46 | 32 | commitment | SHA-256 of the canonical event body |

The genesis creation transaction uses `seq = 0`. The first transfer uses
`seq = 1`; each subsequent transfer increments by exactly 1.

---

## Usage

### Pure (no node required)

```rust
use kcp_transferable_record::{
    record::{record_id, commitment},
    payload::Payload,
    lineage::{TransferEvent, validate_chain},
};
use serde_json::json;

// 1. Establish a record identity from the genesis body.
let genesis_body = json!({"name": "my-record", "issuer": "ExampleCorp"});
let rid = record_id(&genesis_body).unwrap();

// 2. Encode the genesis payload (seq = 0 for creation).
let genesis_commitment = commitment(&genesis_body).unwrap();
let payload_bytes = Payload { record_id: rid, seq: 0, commitment: genesis_commitment }.encode();

// 3. After a transfer (off-chain lineage check).
let transfer_body = json!({"action": "transfer", "to": "kaspatest:qr..."});
let transfer_commitment = commitment(&transfer_body).unwrap();
let events = vec![TransferEvent {
    seq: 1,
    record_id: rid,
    // Carried, NOT verified — validate_chain never reads this field.
    controller_xonly: [0x02; 32], // next controller's x-only pubkey
    commitment: transfer_commitment,
}];
validate_chain(&events).unwrap();
```

### With node (feature `wrpc`)

```rust
use kcp_common::{wallet::{Wallet, Prefix}, wrpc::{NodeClient, NodeConfig}};
use kcp_transferable_record::tx::{create_record_tx, transfer_record_tx};
// ... see examples/transferable_record_testnet_evidence.rs for the full flow
```

---

## Testnet evidence

Recorded 2026-06-11 on **testnet-10** (local kaspad v2.0.0, synced, DAA ~488,321,891):

- record_id `9eb393fbc263f9574f374fafaf4ac79362e15c01b2c4014a18a8fcdceb7c65e5`
- create tx `98c7bcb87c1b4f81593e3b07795b651cd4ada9eafab7fff4fcbf741ed9e62289`
- transfer tx `fa4f5521906f2fd7b1136ed001d3657222679744e933372316865d74f9ab7949`
  (controller rotated to a second key; lineage validated off-chain, TR-1/TR-2/TR-3)

Testnet evidence is perishable — testnets reset by design. Record the network
and date with any claim, and refresh by re-running the example.

To reproduce or refresh: fund a testnet wallet for the target network, then
run against a synced node:

```sh
KCP_NODE_URL=ws://localhost:17210 \
KCP_KEY_FILE=/path/to/wallet.key \
cargo run -p kcp-transferable-record --example transferable_record_testnet_evidence --features wrpc
```

3. Record the printed evidence block alongside your own run (the maintainers
   track it in `docs/EVIDENCE.md`).

---

## Lineage invariants

Validated by [`lineage::validate_chain`]:

- **TR-1** — `seq` starts at 1 for the first transfer and increments by
  exactly 1 for each subsequent event.
- **TR-2** — every event carries the same `record_id` as the first event.
- **TR-3** — every event's `commitment` is non-zero (structural sanity check).

An empty event list is **rejected** (`Error::LineageEmpty`): validating nothing
must not report success.

**Controller keys are not an input to validation.** `controller_xonly` is
carried on each event but never read, and the controller is not in the on-chain
payload either — it is the record UTXO's locking script. `validate_chain`
therefore cannot detect a fabricated chain with invented or discontinuous
controllers; custody continuity comes from the UTXO chain, where consensus
required each previous controller's signature. See the threat model.

Re-affirmation (transferring to the same controller) is **legal**: it updates
the on-chain commitment without changing ownership.

---

## Caveats

- This crate does not mention KCC20, KRC-20, or KTT internally. For the
  relationship between transferable records and other Kii compliance patterns,
  see the workspace `README.md`.
- The fee constant [`kcp_common::tx::CARRIER_FEE_SOMPI`] is fixed and
  conservative for today's testnet; adjust if mempool rules change at Toccata.
- `kaspa-bip32` and `kaspa-consensus-core` are pinned to `tag = "v2.0.0"` of
  `rusty-kaspa`. The API may change before mainnet activation.

---

## Threat model

> **Pre-production, unaudited, testnet-only; testnet evidence is perishable.**
> This section distils what the crate documents. It is **not a security audit**
> and not an assurance that the properties below hold.

**Assets** — the chain of custody: which key controls the record now, and the
ordered history of transfers; whatever registry claim the record stands for.

**Attacker capabilities (assumed)** — on a UTXO chain the **spender is the
adversary**, and here the spender is the **current controller**: they choose the
transfer transaction, the successor's `controllerPk`, the payload (`seq`,
`commitment`), the outputs, and the timing — including transferring to
themselves or never transferring at all. Every payload is public, and the
commitment is **not blinded**.

**What consensus enforces** — on the default path
(`tx::{create_record_tx,transfer_record_tx}`, plain pay-to-address) only that
the current controller's Schnorr signature spends the record UTXO, and that the
UTXO is spent once, so there is a single live successor. The payload is not
read. Under the covenant-id-bound artifact (live `[KCP-TR-003]`) the script VM
enforces TR-1 `seq + 1`, TR-2 identical `record_id`, TR-3
`checkSig(prevStates[0].controllerPk)`, and the `from = 1, to = 1` binding that
precludes fan-out — one live record, not two.

**What this assumes / trusts off-chain** — the binding between `record_id` and
any real-world asset, registry entry, or legal right is a claim this crate
cannot check; it anchors a hash, not a title. `commitment` is
`SHA-256(canonical_json(body))` with **no blinding factor**, so it conceals the
body only while the body is high-entropy — do not treat it as confidential.
`lineage::validate_chain`'s TR-3 is only a non-zero structural check on the
commitment, materially weaker than the covenant rule of the same name (which is
the ownership check) — and the function checks **sequence, record-id and
commitment-non-zero only**. `TransferEvent::controller_xonly` is carried and
never read, so a discloser can hand an auditor a chain with invented or
discontinuous controller keys and it will validate; the only custody anchor is
the UTXO chain itself, read independently. (An empty event list is rejected
rather than vacuously accepted.) Anchoring under the covenant is a manual step;
the shipped transaction builders do not do it.

**Known limits and non-goals** — the in-repo engine proof
(`tests/covenant_engine.rs`) is **script-VM tier only** for input 0: no
transaction mass, no KIP-9 storage mass, no standardness, and no output-value
floor — so a transfer that the script VM accepts can still be refused by the
network on mass if the record UTXO is sized too small.
The live half rests on perishable testnet-10 txids, with the
committed↔deployed link attested by an out-of-repo capture plus an
execution-cost fingerprint — evidence, not a binding. Consensus never checks
that the transferee is *entitled* to receive the record: there is no allowlist,
no KYC hook, no revocation, and no way to claw a record back from a controller
who holds it. A compromised controller key is a transferred record.
Re-affirmation to the same controller is legal by design, so `seq` counts
events, not owners. secp256k1 Schnorr throughout: not post-quantum. Unaudited;
not for mainnet value.
