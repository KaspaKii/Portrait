# Finance covenants

This page is a tight reference for the finance-family covenants in the Portrait
library (`portrait/library/finance/`). Each entry is derived directly from its
`.portrait` source: state fields, transitions, the literal `requires(...)`
guards, and the declared `invariant ...;` lines.

> **Maturity / honest scope.** Pre-production, unaudited, testnet-only —
> perishable evidence. Covenant checks are **structural/relational**, not SMT
> proofs. `value_conserved` is per-field single-additive; `conservation_split`
> is N-field additive-delta cancellation (multiset of `+`-atoms must cancel), not
> a numeric conservation proof. None of these finance covenants is a vProg
> pattern, and none is settled live on TN10 (the live settlements are
> `CsciInstrument` and the five settled-live [cross-layer vProg patterns](./vprog.md),
> elsewhere). Coin movement is the spending wallet's job; the covenant complements it
> with the structural rules it rejects on. Quoted guards are the exact source.

The family has **12 covenants**. All are singleton (single-role) covenants that
emit one `.sil`, **except `DigitalReit`**, which is a two-role covenant
(`token` + `splitter`) and emits **2 `.sil`** — one per role.

---

## DigitalReit

**Purpose.** A two-covenant tokenised-REIT distribution waterfall: a parent
`token` (REIT share registry / distribution declarer) and a child `splitter`
(senior-first payment waterfall) linked by covenant-ID lineage. Emits **2 `.sil`**
(one per role).

### Role `token` (parent)

**State.** `pubkey trustee`, `int supply`, `int period`, `int declared`.

**Transitions.**
- `distribute(sig auth, int next_declared)` — trustee opens a new period and declares the payable amount.
  - `requires checkSig(auth, trustee);`
  - `requires next_declared >= 0;`
  - Updates: `supply` carried unchanged, `period: period + 1`, `declared: next_declared`.

### Role `splitter` (child)

**State.** `bytes32 parent_kov_id`, `pubkey trustee`, `int senior_bps`, `int paid_period`, `int paid_amount`.

**Transitions.**
- `payout(sig auth, int amount, int for_period)` — pay out one period of the waterfall; may only fire when the parent covenant's UTXO is the spending input at index 0.
  - `requires parent_kov_id == OpInputCovenantId(0);` (lineage edge: parent covenant must be input 0)
  - `requires checkSig(auth, trustee);` (committed trustee)
  - `requires senior_bps <= 10000;`
  - `requires amount >= 0;`
  - `requires for_period == paid_period + 1;` (periods strictly in order)
  - Updates: `parent_kov_id`, `trustee`, `senior_bps` carried unchanged; `paid_period: for_period`; `paid_amount: paid_amount + amount`.

**Lifecycle.** `issued -> distributing via token.distribute;` then `distributing -> distributing via splitter.payout;`.

**Invariants.** `value_conserved`, `no_undeclared_state`.

**Honest scope.** `parent_kov_id` is seeded by the deployer at genesis; the covenant enforces only the *structural* lineage edge (child cannot fire without the parent input present), not the correctness of the seeded ID.

---

## RoyaltySplit

**Purpose.** A one-source, N-payee (3-way) royalty distribution demonstrating `conservation_split` on a fan-out with more than two destination legs.

**State.** `int income_balance`, `int payee_a_balance`, `int payee_b_balance`, `int payee_c_balance`, `pubkey distributor`.

**Transitions.**
- `distribute(sig auth, int a, int b, int c)` — distribute `a + b + c` out of the pooled income leg to the three payees.
  - `requires checkSig(auth, distributor);`
  - `requires a >= 0;`
  - `requires b >= 0;`
  - `requires c >= 0;`
  - `requires a + b + c <= income_balance;`
  - Updates: `income_balance: income_balance - (a + b + c)`, `payee_a_balance: payee_a_balance + a`, `payee_b_balance: payee_b_balance + b`, `payee_c_balance: payee_c_balance + c`, `distributor` carried unchanged.

**Lifecycle.** `live -> live via pool.distribute;`.

**Invariants.** `conservation_split`, `authorized`, `no_undeclared_state`.

**Honest scope.** Structural N-field additive-delta cancellation across four legs (an internal split only — no spend out to an external output); not an SMT proof, does not reason about numeric values.

---

## TokenAllowance

