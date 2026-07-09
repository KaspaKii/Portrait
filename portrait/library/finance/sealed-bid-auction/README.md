# finance/SealedBidAuction

A commit-reveal sealed-bid auction covenant. A bidder commits to a sealed bid by
publishing only its `blake2b` digest (`bid_commit`); in the reveal phase the
committed bidder opens the commitment on chain, and the covenant binds the
revealed value to the digest with a **true** `blake2b` hashlock — the same
on-chain hash intrinsic `Htlc` uses for its preimage, reused here to seal a bid.

**Status:** 🟡 drafted — pre-red-team, testnet-only, not audited, not mainnet-safe.

## Parameters / State

One constructor param per state field, in field order:

| Field | Type | Meaning |
|---|---|---|
| `seller` | `pubkey` | Committed auctioneer key. The only key that may `close`. |
| `bid_commit` | `bytes32` | Committed `blake2b` digest of the bidder's sealed bid. |
| `high_bid` | `int` | Current standing high bid (genesis = 0). |
| `high_bidder` | `pubkey` | Committed key of the standing high bidder; the `reveal` authority. |
| `revealed` | `int` | Count of accepted reveals (genesis = 0). |
| `closed` | `int` | One-shot auction-closed flag (genesis = 0). |

## Lifecycle

```
live --reveal(bidderSig, preimage, bid)  [blake2b(preimage)==bid_commit, bid>high_bid, closed==0] --> live  (high_bid := bid; revealed += 1)
live --close(sellerSig)                  [closed==0]                                               --> live  (closed := 1)
```

## Why it's safe by shape

- **Committed-key authorisation (C2).** `reveal` `checkSig`s against the committed
  `high_bidder`; `close` against the committed `seller` — never a caller-supplied
  pubkey. The `authorized` invariant makes this a stated, enforced property.
- **True hashlock.** `reveal` requires `blake2b(preimage) == bid_commit`, computed
  on chain by the covenant (`silverc blake2b(_)` → `OpBlake2b, 0xaa`). The bidder
  can only open the value they actually sealed.
- **Monotone-improving + one-shot close.** Each accepted reveal must strictly beat
  the standing bid (`bid > high_bid`); once the seller closes, `closed == 0` fails,
  so no further reveal or re-close can fire.

## Honest scope

- **"Highest wins" is a monotone guard, not a global-max proof.** The covenant
  enforces that each accepted reveal beats the current standing bid (`bid >
  high_bid`) — a structural shape check. It is **not** an SMT proof that the final
  standing bid is the maximum over all sealed bids submitted.
- **The covenant moves no coin.** Settlement payout is the spending wallet's job;
  the covenant complements it with authorisation + hashlock + ordering gates.
- **Semantic checks are structural/relational, not an SMT solver** (per-field, no
  cross-field flow proof).
- Pre-production, unaudited, testnet-only.

## Files

- `SealedBidAuction.portrait` — the canonical covenant source. `portrait engrave`
  lowers it to `.sil` + CTOR JSON that `silverc` accepts (exit 0).
- `SealedBidAuction.sil` — the emitted Silverscript component.
- `SealedBidAuction_ctor.json` — the emitted CTOR JSON consumed by `silverc --ctor`.
- `SealedBidAuction.json` — the `silverc`-compiled script.

## Reproduce

```sh
cd portrait
cargo run --bin portrait -- check   ../library/finance/sealed-bid-auction/SealedBidAuction.portrait
cargo run --bin portrait -- engrave ../library/finance/sealed-bid-auction/SealedBidAuction.portrait
```
