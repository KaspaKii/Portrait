# Enforcement matrix — what each pattern's `.sil` actually enforces

**Status:** 🟡 honest inventory, pre-review, TN10 only, not audited, not
mainnet-safe.

This is the central "say what is enforced" document for `library/`. Every row is
grounded in the *emitted* `.sil` (read the script, not the pattern name or the
`.portrait` prose). Each declared invariant / guarantee is classified into one of
four buckets. Names like `value_conserved`, `temporal_guard`, `TimeVault`, and
`HTLC` are **kept as-is for now** (label-now, rename-later); their true scope is
this table, not the name.

## The four buckets

- **SCRIPT-ENFORCED** — Kaspa consensus (the compiled Silverscript covenant)
  checks it on every spend. In this library that means: `checkSig(auth, <key>)`
  against a key **committed** in covenant state; covenant-program continuity
  (`binding = cov` — the successor must carry the same covenant); one-shot state
  predicates (`require(settled == 0)` / `released` / `closed` etc. with the flag
  flipped in the successor); and structural successor-state predicates the
  transition hard-codes (e.g. `seq: prev + 1`, `paid_period: for_period` with
  `for_period == paid_period + 1`). A **structural counter** the successor
  advances (e.g. `batch_count >= 1` driving `seq: seq + batch_count`) is
  SCRIPT-ENFORCED — it only moves committed state; a value/amount predicate is
  **not** (see WALLET-ASSUMED), because no on-chain coin binding backs it. A
  `pays(k, payee, amount)` **output binding** (B2) IS SCRIPT-ENFORCED: it emits
  `require(tx.outputs[k].value == <committed amount>)` (→ `OpTxOutputAmount`) and
  `require(tx.outputs[k].scriptPubKey == byte[](new ScriptPubKeyP2PK(<committed
  payee>)))` (→ `OpTxOutputSpk`), so consensus binds the real output value AND
  destination to committed state. This is the one row family that lifts a payout
  above WALLET-ASSUMED. The `amount` operand qualifies one of two ways — it is
  **value-bearing** (typed `coin`, or named `balance`/`amount`/`supply`), **or**
  it is a committed `int` the SAME entrypoint **draws down** (the successor sets
  `<value-bearing field>: <field> - <term>` with the operand as a `+`-atom of
  `<term>`, and every `+`-atom of `<term>` established non-negative there). The
  drawdown link is what proves the paid quantity is the quantity the model gives
  up; a rename to a value-bearing NAME does not, and is rejected as a route.
  **Evidence scope (honest):** the output-introspection
  *opcode semantics* + byte-encoding are proven accept/reject against the pinned
  engine (`v2.0.0` = `90dbf07`) in isolation in
  `portrait-emit/tests/output_binding_engine.rs` (matching output ACCEPTs; wrong
  amount / wrong payee REJECT); the emitted `ScriptPubKeyP2PK` byte sequence is
  golden-checked against silverc's real output; and the fully **composed**
  `Escrow.sil` (a **terminal** `binding = auth` spend — no successor, the coin is
  released to the committed payee and the UTXO is consumed) is proven to
  *compile* under silverc (exit 0). A **composed end-to-end on-engine spend**
  (valid signature + the covenant runtime admitting 0 covenant-successor outputs)
  is **pending** —
  silverscript-lang, whose covenant sig-script/ABI would be needed to assemble it,
  pins a floating pre-release engine branch and cannot be added without violating the
  `v2.0.0` engine pin. Scope caveats that ALWAYS apply: (M4) the payee's spk form
  is chosen from its DECLARED TYPE — a `pubkey` payee binds a **32-byte-Schnorr
  P2PK** spk, a `byte[32]` payee binds a **P2SH script-hash** spk — and the checker
  cannot see which form the payee's REAL settlement address uses, so committing the
  wrong one leaves that path **dead** for them (funds recoverable only via the
  covenant's other paths); (L1) it binds **only** `output[k]` — no
  value-conservation / transaction-mass (KIP-9) check, so an over-funded covenant
  lets the surplus be spender-routed; (L2) it is only as trustworthy as the
  instantiation ceremony that committed `payee`/`amount`. An `after(deadline)`
  **time gate** (B1) IS likewise SCRIPT-ENFORCED: it emits
  `require(tx.time >= <committed deadline>)`, which silverc routes to
  `OpCheckLockTimeVerify`. **The no-early-spend guarantee is TWO consensus rules,
  and the emitted opcode is only HALF** — do not read this row as CLTV enforcing
  elapsed time by itself:
  1. **CLTV** (the txscript opcode `OpCheckLockTimeVerify`, opcodes/mod.rs
     :1039,1057) enforces that the tx **commits** a `lock_time >=` the committed
     `deadline` (domain-matched) **AND** the spending input is **non-final**
     (sequence != max, defeating the final-sequence bypass). It reads only the
     spender-set `lock_time` FIELD — it has **no** access to the block DAA score,
     so it does **not** by itself prove the deadline has elapsed.
  2. The actual **no-early-INCLUSION** rule is the SEPARATE consensus finalization
     check `check_tx_is_finalized`
     (`consensus/src/processes/transaction_validator/tx_validation_in_header_context.rs:72-93`):
     a non-final tx with `lock_time = L` is admissible into the blockDAG only once
     `block_daa_score > L`. This is the load-bearing "time has passed" half; it
     lives OUTSIDE txscript.

  Together they make consensus bar a `release` from a block until the DAA score
  passes the committed deadline. This is the row family that lifts a time gate
  above WALLET-ASSUMED. **Evidence scope (honest):** the CLTV *opcode semantics*
  (half 1) are proven accept/reject against the pinned engine (`v2.0.0` =
  `90dbf07`) in isolation in `portrait-emit/tests/time_gate_engine.rs` (spend
  at/after deadline with a non-final input ACCEPTs; early spend REJECTs; the
  final-sequence bypass at the deadline REJECTs); and the fully **composed**
  `TimeVault.sil` is proven to *compile* under silverc (exit 0). **NOT** proven by
  a unit test: the consensus-finalization half (rule 2) — it is `pub(crate)`,
  reachable only through the full VirtualProcessor pipeline, so it is out of scope
  for an isolated txscript-opcode test (logged, not silently omitted). A
  **composed end-to-end on-engine spend** is also **pending** for the same
  `v2.0.0`-pin reason as the `pays` binding above. Scope caveats that ALWAYS apply:
  (L1) the committed `deadline` and the spending tx's lock time must be in the SAME
  domain (a DAA score below `LOCK_TIME_THRESHOLD = 500_000_000_000`, or a Unix time
  at/above it) — the covenant cannot check which domain the committed value is in;
  (L3) a committed `deadline` of `0` (which maps to `LockTimeType::Finalized`) or
  any value `<=` the instantiation DAA score opens the gate fully — the ceremony
  must commit a real FUTURE deadline. Both are ceremony preconditions, not
  compile-time diagnostics (the committed value is not visible at engrave time).
- **MODEL-ONLY** — the Portrait checker / Lens reasons about the *model* (the
  declared state fields); the emitted `.sil` does **not** enforce it on-chain.
  `invariant value_conserved` (no value-bearing *model* field mutates outside the
  carry / single-additive shape, and every `+`-atom of the adjustment term must
  be established non-negative by the same entrypoint — otherwise a negative term
  inverts the operator, turning `f - e` into an increase and `f + e` into a
  drain; it does **not** bind on-chain output value or payee); `invariant
  conservation_split` (structural N-field internal cross-field cancellation over
  model fields, under the same per-leg non-negativity requirement — cancellation
  alone cannot see that a negative term REVERSES the transfer); `invariant
  temporal_guard`
  (existence of a committed-time gate SHAPE only); `no_undeclared_state`
  (structural lifecycle wellformedness).

  > **WHICH FIELDS THESE COVER — read this before relying on either.** Both
  > invariants act ONLY on fields the checker considers *value-bearing*, and that
  > set is a NAME/TYPE rule, not an inference:
  >
  > * `value_conserved` — a field is value-bearing iff its declared type is
  >   `coin`, **or** its name is exactly `balance`, `amount`, or `supply`.
  > * `conservation_split` — the same, plus any field whose name **ends in**
  >   `balance` (e.g. `from_balance`, `senior_balance`).
  >
  > A value field named anything else (`funds`, `principal`, `escrowed`) is
  > **entirely outside both checks** — the invariant will be declared, will
  > report ok, and will have verified nothing about it. `portrait check` now
  > prints a WARNING when a role declares one of these invariants and no field on
  > that role is value-bearing, but a role with one covered field and three
  > uncovered ones gets no warning and no coverage of the three. Name the field
  > `<x>balance`/`amount`/`supply`, or type it `coin`, if you want it checked.
  >
  > Note on labels: the adjustment-term sign rule above is review item
  > **A6-sign**; it is unrelated to **A6** (`payout_bound`) further down. The two
  > share a number, not a rule.
- **WALLET-ASSUMED** — holds only if the spending wallet cooperates. Caller-asserted
  time gates `now_bucket >= <committed>` (`now_bucket` is a spender-supplied
  argument, **not** consensus time — no CLTV/sequence timelock is emitted). A sound
  consensus alternative now exists — the `after(deadline)` clause (B1,
  SCRIPT-ENFORCED above) — so a `now_bucket`-gated pattern is WALLET-ASSUMED only
  until it is migrated to `after(...)`; TimeVault is the first migrated pattern.
  Amount/cap/supply guards
  (`amount <= limit`, `amount <= balance`, `supply + amount <= total`, ...): the
  numeric predicate over the *argument* is evaluated on-chain, but the argument is
  **not bound to any real output value** — `coin`/amount lowers to a plain `int`
  and no `.sil` constrains output value or payee. So the guard is only as true as
  the wallet's choice to pass an argument matching the coin it actually moves.
- **ENGINE-ASSUMED** — the vProg (`library/vprog/*`) STARK-validity path. The
  guest predicate's soundness is enforced by the engine's tag-`0x21` ZK
  precompile via a **separate** raw `kcp-pq-anchor` script — **not** by the
  emitted covenant. Two caveats apply to every vProg row: (1) the developer must
  author the guest `predicate()` — by DEFAULT the emitted guest's `predicate()`
  body is a `compile_error!`, so it **REFUSES TO BUILD** until real verification
  logic is written; `atelier-build --allow-unimplemented-vprog` opts into a
  true-returning placeholder instead, but only under a loud `// WARNING:
  UNIMPLEMENTED …` banner. So a shipped guest either carries real logic or a
  conscious, loudly-marked placeholder — it can no longer silently "prove" nothing
  via a true-returning stub. (2) `require(proof_cov_id == OpInputCovenantId(<idx>))`
  binds the covenant UTXO at input `<idx>`; the index is now **parametric (default
  0)** — the default **assumes the covenant UTXO is input 0**, and a spend where it
  is not must emit the correct index or the guard binds the wrong input.

Anything in WALLET-ASSUMED or ENGINE-ASSUMED (and MODEL-ONLY) is **outside**
consensus enforcement by the emitted covenant. Do not read a row as a consensus
guarantee unless it is SCRIPT-ENFORCED.

### Formula-bearing structural obligations (checker-level meta-invariants)

Two invariants do not add a *new* on-chain guarantee — they make an EXISTING
SCRIPT-ENFORCED clause **mandatory and non-deletable**, verified STRUCTURALLY at
compile time. Each is a check that the named entrypoint *carries the matching
consensus gate the emitter lowers*; neither is an SMT proof of a temporal or
value obligation.

- **`invariant <name>: <entry> => after(<deadline>);`** (A4-full, formula-bearing
  temporal) — the checker verifies that the named `entry` is a `mode = transition`
  entrypoint carrying an `after(<deadline>)` clause with exactly this deadline (the
  clause that lowers to `OpCheckLockTimeVerify`; a `Sum` window `after(a + b)`
  matches either operand order). This is STRICTLY STRONGER than the existence-only
  `temporal_guard` (which only asserts *some* mutating transition carries *some*
  committed-time gate): the formula form pins the gate to a NAMED entrypoint and
  makes deleting its `after(...)` clause a compile error. **Entrypoint-name scope:**
  `entry` binds by entrypoint NAME across roles — EVERY role that declares an
  entrypoint of that name must carry the matching clause (a two-role app where
  `a.refund` gates but `b.refund` does not is rejected). Honest scope: it certifies
  the entrypoint CARRIES the matching consensus gate — it does NOT SMT-discharge
  "every value-moving path meets the deadline". The underlying `after(...)` clause's
  own consensus guarantee (and its L1/L3 ceremony caveats) is the SCRIPT-ENFORCED
  (time_gate) row for that pattern.
- **`#[covenant(mode = transition, supply_change = <A>)]`** (A2-full) — a supply
  change is an explicit SIGNED capability, **NOT a function name**. The old
  `mint*`/`burn*` name heuristic is retired: naming an entry `mint` buys nothing.
  Declaring `supply_change = A` (a) WAIVES the entry from value-conservation
  checking (`value_conserved` / `conservation_split`) — a supply change
  legitimately does not conserve — and (b) is CHECKED unconditionally: the named
  authority `A` must be a COMMITTED key (role param / state field) AND GUARANTEED
  to sign on every satisfying path — a sound, commutative per-key check: `A`'s
  `checkSig(_, A)` must fire under every `&&`/`||` combination, so it is never
  satisfiable through a `||` branch or a negated arm (the verdict is arm-order
  independent). A supply change must also release NO coin: it may NOT carry a
  `pays(...)` clause and may NOT be a terminal spend — which is what soundly lets
  `payout_bound` exclude it. Fail to commit `A`, leave it only in a disjunctive
  arm, or attach a payout, and the pattern is REJECTED. **Honest scope: this is a
  CHECKED-MODEL capability, NOT
  on-chain minted supply.** It certifies "the committed supply counter only moves
  under `A`'s signature and the model is deliberately waived from conservation" —
  it does NOT, and a UTXO covenant CANNOT, inflate real L1 coin (the field is the
  covenant's own committed integer). Real coin movement stays WALLET-ASSUMED.
  Demonstrator: `finance/MintableToken` (mint gated by a committed `issuer`).
- **`invariant payout_bound;`** (A6) — the checker requires every recognized
  settling transition (a `mode = transition` path without a `supply_change`
  capability — and a supply change is ENFORCED to release no coin, so excluding it
  is sound: see the `supply_change` bullet) to carry at least
  one `pays(...)` clause, making the payout binding a mandatory, non-deletable
  obligation. Deleting the `pays(...)` on a settling path is a compile error.
  A settling transition is recognized in one of two ways. First, a **TERMINAL**
  transition (a lifecycle edge marked `terminal`, which releases the coin and ends
  the lifecycle — no successor covenant) settles by construction. Second, a
  successor-carrying transition settles when it flips a one-shot flag; **`settles`
  is a RECOGNIZER, not a complete settlement detector** — it matches EXACTLY three
  one-shot-flag flip shapes: (1) int-literal flip — `require f == 0;` + return
  `f: <nonzero int>`; (2) computed int flip — `require f == 0;` + return
  `f: f + <nonzero int>`; (3) bool flip — `require f == false;` + return `f: true`.
  A non-terminal settlement written OUTSIDE these shapes is **NOT recognized**, so
  `payout_bound` obligates only recognized settlements — an author must write the
  settlement in a recognized shape (or mark the lifecycle edge `terminal`). **Fail-loud on vacuity:** a declared `payout_bound` that
  recognizes ZERO settling transitions is REJECTED (a vacuous pass must not pose as
  enforcement); `explain` prints the recognized-settlement count as a coverage
  signal. Any recognized one-shot-flag flip is treated as a settlement, so
  co-declaring `payout_bound` with a *non-paying* one-shot flag (e.g. an
  init/activation flag) will correctly reject — do not do that. Honest scope:
  `payout_bound` is **EXISTENCE-ONLY** — it requires *a* `pays(...)` to exist on the
  settling path; it does **NOT** verify the `pays` binds THIS settlement's own coin
  or the correct payee (a committed-but-unrelated `pays(...)` satisfies it; payee/
  amount validity is checked separately by the `pays` rules, the SCRIPT-ENFORCED
  (output_bound) row). It is NOT a value-conservation / KIP-9 mass proof — the L1
  surplus caveat still applies (a spend may attach further inputs/outputs the
  covenant does not constrain), and it does not overload `value_conserved`.

## Matrix

| Pattern (`.sil`) | Declared invariant / guarantee | Bucket | What the emitted `.sil` actually does |
|---|---|---|---|
| **custody/TimeVault** | key authorisation | SCRIPT-ENFORCED | `checkSig(auth, owner)` on `release`, `checkSig(auth, recovery)` on `claw` — both committed keys |
| custody/TimeVault | one-shot | SCRIPT-ENFORCED | `require(released == 0)` on both paths; successor sets `released: 1` |
| custody/TimeVault | covenant continuity | SCRIPT-ENFORCED | `binding = cov` — successor must carry the same covenant |
| custody/TimeVault | `after(unlock_bucket)` time gate | **SCRIPT-ENFORCED (time_gate)** | `require(tx.time >= prev_states[0].unlock_bucket)` → `OpCheckLockTimeVerify`. TWO consensus rules (see bucket note): CLTV forces the tx to COMMIT a `lock_time >= unlock_bucket` on a NON-FINAL input (defeats the final-sequence bypass); the SEPARATE finalization rule `check_tx_is_finalized` then bars inclusion until `block_daa_score > unlock_bucket`. **CLTV half proven accept/early-reject/bypass-reject in isolation on the pinned engine (`time_gate_engine.rs`); the composed `TimeVault.sil` compiles (silverc exit 0). NOT unit-tested: the finalization half (`pub(crate)`, out of txscript scope); composed on-engine spend pending.** `unlock_bucket` and tx lock time must share a domain (L1); a committed `unlock_bucket` of 0 / ≤ the instantiation DAA score is no gate (L3) |
| custody/TimeVault | `value_conserved` | MODEL-ONLY | model-field carry only; no output value/payee bound in `.sil` |
| custody/TimeVault | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **custody/DeadMansSwitch** | key authorisation | SCRIPT-ENFORCED | `heartbeat`→`checkSig(owner)`, `claim`→`checkSig(heir)` |
| custody/DeadMansSwitch | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| custody/DeadMansSwitch | `authorized` | SCRIPT-ENFORCED | every mutating path binds a committed key |
| custody/DeadMansSwitch | `after(last_active + timeout)` (claim time gate) / `temporal_guard` | **SCRIPT-ENFORCED (time_gate)** | `require(tx.time >= prev_states[0].last_active + prev_states[0].timeout)` → `OpCheckLockTimeVerify` on the committed window SUM (same `push;OpCheckLockTimeVerify` shape proven in `time_gate_engine.rs`, threshold computed on-stack from two committed atoms); defeats the final-sequence bypass on a non-final input, with the SEPARATE finalization rule barring inclusion until `block_daa_score > last_active + timeout`. **CLTV half proven in isolation; composed `DeadMansSwitch.sil` compiles (silverc exit 0); finalization half not unit-tested; composed on-engine spend pending.** The caller-asserted `now_bucket >= last_active + timeout` compare is RETAINED (it anchors the successor `last_active`; that anchor advance stays WALLET-ASSUMED). Domain-match (L1) + future-window (L3) preconditions apply |
| custody/DeadMansSwitch | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **custody/SpendingLimitVault** | `authorized` | SCRIPT-ENFORCED | `checkSig(auth, owner)` on `withdraw` |
| custody/SpendingLimitVault | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| custody/SpendingLimitVault | `spending_cap` / `non_negative_amount` (`amount <= limit`, `amount <= balance`, `amount >= 0`) | WALLET-ASSUMED | numeric guards over spender arg `amount`; not bound to output value |
| custody/SpendingLimitVault | `value_conserved` | MODEL-ONLY | per-field shape only |
| custody/SpendingLimitVault | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/ArbiterEscrow** | `multisig_threshold` (2-of-3) | SCRIPT-ENFORCED | disjunction of two-committed-key `checkSig` arms |
| finance/ArbiterEscrow | one-shot | SCRIPT-ENFORCED | `require(settled == 0)`; successor `settled: 1` |
| finance/ArbiterEscrow | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/ArbiterEscrow | `release` payout (`pays(0, seller, amount)`) — output value + payee | **SCRIPT-ENFORCED (output_bound)** | `require(tx.outputs[0].value == amount)` (`OpTxOutputAmount`) + `require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2PK(seller)))` (`OpTxOutputSpk`); consensus binds output[0] to pay the committed `amount` to the committed `seller` on any 2-of-3 settlement. **Opcode semantics proven accept/reject in isolation on the pinned engine (`output_binding_engine.rs`); the composed `ArbiterEscrow.sil` compiles (silverc exit 0); composed on-engine spend pending.** payee committed as a `pubkey`, so the binding is to a **32-byte-Schnorr P2PK** spk (M4) — a seller/buyer whose real settlement address is a script hash must be instantiated with a `byte[32]` payee instead (→ `ScriptPubKeyP2SH`), else that path is dead for them; binds only output[0], no mass check (L1) — the surplus over `amount` in an over-funded covenant is spender-routed and `amount` is not tied to the deposited coin, so this is a payee+amount binding, NOT full escrow value-safety. Payee is fixed to `seller` (a policy choice; a buyer-favoured refund split is out of single-output scope) |
| finance/ArbiterEscrow | value conservation | MODEL-ONLY | `amount` state field carried (bare carry); the covenant declares `authorized` + `multisig_threshold`, NOT `value_conserved`, since `release` moves value OUT via the output_bound row above |
| finance/ArbiterEscrow | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/CollateralVault** | `authorized` | SCRIPT-ENFORCED | `checkSig(owner)` on every path |
| finance/CollateralVault | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/CollateralVault | collateralisation ratio (`collateral >= (debt+amount)*min_ratio`) | WALLET-ASSUMED | numeric guard over model fields + spender arg; not bound to real coin |
| finance/CollateralVault | `non_negative_amount` | WALLET-ASSUMED | `require(amount >= 0)` on spender arg |
| finance/CollateralVault | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/DepositInsurancePool** | `authorized` | SCRIPT-ENFORCED | `deposit`→`checkSig(owner)`, `payout_claim`→`checkSig(claims_authority)` |
| finance/DepositInsurancePool | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/DepositInsurancePool | payout ≤ balance / `non_negative_amount` | WALLET-ASSUMED | numeric guard on spender arg; not bound to output value |
| finance/DepositInsurancePool | `value_conserved` | MODEL-ONLY | per-field shape |
| finance/DepositInsurancePool | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/Escrow** | key authorisation | SCRIPT-ENFORCED | `release`→`checkSig(seller)`, `refund`→`checkSig(buyer)` |
| finance/Escrow | terminal spend (release-XOR-refund) | SCRIPT-ENFORCED | both `release` and `refund` are **TERMINAL** (`binding = auth`, `mode = verification`, **NO successor**): each SPENDS the single escrow UTXO, releasing the coin to the committed payee (release→`seller`, refund→`buyer`) and consuming the UTXO. There is no covenant continuity and no `settled` flag — because a UTXO is spent exactly once, the two paths are mutually exclusive **by construction** (whichever fires first consumes the coin; the other can never fire). Emitted as a `binding = auth` verification function reading state via the singular `prev_state.<field>` accessor. **Composed on-engine spend PENDING (load-bearing):** `to = 1` is silverc's minimum (`to = 0` is rejected), but whether the covenant RUNTIME admits a spend producing 0 covenant-successor outputs under `binding = auth` + `to = 1` is UNVERIFIED on the `v2.0.0` pin (upstream covenant-ABI pin bucket, same as B2/B1). If the runtime requires a successor, a terminal UTXO is stuck — **do NOT deploy a terminal covenant to a value-bearing UTXO until proven.** See [KNOWN-ISSUES.md](../KNOWN-ISSUES.md) (KI-1) |
| finance/Escrow | `after(deadline)` (refund time gate) | **SCRIPT-ENFORCED (time_gate)** | `require(tx.time >= prev_state.deadline)` → `OpCheckLockTimeVerify`. CLTV forces the tx to COMMIT a `lock_time >= deadline` on a NON-FINAL input (defeats the final-sequence bypass); the SEPARATE finalization rule then bars inclusion until `block_daa_score > deadline`. **CLTV half proven accept/early-reject/bypass-reject in isolation on the pinned engine (`time_gate_engine.rs`); the composed `Escrow.sil` compiles (silverc exit 0). NOT unit-tested: the finalization half (`pub(crate)`, out of txscript scope); composed on-engine spend pending.** `deadline` and tx lock time must share a domain (L1); a committed `deadline` of 0 / ≤ the instantiation DAA score is no gate (L3) |
| finance/Escrow | `release` payout (`pays(0, seller, amount)`) + `refund` payout (`pays(0, buyer, amount)`) — output value + payee | **SCRIPT-ENFORCED (output_bound)** | each path emits `require(tx.outputs[0].value == amount)` (`OpTxOutputAmount`) + `require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2PK(<payee>)))` (`OpTxOutputSpk`); consensus binds output[0] to pay the committed `amount` to the committed payee (release→`seller`, refund→`buyer`; the two paths are mutually exclusive by UTXO consumption — each is terminal, release-XOR-refund). **Opcode semantics proven accept/reject in isolation on the pinned engine (`output_binding_engine.rs`), including the TERMINAL shape — a pays-bound spend producing ZERO covenant-successor outputs accepts the committed payee and rejects a wrong payee; the composed terminal `Escrow.sil` compiles (silverc exit 0); composed on-engine spend pending (see bucket note).** payee committed as a `pubkey`, so the binding is to a **32-byte-Schnorr P2PK** spk (M4) — a seller/buyer whose real settlement address is a script hash must be instantiated with a `byte[32]` payee instead (→ `ScriptPubKeyP2SH`), else that path is dead for them; binds only output[0], no mass check (L1) — the surplus over `amount` in an over-funded covenant is spender-routed and `amount` is not tied to the deposited coin, so this is a payee+amount binding, NOT full escrow value-safety |
| finance/Escrow | value handling (no model carry) | MODEL-ONLY | each path is TERMINAL, so there is **no successor state** to carry `amount` — the coin is released via the output_bound row above. Escrow declares `authorized`, NOT `value_conserved` (both paths move value OUT, which a conservation invariant would falsify) |
| finance/Escrow | `payout_bound` (A6) | **CHECKER-ENFORCED (structural)** | compile-time obligation that every recognized settling transition carries a `pays(...)` clause. The recognizer now also treats a **TERMINAL** transition (a lifecycle edge marked `terminal`, which releases the coin and ends the lifecycle) as a settling transition, so here `release`/`refund` are recognized as terminal settles — makes the `release`/`refund` output_bound rows above MANDATORY and non-deletable (2 settling transitions recognized). EXISTENCE-ONLY: it requires *a* `pays(...)` to exist, NOT that it binds THIS settlement's own coin/payee (payee/amount validity checked by the `pays` rows); a settlement written outside the three recognized flip shapes is not recognized (see prose); NOT a value-conservation / KIP-9 mass proof — the L1 surplus caveat still applies |
| finance/Escrow | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/Htlc** | key authorisation | SCRIPT-ENFORCED | `claim`→`checkSig(recipient)`, `refund`→`checkSig(sender)` |
| finance/Htlc | hashlock | SCRIPT-ENFORCED | `require(blake2b(preimage) == hashlock)` against committed `hashlock` |
| finance/Htlc | one-shot | SCRIPT-ENFORCED | `require(settled == 0)`; successor `settled: 1` |
| finance/Htlc | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/Htlc | `after(deadline)` (refund time gate) / `temporal_guard` | **SCRIPT-ENFORCED (time_gate)** | `require(tx.time >= prev_states[0].deadline)` → `OpCheckLockTimeVerify` (defeats the final-sequence bypass on a non-final input); the SEPARATE finalization rule bars inclusion until `block_daa_score > deadline`. **CLTV half proven in isolation (`time_gate_engine.rs`); composed `Htlc.sil` compiles (silverc exit 0). NOT unit-tested: the finalization half; composed on-engine spend pending.** Domain-match (L1) + future-`deadline` (L3) ceremony preconditions apply |
| finance/Htlc | `refund_after_deadline: refund => after(deadline)` (A4-full) | **CHECKER-ENFORCED (structural)** | compile-time obligation that the `refund` entrypoint carries the matching `after(deadline)` clause — pins the time_gate row above to `refund` and makes deleting its `after(...)` a compile error. STRICTLY STRONGER than existence-only `temporal_guard`; certifies the entrypoint CARRIES the matching consensus gate, NOT an SMT-discharged temporal obligation |
| finance/Htlc | `value_conserved` / `temporal_guard` / `no_undeclared_state` | MODEL-ONLY | per-field shape + existence-only time-gate shape + lifecycle wellformedness |
| **finance/InternalSplit** | `authorized` | SCRIPT-ENFORCED | `checkSig(owner)` on `rebalance` |
| finance/InternalSplit | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/InternalSplit | `conservation_split` (`a-(x+y)`, `b+x`, `c+y`) | MODEL-ONLY | structural cross-field cancellation over model fields; no coin bound |
| finance/InternalSplit | `x,y >= 0`, `x+y <= pool_a` | WALLET-ASSUMED | numeric guards over spender args |
| finance/InternalSplit | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/KycGatedTransfer** | `authorized` | SCRIPT-ENFORCED | `checkSig(holder)` |
| finance/KycGatedTransfer | KYC gate | SCRIPT-ENFORCED | `require(allowed == 1)` against committed `allowed` flag |
| finance/KycGatedTransfer | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/KycGatedTransfer | `conservation_split` | MODEL-ONLY | model-field cancellation; no coin bound |
| finance/KycGatedTransfer | `non_negative_amount`, `amount <= from_balance` | WALLET-ASSUMED | numeric guards over spender arg |
| finance/KycGatedTransfer | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/LiquidatableLoan** | key authorisation | SCRIPT-ENFORCED | `repay`→`checkSig(borrower)`, `liquidate`→`checkSig(liquidator)` |
| finance/LiquidatableLoan | one-shot liquidation | SCRIPT-ENFORCED | `require(liquidated == 0)`; `liquidate` sets `liquidated: 1` |
| finance/LiquidatableLoan | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/LiquidatableLoan | under-collateralisation trigger (`collateral < debt*min_ratio`) | WALLET-ASSUMED | numeric guard over model fields; not bound to real coin/oracle |
| finance/LiquidatableLoan | `non_negative_amount`, `amount <= debt` | WALLET-ASSUMED | numeric guards over spender arg |
| finance/LiquidatableLoan | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/PayrollStream** | `authorized` | SCRIPT-ENFORCED | `checkSig(employee)` on `release` |
| finance/PayrollStream | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/PayrollStream | `after(last_paid + period)` (release time gate) / `temporal_guard` | **SCRIPT-ENFORCED (time_gate)** | `require(tx.time >= prev_states[0].last_paid + prev_states[0].period)` → `OpCheckLockTimeVerify` on the committed window SUM (same `push;OpCheckLockTimeVerify` shape proven in `time_gate_engine.rs`); defeats the final-sequence bypass on a non-final input, with the SEPARATE finalization rule barring inclusion until `block_daa_score > last_paid + period`. **CLTV half proven in isolation; composed `PayrollStream.sil` compiles (silverc exit 0); finalization half not unit-tested; composed on-engine spend pending.** The caller-asserted `now_bucket >= last_paid + period` compare is RETAINED (it anchors the successor `last_paid`; that anchor advance stays WALLET-ASSUMED). Domain-match (L1) + future-window (L3) preconditions apply |
| finance/PayrollStream | `spending_cap` / `non_negative_amount` (`amount <= limit`, `<= balance`) | WALLET-ASSUMED | numeric guards over spender arg |
| finance/PayrollStream | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **finance/reit/DigitalReitToken** | `authorized` | SCRIPT-ENFORCED | `checkSig(trustee)` on `distribute` |
| finance/reit/DigitalReitToken | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/reit/DigitalReitToken | period advance (`period: period + 1`) | SCRIPT-ENFORCED | structural successor predicate |
| finance/reit/DigitalReitToken | `next_declared >= 0` | WALLET-ASSUMED | numeric guard over spender arg; not bound to coin |
| finance/reit/DigitalReitToken | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **finance/reit/DigitalReitSplitter** | `authorized` | SCRIPT-ENFORCED | `checkSig(trustee)` on `payout` |
| finance/reit/DigitalReitSplitter | parent-covenant binding | SCRIPT-ENFORCED | `require(parent_kov_id == OpInputCovenantId(0))` (assumes parent is input 0) |
| finance/reit/DigitalReitSplitter | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/reit/DigitalReitSplitter | monotone period (`for_period == paid_period + 1`) | SCRIPT-ENFORCED | structural successor predicate |
| finance/reit/DigitalReitSplitter | `senior_bps <= 10000`, `amount >= 0` | WALLET-ASSUMED | numeric guards; `amount` not bound to coin |
| **finance/RoyaltySplit** | `authorized` | SCRIPT-ENFORCED | `checkSig(distributor)` on `distribute` |
| finance/RoyaltySplit | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/RoyaltySplit | `conservation_split` (`income-(a+b+c)`, `+a/+b/+c`) | MODEL-ONLY | structural model-field cancellation; no coin bound |
| finance/RoyaltySplit | `a,b,c >= 0`, `a+b+c <= income` | WALLET-ASSUMED | numeric guards over spender args |
| finance/RoyaltySplit | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/SealedBidAuction** | key authorisation | SCRIPT-ENFORCED | `reveal`→`checkSig(high_bidder)`, `close`→`checkSig(seller)` |
| finance/SealedBidAuction | commitment open | SCRIPT-ENFORCED | `require(blake2b(preimage) == bid_commit)` |
| finance/SealedBidAuction | monotone high bid | SCRIPT-ENFORCED | `require(bid > high_bid)` on committed state |
| finance/SealedBidAuction | one-shot close | SCRIPT-ENFORCED | `require(closed == 0)`; `close` sets `closed: 1` |
| finance/SealedBidAuction | covenant continuity / `authorized` / `no_undeclared_state` | SCRIPT-ENFORCED / MODEL-ONLY | `binding = cov` + committed-key auth; lifecycle is model-only |
| **finance/StreamingVesting** | `authorized` | SCRIPT-ENFORCED | `checkSig(recipient)` on `withdraw` |
| finance/StreamingVesting | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/StreamingVesting | `bounded_supply` (`supply + amount <= total`) / `non_negative_amount` | WALLET-ASSUMED | numeric guards over spender arg; not bound to coin |
| finance/StreamingVesting | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **finance/Subscription** | `authorized` | SCRIPT-ENFORCED | `checkSig(provider)` on `charge` |
| finance/Subscription | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/Subscription | `after(last_charged + period)` (charge time gate) / `temporal_guard` | **SCRIPT-ENFORCED (time_gate)** | `require(tx.time >= prev_states[0].last_charged + prev_states[0].period)` → `OpCheckLockTimeVerify` on the committed window SUM (same `push;OpCheckLockTimeVerify` shape proven in `time_gate_engine.rs`); defeats the final-sequence bypass on a non-final input, with the SEPARATE finalization rule barring inclusion until `block_daa_score > last_charged + period`. **CLTV half proven in isolation; composed `Subscription.sil` compiles (silverc exit 0); finalization half not unit-tested; composed on-engine spend pending.** The caller-asserted `now_bucket >= last_charged + period` compare is RETAINED (it anchors the successor `last_charged`; that anchor advance stays WALLET-ASSUMED). Domain-match (L1) + future-window (L3) preconditions apply |
| finance/Subscription | `charge` payout (`pays(1, provider, amount_per_period)`) — output value + payee | **SCRIPT-ENFORCED at emit + silverc exit 0; composed on-engine spend PENDING (KI-3)** | `require(tx.outputs[1].value == prev_states[0].amount_per_period)` (`OpTxOutputAmount`) + `require(tx.outputs[1].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_states[0].provider)))` (`OpTxOutputSpk`); consensus binds output[1] to pay the committed per-period fee to the committed `provider` on every charge. `amount_per_period` is an `int` (the type checker forbids the `>= 0` / `<= balance` / `balance - amount_per_period` arithmetic on `coin`), and it was NEITHER retyped NOR renamed: it qualifies as a bound amount via the DRAWDOWN link — the same entrypoint's successor sets `balance: balance - amount_per_period` under `requires amount_per_period >= 0;`, which is what proves the paid quantity is the quantity the model gives up. **⚠ NON-TERMINAL `pays` — the catalogue's first (KI-3).** This spend carries BOTH a covenant successor AND a bound payee output; silverc's `to` counts covenant SUCCESSOR outputs, so `to = 1` + a separate payee output compiles (exit 0, `output_binding_engine.rs`), but WHICH output index the successor occupies at RUNTIME is UNVERIFIED on the `v2.0.0` pin (same composed-on-engine-spend bucket as KI-1). If the successor lands on index 1 the two bindings collide and every `charge` is rejected — stuck funds. **Do NOT deploy to a value-bearing UTXO until that spend is proven.** P2PK-Schnorr payee (M4, `provider` is committed as a `pubkey`); binds only output[1], no mass check (L1) — the residual over the fee is spender-routed and the fee is not tied to the deposited coin |
| finance/Subscription | `amount_per_period >= 0`, `<= balance` / `non_negative_amount` | WALLET-ASSUMED | numeric guards over committed field; not bound to coin |
| finance/Subscription | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **finance/TokenAllowance** | key authorisation | SCRIPT-ENFORCED | `approve`→`checkSig(owner)`, `transfer_from`→`checkSig(spender)` |
| finance/TokenAllowance | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/TokenAllowance | allowance/balance caps (`amount <= allowance`, `<= balance`) / `non_negative_amount` | WALLET-ASSUMED | numeric guards over spender arg; not bound to coin |
| finance/TokenAllowance | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **finance/TrancheWaterfall** | `authorized` | SCRIPT-ENFORCED | `checkSig(trustee)` on `distribute` |
| finance/TrancheWaterfall | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/TrancheWaterfall | `conservation_split` (`coupon-(s+m+j)`, `+s/+m/+j`) | MODEL-ONLY | structural model-field cancellation; no coin bound |
| finance/TrancheWaterfall | `s,m,j >= 0`, `s+m+j <= coupon` | WALLET-ASSUMED | numeric guards over spender args |
| finance/TrancheWaterfall | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/InternalTransfer** | `authorized` | SCRIPT-ENFORCED | `checkSig(owner)` on `transfer` |
| finance/InternalTransfer | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/InternalTransfer | `conservation_split` (`from-amount`, `to+amount`) | MODEL-ONLY | structural model-field cancellation; no coin bound |
| finance/InternalTransfer | `non_negative_amount`, `amount <= from_balance` | WALLET-ASSUMED | numeric guards over spender arg |
| finance/InternalTransfer | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **finance/MintableToken** | `authorized` (supply-change authority) | SCRIPT-ENFORCED | `mint`→`checkSig(auth, issuer)` against the committed `issuer` key |
| finance/MintableToken | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/MintableToken | `supply_change = issuer` capability (mint waived from conservation) | CHECKED-MODEL | committed authority `issuer` guaranteed to sign; **checked-model capability, NOT on-chain minted supply** — a UTXO covenant cannot inflate real coin; `supply` is a committed integer |
| finance/MintableToken | `supply += amount`, `amount >= 0` | WALLET-ASSUMED | supply counter + non-negative guard over spender arg; not bound to coin |
| **finance/VestingCliffClawback** | key authorisation | SCRIPT-ENFORCED | `vest`/`withdraw`→`checkSig(recipient)`, `clawback`→`checkSig(grantor)` |
| finance/VestingCliffClawback | one-shot vest / gate on `vested` | SCRIPT-ENFORCED | `require(vested == 0)` on `vest`/`clawback`, `== 1` on `withdraw` |
| finance/VestingCliffClawback | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| finance/VestingCliffClawback | `after(cliff)` (vest time gate) / `temporal_guard` | **SCRIPT-ENFORCED (time_gate)** | `require(tx.time >= prev_states[0].cliff)` → `OpCheckLockTimeVerify` (defeats the final-sequence bypass on a non-final input); the SEPARATE finalization rule bars inclusion until `block_daa_score > cliff`. **CLTV half proven in isolation (`time_gate_engine.rs`); composed `VestingCliffClawback.sil` compiles (silverc exit 0); finalization half not unit-tested; composed on-engine spend pending.** Domain-match (L1) + future-`cliff` (L3) ceremony preconditions apply |
| finance/VestingCliffClawback | `bounded_supply` / `non_negative_amount` | WALLET-ASSUMED | numeric guards over spender arg; not bound to coin |
| finance/VestingCliffClawback | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **governance/SocialRecovery** | `multisig_threshold` (2-of-3 guardians) | SCRIPT-ENFORCED | disjunction of two-committed-key `checkSig` arms on both paths |
| governance/SocialRecovery | recovery state gate | SCRIPT-ENFORCED | `require(recovering == 0)` on propose, `== 1` on finalize |
| governance/SocialRecovery | covenant continuity / `authorized` | SCRIPT-ENFORCED | `binding = cov` + committed-key auth |
| governance/SocialRecovery | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **governance/MultisigTreasury** | `multisig_threshold` (2-of-2) | SCRIPT-ENFORCED | `checkSig(signer_a)` AND `checkSig(signer_b)` |
| governance/MultisigTreasury | covenant continuity / `authorized` | SCRIPT-ENFORCED | `binding = cov` + committed-key auth |
| governance/MultisigTreasury | `amount <= balance` / `non_negative_amount` | WALLET-ASSUMED | numeric guards over spender arg; not bound to coin |
| governance/MultisigTreasury | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **state/CsciInstrument** | `authorized` | SCRIPT-ENFORCED | `checkSig(owner)` on `settle` |
| state/CsciInstrument | proof-covenant binding | SCRIPT-ENFORCED | `require(proof_cov_id == OpInputCovenantId(0))` (assumes proof is input 0) |
| state/CsciInstrument | `monotonic_seq` (`seq: seq + 1`) | SCRIPT-ENFORCED | structural successor predicate |
| state/CsciInstrument | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| state/CsciInstrument | STARK validity of the settled state | ENGINE-ASSUMED | via separate tag-`0x21` anchor script, not the covenant |
| state/CsciInstrument | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |
| **vprog/BatchRollup** | `authorized` | SCRIPT-ENFORCED | `checkSig(operator)` on `settle` |
| vprog/BatchRollup | proof-covenant binding | SCRIPT-ENFORCED | `require(proof_cov_id == OpInputCovenantId(0))` (assumes proof is input 0) |
| vprog/BatchRollup | `batch_count >= 1`, `seq: seq + batch_count` | SCRIPT-ENFORCED | numeric guard + structural successor predicate |
| vprog/BatchRollup | STARK validity of the batch | ENGINE-ASSUMED | separate tag-`0x21` anchor; guest `predicate()` is a `true` TODO stub |
| vprog/BatchRollup | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/ZkExecutionRollup** | `authorized` / proof binding / `batch_count`+`seq` | SCRIPT-ENFORCED | as BatchRollup (`checkSig(operator)`, `OpInputCovenantId(0)`, `seq: seq + batch_count`) |
| vprog/ZkExecutionRollup | STARK validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/ZkExecutionRollup | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/ComplianceCredential** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(owner)`, `OpInputCovenantId(0)`, `seq: seq + 1` |
| vprog/ComplianceCredential | verdict/credential validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/ComplianceCredential | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/ConfidentialTransfer** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(owner)`, `OpInputCovenantId(0)`, `seq: seq + 1` |
| vprog/ConfidentialTransfer | balance/commitment validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/ConfidentialTransfer | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/MerkleProofOfSolvency** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(attestor)`, `OpInputCovenantId(0)`, `seq: seq + 1`, `epoch: epoch + 1` |
| vprog/MerkleProofOfSolvency | solvency (`solvent`, roots) validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/MerkleProofOfSolvency | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/ProofOfReserves** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(attestor)`, `OpInputCovenantId(0)`, `seq: seq + 1`, `epoch: epoch + 1` |
| vprog/ProofOfReserves | reserves/solvency validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/ProofOfReserves | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/PrivateOrderMatch** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(operator)`, `OpInputCovenantId(0)`, `seq: seq + 1` |
| vprog/PrivateOrderMatch | matching (price-time-priority) validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/PrivateOrderMatch | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/PrivateVickreyAuction** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(auctioneer)`, `OpInputCovenantId(0)`, `seq: seq + 1` |
| vprog/PrivateVickreyAuction | winner/clearing-price validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/PrivateVickreyAuction | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/PrivateVoting** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(owner)`, `OpInputCovenantId(0)`, `seq: seq + 1` |
| vprog/PrivateVoting | tally validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/PrivateVoting | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **vprog/ZkAllowlistTransfer** | `authorized` / proof binding / `monotonic_seq` | SCRIPT-ENFORCED | `checkSig(owner)`, `OpInputCovenantId(0)`, `seq: seq + 1` |
| vprog/ZkAllowlistTransfer | allowlist-membership / nullifier validity | ENGINE-ASSUMED | separate anchor; guest `predicate()` is a `true` TODO stub |
| vprog/ZkAllowlistTransfer | `no_undeclared_state` | MODEL-ONLY | lifecycle wellformedness |
| **attestation/EvidenceLineage** | `authorized` | SCRIPT-ENFORCED | `checkSig(issuer)` on `attest` |
| attestation/EvidenceLineage | covenant continuity | SCRIPT-ENFORCED | `binding = cov` |
| attestation/EvidenceLineage | seq advance (`seq: seq + 1`) | SCRIPT-ENFORCED | structural successor predicate |
| attestation/EvidenceLineage | bucket window (`next_t_bucket >= t_bucket`, `<= t_bucket + window`) | WALLET-ASSUMED | `next_t_bucket` is a spender arg; window is committed |
| attestation/EvidenceLineage | `next_class >= 0` | WALLET-ASSUMED | numeric guard over spender arg |
| attestation/EvidenceLineage | `value_conserved` / `no_undeclared_state` | MODEL-ONLY | per-field shape + lifecycle |

## Time-gate migration fan-out (B1)

`custody/TimeVault` was the FIRST pattern migrated from the caller-asserted
`now_bucket >= <committed>` gate to the consensus `after(...)` clause
(SCRIPT-ENFORCED). This fan-out slice extended the `after(...)` surface to a
committed-SUM deadline (`after(a + b)`, the two-atom window form) and migrated the
bindable time + payout rows across the catalogue. **Done (SCRIPT-ENFORCED, see the
table rows above):**

- `finance/Escrow` — `after(deadline)` refund gate **and** the `refund` payout
  (`pays(0, buyer, amount)`), joining the already-bound `release` payout;
- `finance/Htlc` — `after(deadline)` refund gate;
- `finance/VestingCliffClawback` — `after(cliff)` vest gate;
- `finance/ArbiterEscrow` — `release` payout (`pays(0, seller, amount)`);
- `finance/Subscription` — `after(last_charged + period)` charge gate (window sum),
  **and (later slice) the `charge` payout `pays(1, provider, amount_per_period)` —
  the catalogue's first NON-TERMINAL `pays`, licensed by the DRAWDOWN link rather
  than by a retype or a rename, and carrying the KI-3 pending caveat**;
- `finance/PayrollStream` — `after(last_paid + period)` release gate (window sum);
- `custody/DeadMansSwitch` — `after(last_active + timeout)` claim gate (window sum).

For the three window-sum patterns the caller-asserted `now_bucket >= <sum>` compare
is RETAINED alongside the new CLTV clause: `now_bucket` still anchors the successor
time field (`last_charged`/`last_paid`/`last_active`), and that anchor advance stays
WALLET-ASSUMED — the consensus CLTV gate bounds the spending tx's lock time, but
does not itself constrain the recorded anchor.

**No new engine test was added, by design.** The window-sum threshold is the same
`push;OpCheckLockTimeVerify` shape already proven accept/early-reject/bypass-reject
in `time_gate_engine.rs` (the sum is computed on-stack from two committed atoms
before the identical CLTV check), and the multi-output payouts are the same
`OpTxOutputAmount`/`OpTxOutputSpk` pair already proven in `output_binding_engine.rs`.
The real per-pattern gate is `portrait engrave` succeeding (silverc exit 0) on every
edited `.portrait`, which it does.

**Deferred (documented, NOT migrated — do not force):**

- `attestation/EvidenceLineage` (`next_t_bucket` bucket window) — the window
  UPPER bound (`next_t_bucket <= t_bucket + window`) is not CLTV-expressible
  (`OpCheckLockTimeVerify` is a lower-bound-only monotone gate).
- Spender-arg-amount / no-committed-external-payee payouts —
  `finance/PayrollStream`, `finance/StreamingVesting`, `custody/SpendingLimitVault`,
  `finance/DepositInsurancePool`, `finance/CollateralVault` (the released amount is
  a spender arg, so there is no committed value to bind).
- Int-balance-payee patterns — `finance/RoyaltySplit`, `finance/TrancheWaterfall`,
  `finance/InternalSplit`, `finance/InternalTransfer`, `finance/KycGatedTransfer`,
  `finance/TokenAllowance` (payees are internal int balances; binding a real output
  needs committed payee pubkeys or committed `byte[32]` script hashes + a
  redesign). `pays` now dispatches the payee's spk form from its DECLARED TYPE
  (`pubkey` → `ScriptPubKeyP2PK`, `byte[32]` → `ScriptPubKeyP2SH`), which removes
  the "P2PK-only" footgun — an Escrow/ArbiterEscrow instantiated for a P2SH /
  multisig seller previously had a permanently DEAD `release` path and no way to
  express the working one. It unblocks NO pattern in this deferred list: these
  patterns have no committed external payee at all, and the spender-arg-amount
  group above is blocked on the AMOUNT, not the payee.

## How to extend this table

When a new pattern lands in `library/`, read its emitted `.sil` and add one row
per declared invariant/guarantee, classified by the buckets above. Do not copy a
claim from the `.portrait` prose or the pattern name — classify only what the
`.sil` (and, for vProg, the separate anchor + guest) actually does.
