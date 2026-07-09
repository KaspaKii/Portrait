# kcp-paired-attestation

> **v1 (on-chain two-datasig, full) + v0 (off-chain mating) — unaudited — testnet first.**

A Kaspa compliance pattern: a two-party mutual attestation anchored to an
on-chain lineage. Each counterparty independently commits to a shared record
under their own blinding factor. "Mating" proves both committed to the same
record (equality under disclosed blinds), verified off-chain. The attestation
sequence is anchored as a two-step on-chain lineage.

Part of the `kaspa-compliance-patterns` workspace, targeting the
[Toccata](https://github.com/kaspanet/rusty-kaspa) hardfork (~30 Jun 2026,
DAA 474,165,565).

---

## What is paired attestation?

Two counterparties (Party A and Party B) agree on a shared attestation record.
Each party independently generates a blinding factor, commits to the record
under that blind, and publishes the commitment. After both commitments are
established, both parties disclose their blinds to a verifier. The verifier
recomputes both commitments from the shared record and the disclosed blinds;
if they match, both parties are proven to have committed to the same record.
This is the "equality under disclosed blinds" proof.

The commitment sequence is then anchored on-chain as a two-step UTXO chain:

1. Party A's commitment (`PartyACommit`, `seq = 0`) — creates the lineage UTXO.
2. Party B's commitment (`PartyBMate`, `seq = 1`) — spends the lineage UTXO and
   re-locks it, carrying the mate event.

---

## v0 enforcement: honest accounting

> **v0 scope-down — the pre-committed fallback.** The full on-chain version
> (two independent data-signatures enforced inside one covenant entry-point) is
> now **proven viable**: FACTS SS-024-v4 confirms that `OpCheckSigFromStack`
> (`checkDataSig`) correctly binds key+message on kaspad v2.0.0 (CSFS
> positive-control passed on the engine). The remaining work is P2SH
> spend-path plumbing in `kcp-common`, not yet present. That is the
> documented next step; it is not in v0 scope.

### What IS enforced on-chain in v0

| Property | Mechanism |
|---|---|
| Only the wallet holder can append | Pay-to-address locking script; requires the Schnorr signature to spend |
| Each event creates a new on-chain record | Real Kaspa transaction, visible on any block explorer |

### What is verified off-chain (NOT enforced by consensus in v0)

| Property | Where it lives | Notes |
|---|---|---|
| Sequence numbers (PA-1) | Payload `seq` field | Verified off-chain by `invariants::validate_chain` |
| Attestation identity (PA-2) | Payload `attestation_id` field | Verified off-chain by `invariants::validate_chain` |
| Event-class order (PA-3) | Payload `event_class` field | Verified off-chain by `invariants::validate_chain` |
| Mate proof validity (PA-4) | Off-chain `MateProof` | Verified off-chain by `mate::verify_mate` before anchoring |

**Consensus does not inspect the payload or reject malformed successors in v0.**
Mating is verified by this library before the `PartyBMate` transaction is
submitted; an adversary who obtains the wallet key could submit an arbitrary
`PartyBMate` payload, but the invariant library will detect the violation.

### Single-publisher lineage (v0)

In v0, **one wallet anchors both steps**. The two-party datasig binding —
where Party A's signature is enforced at the covenant entry-point and Party B's
signature is a second datasig on the same entrypoint — requires P2SH
spend-path plumbing not yet in `kcp-common`. That is the next step.

---

## Payload format

The on-chain payload is exactly **79 bytes**:

| Offset | Len | Field | Value |
|---|---|---|---|
| 0 | 5 | magic | `b"KCPPA"` |
| 5 | 1 | version | `0x01` |
| 6 | 32 | attestation_id | SHA-256 of the canonical attestation record |
| 38 | 8 | seq | event sequence number, u64 little-endian |
| 46 | 1 | event_class | `0x00` = PartyACommit, `0x01` = PartyBMate, `0x02` = Close |
| 47 | 32 | commitment | blinded SHA-256 commitment of the committing party |

---

## Commitment construction (SHA-256 vs. Poseidon)

The commitment is a **blinded SHA-256 seal**, matching `kcp-sealed-lineage`:

```text
commitment = SHA-256(canonical_json(record) || blind)
```

1. `record` is serialised to canonical JSON (sorted object keys).
2. The 32-byte `blind` is appended directly.
3. SHA-256 is computed over the concatenation.

The blind must be kept off-chain. Without it, an observer cannot verify a
hypothesised record against the on-chain commitment.