**Purpose.** The ERC-20 `approve` / `transferFrom` delegated-spend shape: an owner holds a balance and grants a single spender a capped allowance to pull from it.

**State.** `pubkey owner`, `pubkey spender`, `int allowance`, `int balance`.

**Transitions.**
- `approve(sig auth, int new_allowance)` — owner path; (re)sets the spender's cap.
  - `requires checkSig(auth, owner);`
  - `requires new_allowance >= 0;`
  - Updates: `owner`, `spender` carried unchanged; `allowance: new_allowance`; `balance` carried unchanged.
- `transfer_from(sig auth, int amount)` — spender path; pulls `amount` from the owner's balance, debiting both the allowance and the balance.
  - `requires checkSig(auth, spender);`
  - `requires amount >= 0;`
  - `requires amount <= allowance;`
  - `requires amount <= balance;`
  - Updates: `owner`, `spender` carried unchanged; `allowance: allowance - amount`; `balance: balance - amount`.

**Lifecycle.** `live -> live via allowance.approve;` and `live -> live via allowance.transfer_from;`.

**Invariants.** `value_conserved`, `authorized`, `non_negative_amount`, `no_undeclared_state`.

**Honest scope.** Models a single owner→spender pair (not a mapping); `amount` is a caller-asserted integer compared against committed state, not an SMT proof; coin movement is the wallet's job.

---

## ArbiterEscrow

**Purpose.** A three-party 2-of-3 conditional-payment escrow: a `buyer` and `seller` with a neutral `arbiter` tie-breaker; any two of the three committed keys settle.

**State.** `pubkey buyer`, `pubkey seller`, `pubkey arbiter`, `int amount`, `int settled`.

**Transitions.**
- `release(sig auth_x, sig auth_y)` — settle when any two of the three committed keys authorise.
  - `requires checkSig(auth_x, buyer) && checkSig(auth_y, seller) || checkSig(auth_x, buyer) && checkSig(auth_y, arbiter) || checkSig(auth_x, seller) && checkSig(auth_y, arbiter);`
  - `requires settled == 0;` (one-shot)
  - Updates: `buyer`, `seller`, `arbiter` carried unchanged; `amount: amount` (bare carry); `settled: 1`.

**Lifecycle.** `live -> live via escrow.release;`.

**Invariants.** `value_conserved`, `authorized`, `multisig_threshold`, `no_undeclared_state`.

**Honest scope.** `multisig_threshold` is a structural count of distinct committed-key checkSigs (≥2 of buyer/seller/arbiter), not a proof the boolean combination is a true k-of-n; `amount` is a conserved bare carry, real coin movement is the wallet's.

---

## InternalTransfer

**Purpose.** A paired two-field value transfer — the smallest step beyond per-field `value_conserved`, where `conservation_split` forces the amount leaving one leg to equal the amount arriving in the other.

**State.** `int from_balance`, `int to_balance`, `pubkey owner`.

**Transitions.**
- `transfer(sig auth, int amount)` — move `amount` from the source leg to the destination leg.
  - `requires checkSig(auth, owner);`
  - `requires amount >= 0;`
  - `requires amount <= from_balance;`
  - Updates: `from_balance: from_balance - amount`, `to_balance: to_balance + amount`, `owner` carried unchanged.

**Lifecycle.** `live -> live via acct.transfer;`.

**Invariants.** `conservation_split`, `authorized`, `non_negative_amount`, `no_undeclared_state`.

**Honest scope.** Structural two-field match (increase term and decrease term are the same AST node); not an SMT proof, no fan-out, does not read on-chain coin values.

---

## StreamingVesting

**Purpose.** A single-recipient linear vesting / payment-streaming covenant: a fixed grant unlocks over a window, the recipient withdraws vested portions, and a value-bearing accumulator tracks the cumulative withdrawn.

**State.** `pubkey recipient`, `int total`, `int start`, `int duration`, `int supply`.

**Transitions.**
- `withdraw(sig auth, int amount)` — recipient withdraws a vested portion.
  - `requires checkSig(auth, recipient);`
  - `requires amount >= 0;`
  - `requires supply + amount <= total;`
  - Updates: `recipient`, `total`, `start`, `duration` carried unchanged; `supply: supply + amount`.

**Lifecycle.** `live -> live via stream.withdraw;`.

**Invariants.** `value_conserved`, `non_negative_amount`, `bounded_supply`, `no_undeclared_state`.

