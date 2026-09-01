# kcp-ktt-token

> **v0 — unaudited — testnet first.**

A Kaspa compliance pattern: KCC20-shape-aligned regulated-token profile (KTT).
Models the 4-field KCC20 covenant-token state, enforces supply-conservation and
minter-integrity invariants off-chain, and anchors operation evidence on testnet
in carrier transactions.

Part of the `kaspa-compliance-patterns` workspace, targeting the
[Toccata](https://github.com/kaspanet/rusty-kaspa) hardfork (~30 Jun 2026,
DAA 474,165,565).

---

## What KTT is

KTT (**Kaspa Trust Token**) is a regulated-token profile whose on-chain state
mirrors the 4-field KCC20 covenant-token shape. Every token balance is
represented by one `KttState`:

| Field | Type | Description |
|---|---|---|
| `identifier_type` | byte (0x00/0x01/0x02) | How to interpret `owner_identifier` |
| `owner_identifier` | `[u8; 32]` | Pubkey, script-hash, or covenant-id of the owner |
| `amount` | `u64` (little-endian) | Token balance in the smallest representable unit |
| `is_minter` | bool (0x00/0x01) | Whether this state holder controls issuance |

### Identifier types

| Byte | Meaning |
|---|---|
| `0x00` | 32-byte x-only public key (Schnorr / BIP-340) |
| `0x01` | 32-byte P2SH script hash |
| `0x02` | 32-byte KIP-20 covenant identifier |

---

## KCC20 / KRC-20 / KTT disambiguation

**These are three distinct things.** See the workspace README for the full
explanation. In brief:

- **KCC20** — a reference covenant-token contract in the rusty-kaspa codebase
  that demonstrates the `validateOutputState` / `validateOutputStateWithTemplate`
  enforcement primitives. It is a proof-of-concept, not a production standard.
- **KRC-20** — an off-chain indexer protocol for Kaspa (analogous to BRC-20 on
  Bitcoin). Operates via `OP_RETURN` inscriptions; entirely separate from KCC20.
- **KTT** — the Kii regulated-token profile. Shape-aligned with KCC20 (targets
  the same 4-field state and the same on-chain enforcement primitives). Not a
  port of KRC-20. Not conformant with any published standard; v0 is
  KCC20-*shape-aligned*, meaning the state layout and transition rules follow
  the KCC20 contract logic.

---

## v1 on-chain enforcement (introspection)

> This crate ships a SilverScript covenant (`covenant/ktt.sil`, compiled to
> `covenant/ktt.script.hex`) whose state-transition correctness is **proven
> against the real rusty-kaspa script engine** — mirroring how upstream verifies
> KCC20 itself. The compiled script is embedded here as data; the engine
> **proof** lives as a reproducible research artifact archived outside this repo
> (see below), because running it in-process pulls a path dependency on the
> SilverScript clone and an off-`v2.0.0` engine pin (kept out of this
> publishable workspace).

### Covenant artifact

| Artifact | Path |
|---|---|
| SilverScript source | `covenant/ktt.sil` |
| Compiled script (hex) | `covenant/ktt.script.hex` |
| Full compiler output | `covenant/ktt.compiled.json` |
| Provenance record | `covenant/README.md` |

Compiled script size: **1540 bytes** (identical to KCC20).
State layout: start=1, len=46 (4-field, same wire layout as KCC20).
SilverScript compiler: `silverscript@2c46231` (silverscript-lang v0.1.0).
Engine proof rev: rusty-kaspa `42b734f` (the rev silverscript-lang pins) —
**this library's own workspace stays on `tag=v2.0.0`** and embeds only the
compiled script bytes; it does not depend on silverscript-lang.

### Engine-proof results

The proof harness (an archived research artifact, kept outside this published
repo) executes KTT covenant
transfers through `TxScriptEngine::from_transaction_input` with
`covenants_enabled: true` and `CovenantsContext::from_tx` — the same pattern as
`kcc20_tests.rs`. Six tests, all passing:

