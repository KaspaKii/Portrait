# kcp-ktt-token

`kcp-ktt-token` provides a KCC20-shape regulated-token profile: a 4-field
state machine (`owner_identifier`, `identifier_type`, `amount`, `is_minter`)
that enforces supply conservation, minter-escalation guard, and owner
authorisation. It is the regulated-token equivalent for Kaspa compliance.

## Constructing a token operation

```rust
use kcp_ktt_token::{
    state::{IdentifierType, KttState},
    token::{mint, AuthContext},
};

let minter = KttState {
    identifier_type: IdentifierType::Pubkey,
    owner_identifier: minter_key,
    amount: 0,
    is_minter: true,
};
let holder = KttState {
    identifier_type: IdentifierType::Pubkey,
    owner_identifier: holder_key,
    amount: 1_000_000,
    is_minter: false,
};
let persisted_minter = KttState { amount: 0, ..minter };

let auth = AuthContext { authorised_owners: vec![minter_key] };
mint(&minter, &holder, &persisted_minter, &auth, 0)?; // KTT-1..KTT-4 enforced
```

## A note on the four invariants (KTT-1..KTT-4)

- **KTT-1 supply conservation** — for `transfer` and `burn`: outputs must balance inputs. Not enforced on `mint`.
- **KTT-2 minter escalation guard** — a non-minter input cannot produce a minter output. **It is critical** that callers never set `is_minter = true` on an output unless the corresponding input was also a minter.
- **KTT-3 owner authorisation** — `authorised_owners` must include the `owner_identifier` of every spending input.
- **KTT-4 identifier type validity** — `identifier_type` must be one of `Pubkey`, `ScriptHash`, or `CovenantId`.

## A note on KCC20 shape alignment

`kcp-ktt-token` targets the same 4-field layout as KCC20 covenants. This is **engine-proven** `[KCP-KTT-002]` and **live on testnet-10** `[KCP-KTT-003]`.

## Extensions

- **Compliance workflow** — mint a compliance token after verifying a credential. See `examples/compliance-workflow`.

→ API reference: [`KttState`], [`IdentifierType`], [`AuthContext`], [`mint`], [`transfer`], [`burn`]