**Honest scope.** The per-call vested-ceiling against wall-clock is NOT enforced on-chain; the covenant enforces only committed-recipient auth, non-negativity, conservation, and that cumulative draw never exceeds the committed grant — the real linear release is the wallet/relay's relative-timelock.

---

## Subscription

**Purpose.** A recurring, rate-limited pull-payment covenant: a committed `provider` pulls a fixed `amount_per_period` from a subscriber-funded balance, no more than once per `period`.

**State.** `pubkey provider`, `pubkey subscriber`, `int amount_per_period`, `int period`, `int last_charged`, `int balance`.

**Transitions.**
- `charge(sig auth, int now_bucket)` — provider pulls one period's fee.
  - `requires checkSig(auth, provider);`
  - `requires now_bucket >= last_charged + period;` (rate limit)
  - `requires amount_per_period >= 0;`
  - `requires amount_per_period <= balance;`
  - Updates: `provider`, `subscriber`, `amount_per_period`, `period` carried unchanged; `last_charged: now_bucket`; `balance: balance - amount_per_period`.

**Lifecycle.** `live -> live via subscription.charge;`.

**Invariants.** `value_conserved`, `authorized`, `non_negative_amount`, `temporal_guard`, `no_undeclared_state`.

**Honest scope.** `now_bucket` is caller-asserted and coarse, not a wall-clock proof; the real "can this be relayed yet" decision is the engine's relative-timelock on the spending input's sequence.

---

## Htlc

**Purpose.** A hash-time-locked contract: value locked between a `sender` and `recipient` behind a real blake2b hashlock and a deadline, with a one-shot `settled` flag (claim XOR refund, never both).

**State.** `pubkey sender`, `pubkey recipient`, `bytes32 hashlock`, `int deadline`, `int settled`.

**Transitions.**
- `claim(sig auth, bytes32 preimage)` — recipient settles by revealing the preimage.
  - `requires checkSig(auth, recipient);`
  - `requires blake2b(preimage) == hashlock;` (true on-chain hashlock)
  - `requires settled == 0;` (one-shot)
  - Updates: all fields carried unchanged except `settled: 1`.
- `refund(sig auth, int now_bucket)` — sender claws funds back after the deadline.
  - `requires checkSig(auth, sender);`
  - `requires now_bucket >= deadline;` (temporal gate)
  - `requires settled == 0;` (one-shot)
  - Updates: all fields carried unchanged except `settled: 1`.

**Lifecycle.** `live -> live via htlc.claim;` and `live -> live via htlc.refund;`.

**Invariants.** `value_conserved`, `temporal_guard`, `no_undeclared_state`.

**Honest scope.** The blake2b hashlock IS computed on-chain by the covenant (`blake2b(_)` → engine intrinsic). The deadline gate uses a caller-asserted coarse `now_bucket`, not wall-clock; no value-bearing field changes — the locked value is governed by the spending wallet's output script.

---

## Escrow

**Purpose.** A two-party, deadline-gated conditional-payment escrow: `amount` locked between a `buyer` and `seller`, with a `deadline` and a one-shot `settled` flag (release XOR refund).

**State.** `pubkey buyer`, `pubkey seller`, `coin amount`, `int deadline`, `int settled`.

**Transitions.**
- `release(sig auth)` — seller settles (happy path).
  - `requires checkSig(auth, seller);`
  - `requires settled == 0;` (one-shot)
  - Updates: `buyer`, `seller`, `amount` (bare carry), `deadline` carried unchanged; `settled: 1`.
- `refund(sig auth, int now_bucket)` — buyer claws funds back after the deadline.
  - `requires checkSig(auth, buyer);`
  - `requires now_bucket >= deadline;` (temporal gate)
  - `requires settled == 0;` (one-shot)
  - Updates: `buyer`, `seller`, `amount` (bare carry), `deadline` carried unchanged; `settled: 1`.

**Lifecycle.** `live -> live via escrow.release;` and `live -> live via escrow.refund;`.

**Invariants.** `value_conserved`, `no_undeclared_state`.

**Honest scope.** `amount` is typed `coin` (strictly conserved Portrait surface type — only a bare carry is permitted; lowered to `int` in the emitted `.sil`). The deadline gate is a coarse caller-asserted `now_bucket`; the covenant does not move coin.

---

## CollateralVault