| Test | Verdict | What it proves |
|---|---|---|
| `ktt_engine_accepts_valid_handoff` | ACCEPT | Valid single-input→single-output transfer passes engine |
| `ktt_engine_rejects_transfer_when_amounts_do_not_match` | REJECT | Supply-conservation enforced by covenant |
| `ktt_engine_rejects_minter_escalation_from_non_minter` | REJECT | Minter-integrity enforced by covenant |
| `ktt_engine_rejects_wrong_signature` | REJECT | Owner-authorisation enforced by covenant |
| `ktt_engine_accepts_minter_changing_supply` | ACCEPT | Minter can change supply |
| `ktt_engine_accepts_split_then_merge` | ACCEPT | Multi-input merge passes with correct leader/delegate sigscripts |

All 6 archived-harness tests pass against the real engine. The publishable
workspace (this library, on `tag=v2.0.0`) carries the compiled covenant as data
and remains green independent of the harness.

The covenant was then deployed **live on testnet-10** `[KCP-KTT-003]` (v0,
unaudited, synthetic data) — a covenant-id-bound genesis + append validated by
the live consensus covenant engine.

### 5th compliance-tier field — deferred

Adding a `complianceTier` field to the covenant state was evaluated and deferred.
Reason: the field changes the compiled script hash, which changes the covenant-id
and breaks the KCC20Minter composition pattern. It is incompatible with the
existing state layout. See `covenant/ktt.sil` lines 13–20 for the rationale
comment in the source.

### What is enforced vs not enforced

| Claim | Mechanism |
|---|---|
| KCC20-shape state modelled | `KttState` with the 4-field wire layout |
| Supply conservation (KTT-1) enforced | **On-chain by covenant** (engine-proven) |
| Minter-integrity (KTT-2) enforced | **On-chain by covenant** (engine-proven) |
| Owner authorisation (KTT-3) enforced | **On-chain by covenant** (engine-proven) |
| Evidence anchored on-chain | Carrier transactions with 71-byte `KCPKT` payload |
| Compliance-attestation hooks modelled | `transfer_rules` bitmask (KYC_REQUIRED, JURISDICTION, etc.) |

Not enforced by the covenant in this version:
- The `transfer_rules` bitmask — modelled off-chain only; no compliance oracle.
- This crate is pre-production and unaudited. The covenant has not been deployed
  to mainnet or audited for correctness beyond the harness tests.

## v0 enforcement (legacy note)

> Kept for historical reference. v0 used off-chain-only enforcement.

### What WAS done in v0

| Claim | Mechanism |
|---|---|
| KCC20-shape state modelled | `KttState` with the 4-field wire layout |
| Supply conservation (KTT-1) enforced | Off-chain `token::transfer` / `token::burn` |
| Minter-integrity (KTT-2) enforced | Off-chain — non-minter inputs cannot produce minter outputs |
| Owner authorisation (KTT-3) modelled | Off-chain `AuthContext` — caller asserts which owners signed |
| Evidence anchored on-chain | Carrier transactions with 71-byte `KCPKT` payload |
| Compliance-attestation hooks modelled | `transfer_rules` bitmask (KYC_REQUIRED, JURISDICTION, etc.) |

### What was NOT done in v0

- State transitions were validated **off-chain**. The UTXO itself was
  key-controlled (pay-to-address). Any party holding the wallet key could spend
  the UTXO without satisfying the token invariants.
- The `transfer_rules` bitmask was modelled but **not enforced**.

---

## Compliance-attestation hooks

