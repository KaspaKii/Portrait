# kcp-transferable-record

`kcp-transferable-record` provides an ownership-transfer chain for unique
records: a genesis controller creates a record, and each transfer reassigns
the controlling key while preserving the `record_id` and incrementing the
sequence. It is the ownership model equivalent for Kaspa compliance.

## Constructing a transfer chain

```rust
use kcp_transferable_record::{
    lineage::{validate_chain, TransferEvent},
    record::{commitment, record_id},
};
use serde_json::json;

let rec_id     = record_id(&json!({ "subject": "entity-a-id" }))?;
let rec_commit = commitment(&json!({ "action": "kyc-transfer" }))?;

let genesis_controller: [u8; 32] = issuer_pubkey;
let events = vec![
    TransferEvent {
        seq: 1,
        record_id: rec_id,
        controller_xonly: new_controller_pubkey,
        commitment: rec_commit,
    },
];
validate_chain(&genesis_controller, &events)?; // verifies TR-1..TR-3
```

## A note on the three invariants (TR-1..TR-3)

- **TR-1 monotone sequence** — `seq` starts at 1 for the first transfer and increments by 1.
- **TR-2 record identity** — every event carries the same `record_id`.
- **TR-3 commitment non-zero** — each `commitment` must be a non-zero 32-byte value.

An empty `events` slice is valid — it means the record has never been transferred.

**Re-affirmation** (transferring to the same controller) is explicitly allowed.
Callers that want to prohibit no-op rotations must enforce that rule above this layer.

## Extensions

- **Compliance workflow** — chain with `kcp-sealed-lineage` to audit all transfers in a tamper-evident log. See `examples/compliance-workflow`.
- **On-chain enforcement** — v1 covenant enforces TR-1..TR-3 at consensus `[KCP-TR-002, KCP-TR-003]`.

→ API reference: [`TransferEvent`], [`record_id`], [`commitment`], [`validate_chain`]