**Purpose.** A single-owner over-collateralised debt position (CDP-style): deposit collateral, draw debt against it gated by a structural collateralisation ratio, repay debt.

**State.** `pubkey owner`, `int collateral`, `int debt`, `int min_ratio`.

**Transitions.**
- `deposit(sig auth, int amount)` — grow the collateral leg.
  - `requires checkSig(auth, owner);`
  - `requires amount >= 0;`
  - Updates: `owner` unchanged; `collateral: collateral + amount`; `debt`, `min_ratio` unchanged.
- `borrow(sig auth, int amount)` — draw debt, gated by the structural ratio guard on post-borrow debt.
  - `requires checkSig(auth, owner);`
  - `requires amount >= 0;`
  - `requires collateral >= debt * min_ratio + amount * min_ratio;` (distributed form — precedence-stable)
  - Updates: `owner`, `collateral` unchanged; `debt: debt + amount`; `min_ratio` unchanged.
- `repay(sig auth, int amount)` — shrink the debt leg.
  - `requires checkSig(auth, owner);`
  - `requires amount >= 0;`
  - `requires amount <= debt;`
  - Updates: `owner`, `collateral` unchanged; `debt: debt - amount`; `min_ratio` unchanged.

**Lifecycle.** `live -> live via vault.deposit;`, `live -> live via vault.borrow;`, `live -> live via vault.repay;`.

**Invariants.** `authorized`, `non_negative_amount`, `no_undeclared_state`.

**Honest scope.** The ratio guard is a single integer-multiply committed-state comparison (distributed to stay precedence-stable through engraving) — NOT an economic soundness proof: no fractional ratios, no oracle/price feed, no liquidation-safety proof.

---

## SealedBidAuction

**Purpose.** A commit-reveal sealed-bid auction: bidders commit a blake2b digest of their sealed bid off-chain, then reveal on-chain bound to the committed digest; the seller closes the auction one-shot.

**State.** `pubkey seller`, `bytes32 bid_commit`, `int high_bid`, `pubkey high_bidder`, `int revealed`, `int closed`.

**Transitions.**
- `reveal(sig auth, bytes32 preimage, int bid)` — committed bidder opens their sealed bid; must beat the standing high bid.
  - `requires checkSig(auth, high_bidder);`
  - `requires blake2b(preimage) == bid_commit;` (true on-chain hashlock)
  - `requires bid > high_bid;` (monotone-improving)
  - `requires closed == 0;`
  - Updates: `seller`, `bid_commit` unchanged; `high_bid: bid`; `high_bidder` unchanged; `revealed: revealed + 1`; `closed` unchanged.
- `close(sig auth)` — seller closes the auction (one-shot).
  - `requires checkSig(auth, seller);`
  - `requires closed == 0;`
  - Updates: all fields carried unchanged except `closed: 1`.

**Lifecycle.** `live -> live via auction.reveal;` and `live -> live via auction.close;`.

**Invariants.** `authorized`, `no_undeclared_state`.

**Honest scope.** The blake2b hashlock IS enforced live by the covenant. "Highest bid wins" is the structural monotone-improving guard `bid > high_bid` per accepted reveal, NOT an SMT proof the final standing bid is the global maximum; no coin field is moved.

---

## InternalSplit

**Purpose.** An N-field (3-way) internal value split: one source leg split across two destination legs, exercising `conservation_split` beyond the paired two-field transfer.

**State.** `int pool_a_balance`, `int pool_b_balance`, `int pool_c_balance`, `pubkey owner`.

**Transitions.**
- `rebalance(sig auth, int x, int y)` — split `x + y` out of the source leg: `x` into leg b, `y` into leg c.
  - `requires checkSig(auth, owner);`
  - `requires x >= 0;`
  - `requires y >= 0;`
  - `requires x + y <= pool_a_balance;`
  - Updates: `pool_a_balance: pool_a_balance - (x + y)`, `pool_b_balance: pool_b_balance + x`, `pool_c_balance: pool_c_balance + y`, `owner` carried unchanged.

**Lifecycle.** `live -> live via pool.rebalance;`.

**Invariants.** `conservation_split`, `authorized`, `no_undeclared_state`.

**Honest scope.** Structural N-field additive-delta cancellation (multiset of added `+`-atoms equals subtracted), an internal split only — not a spend out to an external output, not an SMT proof.