**Divergence from donor:** the Kii PLA donor system uses a BN254 Poseidon
commitment (ZK-friendly). v0 deliberately uses SHA-256 — simpler, no
elliptic-curve dependencies, reviewable without ZK tooling. The Poseidon
upgrade is tied to KIP-16-era ZK-circuit work; it is out of scope for v0.

---

## Blind negotiation

Each party independently generates a 32-byte random share. The two shares are
exchanged off-band. The combined blind is their XOR:

```text
blind = share_a XOR share_b
```

This is symmetric and prevents either party from unilaterally choosing a blind
that enables later equivocation. The XOR is computed by `mate::negotiate_blind`.

---

## Lineage invariants

Validated by `invariants::validate_chain`:

- **PA-1** — `seq` starts at `0` (PartyACommit) and increments by exactly `1`.
- **PA-2** — every event carries the same `attestation_id` as the first event.
- **PA-3** — `PartyACommit` (`0x00`) is only valid at `seq = 0`;
  `PartyBMate` (`0x01`) is only valid at `seq = 1`; `Close` (`0x02`) is
  terminal (nothing may follow); unknown class values are rejected.
- **PA-4** — the `seq = 1` event must carry a `MateProof` that passes
  `mate::verify_mate` (equality under disclosed blinds).

---

## Usage

### Pure (no node required)

```rust
use kcp_paired_attestation::{
    record::{AttestationRecord, attestation_id, commit},
    mate::{negotiate_blind, build_mate_proof, verify_mate},
    payload::Payload,
    invariants::{validate_chain, PARTY_A_COMMIT, PARTY_B_MATE},
};

// 1. Define the shared record.
let record = AttestationRecord::new([0x01u8; 32], [0x02u8; 32], 1);
let aid = attestation_id(&record).unwrap();

// 2. Each party commits under their own blind (exchanged off-band).
let blind_a = [0x11u8; 32];
let blind_b = [0x22u8; 32];
let commit_a = commit(&record, &blind_a).unwrap();
let commit_b = commit(&record, &blind_b).unwrap();

// 3. Build and verify the mate proof.
let proof = build_mate_proof(&record, blind_a, blind_b, commit_a, commit_b).unwrap();
verify_mate(&proof).unwrap();

// 4. Validate the chain off-chain.
let chain = vec![
    Payload { attestation_id: aid, seq: 0,
              event_class: PARTY_A_COMMIT, commitment: commit_a },
    Payload { attestation_id: aid, seq: 1,
              event_class: PARTY_B_MATE, commitment: commit_b },
];
validate_chain(&chain, Some(&proof)).unwrap();
```

### With node (feature `wrpc`)

```rust
use kcp_common::{wallet::{Wallet, Prefix}, wrpc::{NodeClient, NodeConfig}};
use kcp_paired_attestation::tx::{
    create_attestation_tx, append_mate_tx, DEFAULT_ATTESTATION_VALUE_SOMPI
};
// ... see examples/testnet_evidence.rs for the full flow
```

---

## Testnet evidence

Recorded 2026-06-11 on **testnet-10** (local kaspad v2.0.0, synced, DAA ~488,407,236).
Party-A commit → Party-B mate, each carrier-anchored, the two-party chain
validated off-chain (PA-1/2/3/4, mate proof equality-under-disclosed-blinds):

- attestation_id `eb457095db7f7d6d834383c901c34a9b0f015cd8fdc77a3ae2f8d5dd116520d2`
- create tx (Party A, seq 0) `42c10ca6d25295fc1d4a134fb09d750c213c0fbe4296847fc0e4fd6052fac71c`
- mate tx (Party B, seq 1) `e4f45509141e9cfba93363f73d7a2f282d0f2517ff58cfc41c5d6ebc5154236f`

Two-party on-chain datasig binding is proven viable (FACTS SS-024-v4) and is
the documented next step (needs P2SH spend plumbing).

To reproduce or refresh — run `examples/testnet_evidence.rs` against a funded
testnet wallet:

```sh
KCP_NODE_URL=ws://localhost:17210 \
KCP_KEY_FILE=/path/to/wallet.key \
cargo run -p kcp-paired-attestation --example testnet_evidence --features wrpc
```

Record the output in `FACTS.yaml` with:

```
id: KCP-PA-001
note: "v0 — unaudited — bilateral mating verified OFF-CHAIN;
       two-party on-chain datasig binding is proven viable (FACTS SS-024-v4)
       and is the next step (needs P2SH spend plumbing)"
```

