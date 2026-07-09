# kcp-sealed-lineage

`kcp-sealed-lineage` provides an append-only, tamper-evident evidence chain.
Each event in the lineage carries a blinded commitment to an off-chain record,
sealed with a `lineage_id` that ties all events to a single genesis identity.
It is the audit trail primitive for Kaspa compliance.

## Constructing a lineage

```rust
use kcp_sealed_lineage::{
    invariants::{validate_chain, APPEND, GENESIS},
    payload::Payload,
    record::{commitment, lineage_id},
};
use serde_json::json;

let lid  = lineage_id(&json!({ "subject": "entity-a-id" }))?;
let c0   = commitment(&json!({ "subject": "entity-a-id" }), &blind_a)?;
let c1   = commitment(&json!({ "event": "kyc-approved" }), &blind_b)?;

let chain = vec![
    Payload { lineage_id: lid, seq: 0, event_class: GENESIS, t_bucket: 0, commitment: c0 },
    Payload { lineage_id: lid, seq: 1, event_class: APPEND,  t_bucket: 1, commitment: c1 },
];
validate_chain(&chain)?; // verifies L-1..L-4
```

## A note on the four invariants (L-1..L-4)

`validate_chain` enforces:
- **L-1 monotone sequence** — `seq` starts at 0 and increments by exactly 1.
- **L-2 lineage identity** — every event carries the same `lineage_id`.
- **L-3 event-class rules** — `GENESIS` only at `seq=0`; `APPEND` at any `seq≥1`; after `CLOSE`, nothing may follow.
- **L-4 temporal envelope** — `t_bucket` values must be non-decreasing.

**It is critical** that `t_bucket` values come from a reliable clock source.
A `t_bucket` far in the future can artificially extend the L-4 envelope.

## Extensions

- **On-chain enforcement** — the v1 covenant enforces L-1..L-4 at consensus via `validateOutputState` `[KCP-SL-002, KCP-SL-003]`.
- **Compliance workflow** — chain with `kcp-paired-attestation` by sealing the `attestation_id` as an APPEND commitment. See `examples/compliance-workflow`.

→ API reference: [`Payload`], [`lineage_id`], [`commitment`], [`validate_chain`], [`GENESIS`], [`APPEND`], [`CLOSE`]