The `transfer_rules` module provides a `u32` bitmask with the following flags
(sourced from the KTT donor codebase's `TransferRule` enum):

| Flag | Value | Meaning |
|---|---|---|
| `KYC_REQUIRED` | `0x0001` | KYC check required before transfer |
| `JURISDICTION` | `0x0002` | Jurisdiction restriction |
| `HOLDING_PERIOD` | `0x0004` | Minimum holding period |
| `TRANSFER_LIMIT` | `0x0008` | Per-transfer amount cap |
| `FREEZE_CAPABLE` | `0x0010` | Authority may freeze balances |
| `CLAWBACK` | `0x0020` | Authority may clawback tokens |
| `WHITELIST_ONLY` | `0x0040` | Only whitelisted addresses may receive |
| `ACCREDITED_ONLY` | `0x0080` | Only accredited investors may hold |

These flags are **modelled** in v0. Passing a `transfer_rules` bitmask to the
token operation functions records the intended ruleset; enforcement requires a
future on-chain compliance covenant.

---

## Usage

### Pure state operations (no node required)

```rust
use kcp_ktt_token::{
    state::{IdentifierType, KttState},
    token::{transfer, AuthContext},
};

let issuer_xonly = [0x01u8; 32];
let recipient_xonly = [0x02u8; 32];

let input = KttState {
    identifier_type: IdentifierType::Pubkey,
    owner_identifier: issuer_xonly,
    amount: 1_000_000,
    is_minter: true,
};

let output_issuer = KttState {
    identifier_type: IdentifierType::Pubkey,
    owner_identifier: issuer_xonly,
    amount: 600_000,
    is_minter: true,
};
let output_recipient = KttState {
    identifier_type: IdentifierType::Pubkey,
    owner_identifier: recipient_xonly,
    amount: 400_000,
    is_minter: false,
};

let auth = AuthContext { authorised_owners: vec![issuer_xonly] };
transfer(&[input], &[output_issuer, output_recipient], &auth, 0).unwrap();
```

### State encode / decode

```rust
use kcp_ktt_token::state::{IdentifierType, KttState};

let state = KttState {
    identifier_type: IdentifierType::Pubkey,
    owner_identifier: [0xabu8; 32],
    amount: 500_000,
    is_minter: false,
};
let bytes = state.encode();           // 42 bytes
let decoded = KttState::decode(&bytes).unwrap();
assert_eq!(decoded, state);
```

### Payload encode / decode

```rust
use kcp_ktt_token::payload::{OpClass, Payload};

let p = Payload {
    token_id: [0x11u8; 32],
    op_class: OpClass::Transfer,
    state_commitment: [0x22u8; 32],
};
let bytes = p.encode();               // 71 bytes
let decoded = Payload::decode(&bytes).unwrap();
assert_eq!(decoded.op_class, OpClass::Transfer);
```

### Anchoring on-chain (feature `wrpc`)

```rust
use kcp_ktt_token::tx::{anchor_token_op_tx, DEFAULT_TOKEN_OP_VALUE_SOMPI};
// See examples/ktt_testnet_evidence.rs for the full three-step flow.
```

---

## Testnet evidence

Recorded 2026-06-11 on **testnet-10** (local kaspad v2.0.0, synced, DAA ~488,373,798).
Full KCC20-shape lifecycle (issue → transfer → burn), each op carrier-anchored,
the transition chain validated off-chain (KTT-1/2/3 + burn invariants):

- token_id `86dc55379205924b6e594165023cb5a90bf3bc0789360e5b5a7ef095ca3a5d43`
- issue tx `258a857c9dc43f4c2327ef4610d15339e74e10ed5d4632d84e57a3a2f767b7a0`
- transfer tx `06d9b05f7af1793cd1ce5a151cb3f1b16e955aadb5119f5e05c3a036102b484a`
- burn tx `dfa677bd03a1e336441ac5824bbf3b46fe332c2ddb19a381588c3e8617fa4605`

The on-chain binding target — KCC20 `validateOutputStateWithTemplate` — is
engine-enforced and verified real (pattern-library FACTS SS-026); authoring the
Kii covenant against it is the documented next step.

To reproduce or refresh: fund a testnet wallet, then run:

```sh
KCP_NODE_URL=ws://localhost:17210 \
KCP_KEY_FILE=/path/to/wallet.key \
cargo run -p kcp-ktt-token --example ktt_testnet_evidence --features wrpc
```

The example prints a FACTS-ready block with an honest note:
`"v0 — unaudited — KCC20-shape state transitions validated off-chain; carrier-anchored on testnet"`.

Testnet evidence is perishable — testnets reset by design. Record the network
and date with any claim, and refresh by re-running the example.

---

## Caveats

- This crate is KCC20-*shape-aligned*, not KCC20-conformant. The upstream KCC20
  contracts are examples, not a production token standard.
- `kcp_common::tx::CARRIER_FEE_SOMPI` (1,000,000 sompi) is the v2.0.0
  relay-fee floor. Revisit if mempool rules change at Toccata mainnet.
- The library workspace pins `tag = "v2.0.0"` of `rusty-kaspa` (see above).
  The external covenant-engine proof harness (archived, not a library dependency)
  used silverscript-lang rev `42b734f` (version 1.1.1-toc.1) as required by
  silverscript-lang. The API may differ from v2.0.0.
- The `transfer_rules` bitmask is modelled, not enforced. Do not rely on it
  for compliance in production.

---

## Threat model

> **Pre-production, unaudited, testnet-only; testnet evidence is perishable.**
> This section distils what the crate documents. It is **not a security audit**
> and not an assurance that the properties below hold.

**Assets** — token supply integrity (no unauthorised issuance), holder
balances, minter authority, and whatever compliance meaning an issuer attaches
to the token.

**Attacker capabilities (assumed)** — on a UTXO chain the **spender is the
adversary**: a holder constructing the spend chooses up to 2 covenant inputs and
up to 2 covenant outputs, every field of each new state (`ownerIdentifier`,
`identifierType`, `amount`, `isMinter`), the signatures, and the transaction's
non-covenant inputs and outputs. That adversary may be a holder, the minter, or
a third party who obtained a holder key.

**What consensus enforces** — under the covenant-id-bound artifact (live
`[KCP-KTT-003]`, engine-proven in `tests/covenant_engine.rs`): **KTT-1** supply
conservation for non-minter spends, as a *sum over the covenant outputs*
(1000 → 400 + 600 is accepted; 400 + 601 is rejected); **KTT-2** no minter
escalation from a non-minter input; **KTT-3** owner authorisation dispatched on
identifier type — `checkSig` for a pubkey, a matching `scriptPubKey` for a P2SH
hash, `OpInputCovenantId` for a covenant id. Arity is bounded at
`maxCovIns = maxCovOuts = 2` (read off the committed artifact): **fan-out and
merge are permitted**, capped at two. It is supply conservation, not arity, that
stops a holder minting.

**What this assumes / trusts off-chain** — everything "regulated" about the
profile. The `transfer_rules` bitmask (`KYC_REQUIRED`, `WHITELIST_ONLY`,
`HOLDING_PERIOD`, `FREEZE_CAPABLE`, `CLAWBACK`, …) is **modelled, not
enforced**: no compliance oracle, no freeze, no clawback, no jurisdiction check
exists in the covenant or in this crate. The Rust `token::transfer` path models
the same invariants with an `AuthContext` whose `authorised_owners` the *caller
asserts* — it is a model, not the enforcement. The v0 carrier-anchored evidence
path is key-controlled pay-to-address, and its payload commits to a state
without governing it. The minter is trusted absolutely: a minter spend may
change supply arbitrarily, by design.

**Known limits and non-goals** — the name promises more than the covenant
delivers. It enforces supply conservation, minter integrity and owner
authorisation, and *nothing else* the word "regulated" implies; it is
KCC20-*shape-aligned*, not conformant to any standard (upstream KCC20 is an
example set). Evidence boundary: the in-repo proof is **script-VM tier only**
for input 0 — no transaction mass, no KIP-9 storage mass, no standardness, no
output-value floor — and the **2-covenant-input merge shape has no in-repo
coverage**, so the sibling-signature parse surface is untested here.
**Those two gaps meet in the fan-out the covenant advertises:** a 1→2 split is
precisely the KIP-9 storage-mass worst case (mass grows as
`≈ 10¹² · outputs² / amount`), so splitting into small-sompi outputs produces a
transaction the script VM accepts and the network rejects on mass. Nothing in
this crate or the covenant places a floor on an output's value — sizing splits
so they are relayable is entirely the caller's job. The live
half rests on perishable testnet-10 txids, with the committed↔deployed link
attested by an out-of-repo capture plus an execution-cost fingerprint. The
`covenant/*_ctor.json` files are **not** the constructor inputs that produced
the artifact; the real ones are `[FACT-NEEDED]`. No pause, no upgrade path, no
recovery of a lost key. secp256k1 Schnorr; not post-quantum. Unaudited; not for
mainnet value.