---

---

## v1 on-chain two-datasig (full)

The full paired-attestation pattern is now consensus-enforced via direct
`OP_CHECKSIGFROMSTACK` (CSFS) opcodes, built without the silverscript compiler.

### What is enforced on-chain in v1

Both oracle data-signatures over the shared `msg_hash` are verified by the Kaspa
script engine at spend time. Value locked under the two-datasig covenant can only
be released when both `sig_a` (from oracle A) and `sig_b` (from oracle B) are
valid 64-byte Schnorr signatures over `msg_hash` with the respective keys embedded
in the redeem script.

| Property | Mechanism |
|---|---|
| Oracle A signature required | `OP_CHECKSIGFROMSTACK` over `msg_hash` with `pk_a` |
| Oracle B signature required | `OP_CHECKSIGFROMSTACK` over `msg_hash` with `pk_b` |
| Both keys embedded at lock time | Redeem script encodes `pk_a`, `pk_b`, `msg_hash` |
| Offline engine preflight | `verify_p2sh_spend_offline(covenants_enabled=true)` before submit |

### Redeem script shape

```text
OP_TOALTSTACK
<msg_hash> <pk_a> OP_CHECKSIGFROMSTACK OP_VERIFY
OP_FROMALTSTACK
<msg_hash> <pk_b> OP_CHECKSIGFROMSTACK
```

Satisfier (pushed before redeem in sig_script):

```text
<sig_a>  (deepest)
<sig_b>  (top)
```

### Signature format

`OP_CHECKSIGFROMSTACK` takes **64-byte** raw Schnorr data-signatures over the
32-byte `msg_hash` directly (not the transaction sighash). Both oracles can
pre-sign the attestation off-band; the covenant binds both signatures at spend.

### covenants_enabled

`OP_CHECKSIGFROMSTACK` (0xd7) requires `covenants_enabled = true`. The
`EngineCtx` used by `verify_p2sh_spend_offline` already defaults to
`EMPTY_COV_CONTEXT`, which suffices for CSFS — the opcode only checks
`vm.flags.covenants_enabled` and performs Schnorr verification; it does not
access the covenants context. No changes to `kcp-common::p2sh` were required.

### Facts basis

- **FACTS SS-024-v4**: CSFS primitive proven on rusty-kaspa v2.0.0 — valid sig
  accepted, zero/garbage/wrong-msg/wrong-key rejected on the released engine.
- **Offline engine tests** in `src/onchain.rs` (`#[cfg(test)]`): four cases
  exercised against the real engine with `covenants_enabled = true`:
  - `two_datasig_valid_both_sigs_accepted_by_engine` — ACCEPT
  - `two_datasig_wrong_sig_a_rejected` — REJECT
  - `two_datasig_wrong_sig_b_rejected` — REJECT
  - `two_datasig_swapped_sigs_rejected` — REJECT

These run with `cargo test -p kcp-paired-attestation --features wrpc`.

### Implementation note: direct opcodes, not the silverscript compiler

The CSFS redeem script is built directly with `kaspa_txscript::opcodes::codes`
(the same approach as `kcp-vault`'s multisig redeem). This does NOT require the
silverscript compiler. The upstream silverscript `checkDataSig` lowering is a
known no-op at `master@2c46231` (FACTS SS-025); a fix patch is archived outside this published repo (available on request) and is ready
for upstream contribution. The library is not blocked
on that contribution: the CSFS primitive already works in the released engine.

### v0 off-chain mating path

The v0 off-chain-mating + on-chain-lineage path (`src/tx.rs` +
`examples/testnet_evidence.rs`) remains in the crate for the privacy-preserving
disclosed-blind use case: parties prove equality-under-disclosed-blinds without
embedding the commitment hash in the script. Both paths are labelled in the code
and documented here.

---

## Caveats

- This crate does not mention KCC20, KRC-20, or KTT internally.
- The fee constant `kcp_common::tx::CARRIER_FEE_SOMPI` is fixed and
  conservative for today's testnet; adjust if mempool rules change at Toccata.
- `kaspa-bip32` and `kaspa-consensus-core` are pinned to tag `v2.0.0` of
  `rusty-kaspa`. The API may change before mainnet activation.
- The SHA-256 commitment (not Poseidon) is documented above; the Poseidon
  upgrade is deferred to KIP-16-era ZK work.
- This is not production-grade software. No security audit has been performed.
