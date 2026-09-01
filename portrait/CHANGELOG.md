# Changelog

All notable changes to Portrait are recorded here.

> **Maturity: pre-production, unaudited, testnet-only.** No external security
> audit or external review has taken place. Nothing is on mainnet.

## [Unreleased]

### Added
- **`pays(...)` amounts can be licensed by a DRAWDOWN link, not just a type or a
  name** — a `pays(index, payee, amount)` operand was previously bindable only if
  it was value-bearing (typed `coin`, or named exactly `balance`/`amount`/
  `supply`). `portrait-sema` now accepts a SECOND, structural path: a committed
  `int` field qualifies when the SAME entrypoint's object return **decreases** a
  value-bearing field by a term carrying that field as one of its `+`-atoms, with
  every `+`-atom of the term established non-negative there (the same A6-sign
  guard `value_conserved` uses). That interlock is load-bearing — an unguarded
  term can invert the subtraction, and a "drawdown" by a negative term is a
  top-up. This is a pure WIDENING: nothing previously accepted is now rejected,
  and a committed `int` with no drawdown link stays rejected. Rationale: the two
  alternatives were both bad. Retyping to `coin` cannot express a drawdown at all
  (the type checker forbids arithmetic on `coin`), and renaming the field to a
  value-bearing NAME would buy the guarantee off a name — the naming-as-enforcement
  class the A2/A5 capability work retired.
- **`finance/Subscription` binds its `charge` payout** —
  `pays(1, provider, amount_per_period)`, licensed by the drawdown link above
  (`balance: balance - amount_per_period` under `requires amount_per_period >= 0;`),
  so consensus binds output[1] to pay the committed per-period fee to the committed
  provider. **⚠ This is the catalogue's FIRST NON-TERMINAL `pays`** — one spend
  carrying BOTH a covenant successor and a bound payee output. silverc's `to`
  counts covenant SUCCESSOR outputs, so `to = 1` plus a separate payee output is
  well-formed at compile time (silverc exit 0, asserted in
  `output_binding_engine.rs`), **but which output index the successor occupies at
  RUNTIME is UNVERIFIED on the `v2.0.0` pin** — filed as **KI-3** in
  `KNOWN-ISSUES.md` with a do-not-deploy directive. The enforcement row reads
  *SCRIPT-ENFORCED at emit + silverc exit 0; composed on-engine spend PENDING*,
  never plain SCRIPT-ENFORCED.
- **`pays(...)` supports a non-P2PK payee via type-directed dispatch** — the
  `pays(index, payee, amount)` surface is UNCHANGED; the emitter now picks the spk
  builtin from the payee's already-declared type: `pubkey` → `ScriptPubKeyP2PK`
  (byte-identical to before — zero churn on the three shipped clauses),
  `byte[32]` → `ScriptPubKeyP2SH`, anything else → an emit error naming the payee
  rather than a guessed lowering. No `pays_p2sh` variant, no raw spk bytes in
  committed state. **Honest framing: this unblocks NO currently-deferred catalogue
  pattern** (the int-balance-payee patterns have no committed external payee at
  all; the spender-arg-amount group is blocked on the AMOUNT). What it removes is a
  live FOOTGUN (M4): an `Escrow`/`ArbiterEscrow` instantiated for a P2SH/multisig
  seller previously had a permanently DEAD `release` path with no way to express
  the working one. It is also the prerequisite for multisig-payee patterns.
  Evidence: the P2SH binding is proven accept (committed script hash) / reject
  (different script hash) on the pinned engine, and
  `scriptpubkeyp2sh_lowering_matches_our_reconstruction_golden` pins our
  reconstruction against silverc's REAL compiled bytes (not against itself).
- **Explicit supply-change capability (A2-full)** — a supply change (mint/burn) is
  now an EXPLICIT, SIGNED, CHECKED capability declared via a new optional covenant
  key: `#[covenant(mode = transition, supply_change = <field>)]`. The value names a
  committed authority. Declaring it (a) WAIVES the entry from value-conservation
  checking (`value_conserved` / `conservation_split`) — a supply change legitimately
  does not conserve — and (b) is CHECKED by `portrait-sema` UNCONDITIONALLY: the
  named authority must be a COMMITTED key (role param / state field), GUARANTEED to
  sign on every satisfying path (a sound, commutative per-key predicate — `And`
  needs either arm to force the key, `Or` needs both, so the authority is never
  satisfiable through a `||` branch or a negated arm; the verdict is arm-order
  independent), AND release NO coin (no `pays(...)` clause, not a terminal spend) —
  so a supply change provably adjusts committed supply only, which soundly lets
  `payout_bound` exclude it. A non-committed authority, one only in a disjunctive
  arm, or one attaching a payout, is REJECTED with a diagnostic naming the
  authority. Lens (`portrait-lens`) agrees on which entries are conservation-exempt
  (the annotation, never the name), with `portrait-sema` remaining the single source
  of the authority-signs check. New demonstrator `finance/MintableToken` (a
  single-`issuer` mint; silverc exit 0). **Honest boundary: `supply_change` is a
  CHECKED-MODEL
  capability — the named authority signs and the model is waived from conservation —
  NOT an on-chain minted-supply guarantee. A UTXO covenant cannot inflate real coin;
  the field is the covenant's own committed integer, not a mint of L1 KAS.**
- **Terminal spend (B3)** — a lifecycle-ending transition can now RELEASE the coin
  to a committed payee (bound via `pays(...)`) and CONSUME the UTXO, instead of
  trapping the value in a dead successor covenant. A transition named by a
  lifecycle edge marked `terminal` (`... via role.entry terminal;`) is emitted as a
  `binding = auth`, `mode = verification` function with **no successor return**: the
  spend is authorised by the `checkSig` the body checks (no covenant-id to inherit),
  state is read via the singular `prev_state.<field>` accessor, and the `pays(...)`
  / `after(...)` guards lower to the same output-introspection / CLTV opcodes as the
  non-terminal path (the non-terminal emit path is byte-identical). `portrait-sema`
  now (1) recognizes a terminal transition as a settling transition for
  `payout_bound`, so a terminal spend **must** bind its payout via `pays(...)`
  (non-deletable), and (2) fail-loud rejects a terminal transition that still
  declares a `return` successor. A terminal transition carrying a vProg is refused
  at emit (no successor to carry the proof-covenant-id binding). `finance/Escrow`
  now demonstrates this: `release`/`refund` are terminal spends (the `settled`
  one-shot flag is dropped — UTXO consumption already gives release-XOR-refund).
  Evidence (honest scope): the isolated pays-bound terminal spend producing ZERO
  covenant-successor outputs is proven accept (committed payee) / reject (wrong
  payee) on the pinned engine (`v2.0.0` = `90dbf07`,
  `output_binding_engine.rs`), and the composed terminal `Escrow.sil` compiles
  under silverc (exit 0); the composed on-engine terminal spend remains pending
  (same upstream covenant-ABI pin bucket as B2/B1).

### Changed
- **Parser diagnostics report `line:col` with the offending source line and a
  caret**, replacing the un-actionable `at byte <offset>` rendering. All 37 parser
  rejection sites (16 calling `error_at` directly, 21 via `error`) funnel through
  that one renderer, so the upgrade lands on every one of them at once:

  ```
  error: expected identifier at line 4:15
        param int 7bad;
                  ^
  ```

  The parallel span-carrying refactor of the SEMA diagnostics is DEFERRED on
  purpose and recorded in `docs/ARCHITECTURE.md` §9: it would need byte spans
  threaded through `Role`/`Entry`/`Param`/`Field`/`Stmt` plus every `Diagnostic`
  construction site, for zero correctness value — and 51 of the 55 sema
  diagnostics already carry a symbolic `role.entry` locator; the other 4 are
  app-/lifecycle-level rules with no single entrypoint to name.
- **BREAKING (authors): genesis state binds constructor params BY NAME, not by
  position.** The Engraver previously initialised `model.state[i]` from
  `model.params[i]` and, when the param list ran short, from the literal `0`.
  Both halves were silent genesis corruption: reordering either list rebound
  every field to a different param, and a state field past the end of the param
  list was born zero with no diagnostic. `examples/engraver-demo/PausableToken`
  carried the bug in the wild — its `paused` field was silently `0` at genesis.
  Emission now matches each state field to the constructor param of the SAME
  NAME; a field with no same-named param, or one whose same-named param has a
  different type, is a fail-loud emit error naming the field and prescribing the
  `param <ty> <name>;` declaration. It is NEVER defaulted to 0. Params BEYOND the
  state set stay legal (policy params such as `issuer`/`window` on
  `attestation/EvidenceLineage` need no state field). **Author action:** rename
  any constructor param that does not already match its state field. Six example
  sources and the `portrait new --template counter` scaffold were updated.

  The MATCHING is by name; the EMITTED constructor identifier is not. A state
  field and a constructor param cannot share an identifier in the emitted
  contract: silverscript's public `ContractAst::resolve_contract_state_values` —
  the API a deployer uses to compute an instance's concrete genesis state, and
  the one the upstream `cli-debugger` calls — hard-errors `duplicate contract
  field name: <f>` on that collision. `silverc`'s own compile path never calls
  it, which is why the collision compiled clean and went unnoticed. Emission now
  follows upstream's convention (`contract ResolveState(int initAmount) { int
  amount = initAmount; }`) and writes the genesis param as `init_<field>`, with
  deterministic underscore-widening (`init__<field>`, …) if an author already
  holds that name. **Every emitted `.sil` and `_ctor.json` changed shape.**
  Measured against the real upstream API: at the previous release **39 of 45**
  emitted covenants were UNRESOLVABLE; all **45 of 45** now resolve.
- **`portrait check` now covers the genesis-binding rule.** It was enforced only
  at `engrave`, so a source with an unbound state field passed the documented
  first gate — the one the Hallmark cites — and failed only later. `check` now
  runs `portrait_emit::validate_genesis_binding` as an additional pass;
  `portrait_sema::check`'s own contract is unchanged (it cannot carry the rule —
  many parse/check-only fixtures declare state fields with no params at all).
- **`<Name>_ctor.json` is now declared to be a placeholder.** The emitter fills
  every constructor argument with a type-shaped ZERO (`0` for int/coin, 32 zero
  bytes for a pubkey, 64 for a sig) and there is no flag for supplying real
  values — but nothing said so, so `portrait new --template counter && portrait
  ship` ran clean and printed a KovId that identifies the ZERO-GENESIS instance.
  An all-zero `owner` pubkey is a key nobody holds: a covenant committed to one
  has a `checkSig` that can never be satisfied, i.e. permanently locked funds.
  Now: `engrave`/`build`/`ship` print a loud warning naming the artifact and the
  hazard; the Hallmark records `"genesis": "placeholder-zero"` with a qualifying
- **BREAKING (pre-1.0, Rust API):** `Hallmark` gains a public `genesis:
  Option<String>` field, so struct-literal construction of `Hallmark` no longer
  compiles; use the constructor or add the field. `portrait-emit` and
  `portrait-sema` also gain additive `pub` items (`placeholder_ctor_warning`,
  `validate_genesis_binding`, `warnings`). Nothing was removed.
  note; the `ship` summary marks the KovId line as a PLACEHOLDER instance; and
  `docs/GETTING-STARTED.md` carries the caveat beside the artifact listing. The
  genesis diagnostic was also softened to claim only what it enforces — the
  compiler never silently binds a *source* field to 0; the ctor VALUES remain
  placeholders until the author supplies them.
- **Reserved emitter identifiers and duplicate params are now rejected.** A role
  param or state field named `max_ins`/`max_outs` was emitted alongside the bound
  the Engraver injects under the same name; silverc accepted the duplicate and
  the USER's param won, making the covenant's own output-count bound
  deployer-controlled. A duplicate `param` name was also accepted, with the
  by-name genesis lookup silently taking the first — so a `pubkey balance`
  shadowing an `int balance` produced a misleading type-mismatch diagnostic.
  Both are now named sema rejections (and emit asserts param uniqueness).
- **A non-terminal `mode = verification` entrypoint no longer drops its guards.**
  Those bodies are not lowered, so `requires checkSig(auth, owner)` was SILENTLY
  DROPPED and `portrait check` reported ok — a covenant that looks gated and
  enforces nothing, failing closed only by accident. This contradicted `emit`'s
  own fail-loud contract; it is now a loud emit error naming the dropped guard.
- **`value_conserved` / `conservation_split` now check the SIGN of the adjustment
  term (A6-sign — distinct from A6 `payout_bound`; the two review items share a
  number, not a rule).** The accepted shape `f: f ± e` was sign-blind: a NEGATIVE `e` in
  `f - e` INCREASES the field (model money-printing through the accepted shape),
  and in `f + e` DESTROYS value — which under `conservation_split` is a REVERSE
  transfer that drains the destination leg, invisible to structural cancellation
  because the same term appears on both legs whatever its sign. The adjustment
  term is now decomposed into its top-level `+`-atoms, and every atom must be a
  non-negative int literal or a name the SAME entrypoint guards with
  `requires <name> >= 0;` (or `> -1`, or the mirrored `0 <= <name>`). A merely
  COMMITTED field does not qualify — genesis can commit a negative. It is
  unconditional **within C1's value-bearing field set**, which is a name/type
  rule, not an inference: `coin`, or a name that is exactly `balance`/`amount`/
  `supply` (plus any `*balance` suffix for the split). A value field called
  `funds` or `principal` is outside the check entirely — `portrait check` now
  WARNS when a role declares one of these invariants and NO field on it is
  value-bearing, so a wholly vacuous declaration can no longer report ok in
  silence. The explicit `supply_change` capability does NOT waive this: it
  authorises a supply change, not a sign inversion, so it waives the conservation
  SHAPE only. Every shipped catalogue pattern already carried the guard, so the
  catalogue's accept goldens stay green — the ratchet closed a real hole with
  zero true defects. Honest limit unchanged: only the term's LOWER bound is
  established; a ceiling is still the job of the opt-in `bounded_supply` /
  `spending_cap` refinements, and none of this is an SMT proof.
- **Conservation exemption retired the `mint*`/`burn*` NAME heuristic (A2-full).**
  Previously an entrypoint whose name began with `mint`/`burn` was silently exempt
  from value-conservation checking — a name bought a real check-waiver. That
  heuristic (`is_mint_or_burn`) is DELETED. The conservation waiver is now earned
  ONLY by the explicit, checked `supply_change` capability (see Added). Consequence:
  an unannotated `mint`-named entry with a non-conserving return under
  `value_conserved` is now conservation-CHECKED and rejected, and Lens emits its
  conservation VC. `payout_bound`'s settling-transition recognizer likewise now
  gates on `supply_change` (a supply change releases no coin) rather than the name.
- vProg guest emitter (`portrait-atelier`) — the generated guest's developer
  `predicate()` now **REFUSES TO BUILD by default** (C2, finding 1). Previously every
  emitted guest carried `fn predicate(...) -> bool { /* TODO */ true }` — a stub that
  returns `true`, so a developer who never authored the predicate could ship a guest
  that reads as a working ZK covenant but proves nothing. The default body is now a
  `compile_error!`, so `cargo build` of an unauthored guest fails. `portrait
  atelier-build --allow-unimplemented-vprog` opts into the old true-returning
  placeholder, but only under a loud `// WARNING: UNIMPLEMENTED vProg predicate —
  returns true unconditionally; proves NOTHING. Placeholder only.` banner. A shipped
  guest now either carries real logic or a conscious, loudly-marked placeholder.
- vProg proof-covenant-id binding index is now **parametric** (C2, finding 2). The
  emitter-injected `require(proof_cov_id == OpInputCovenantId(<idx>))` no longer
  hardcodes input 0 — the index is a named parameter (`DEFAULT_PROOF_COV_INPUT_INDEX`,
  default 0). The default still assumes the covenant UTXO is input 0; a spend where it
  is not can emit the correct index rather than binding the wrong input. Assumption
  documented at the emission site and in `library/ENFORCEMENT.md`.

### Added
- Formula-bearing structural invariants (A4-full + A6) — two compile-time
  obligations that make an EXISTING script-enforced clause mandatory and
  non-deletable, verified STRUCTURALLY (the checker confirms the entrypoint
  *carries the matching consensus gate the emitter lowers* — neither is an SMT
  proof). (1) **A4-full**: `invariant <name>: <entry> => after(<deadline>);` binds
  a named transition to carry an `after(<deadline>)` CLTV clause with exactly that
  deadline (reusing the existing `AfterDeadline` grammar; a `Sum` window matches
  either operand order), strictly stronger than the existence-only `temporal_guard`
  — deleting the `after(...)` clause is now a compile error. The `entry` name binds
  by entrypoint NAME across roles: EVERY role declaring that entrypoint must carry
  the matching clause. Honest scope: certifies the entrypoint CARRIES the matching
  consensus gate (→ `OpCheckLockTimeVerify`), NOT an SMT-discharged temporal
  obligation. (2) **A6** `invariant payout_bound;`: every RECOGNIZED settling
  transition (a non-mint/burn `mode = transition` path) must carry a `pays(...)`
  clause, making the payout binding mandatory — deleting the `pays(...)` on a
  settling path is a compile error. `settles` is a RECOGNIZER, not a complete
  settlement detector: it matches exactly three one-shot-flag flip shapes —
  int-literal (`require f == 0` + `f: <nonzero>`), computed int
  (`require f == 0` + `f: f + <nonzero>`), and bool (`require f == false` +
  `f: true`); a settlement written outside these is NOT recognized. FAIL-LOUD ON
  VACUITY: a `payout_bound` recognizing ZERO settlements is rejected, and `explain`
  prints the recognized-settlement count as a coverage signal. Honest scope:
  EXISTENCE-ONLY — it requires *a* `pays(...)` on the settling path
  (→ `OpTxOutputAmount`/`OpTxOutputSpk`), NOT that it binds THIS settlement's own
  coin/payee (payee/amount validity checked separately), NOT a value-conservation /
  KIP-9 mass proof — the L1 surplus caveat still applies. Demonstrated on
  `finance/htlc` (`refund_after_deadline`) and `finance/escrow` (`payout_bound`);
  both invariants leave the emitted `.sil` unchanged. See `library/ENFORCEMENT.md`.
- `after(<committed sum>)` covenant clause (B1, D1) — the `after(...)` time gate now
  accepts a two-atom window sum `after(a + b)` (e.g. `after(last_charged + period)`)
  in addition to the single-field `after(field)` form. Both operands must be committed
  int-typed time atoms; no arbitrary arithmetic. It lowers to
  `require(tx.time >= prev_states[0].a + prev_states[0].b)` — the SAME
  `push;OpCheckLockTimeVerify` shape the single-field form uses, with the threshold
  computed on-stack from the two committed atoms before the identical CLTV check, so
  **no new engine test is needed**: the opcode semantics are already proven in
  `portrait-emit/tests/time_gate_engine.rs`. `portrait-sema` validates BOTH operands
  are committed + time-named. Also added a `check_pays` diagnostic (D2) rejecting two
  `pays` clauses at the SAME output index in one entrypoint.
- Time-gate + payout fan-out migration — migrated the catalogue's bindable time and
  payout rows to the real `after(...)`/`pays(...)` consensus enforcement (each
  re-engraved; silverc exit 0): `finance/Escrow` (`after(deadline)` + `refund`
  payout `pays(0, buyer, amount)`), `finance/Htlc` (`after(deadline)`),
  `finance/VestingCliffClawback` (`after(cliff)`), `finance/ArbiterEscrow`
  (`pays(0, seller, amount)`), and the window-sum gates `finance/Subscription`
  (`after(last_charged + period)`), `finance/PayrollStream`
  (`after(last_paid + period)`), `custody/DeadMansSwitch`
  (`after(last_active + timeout)`). The multi-output payouts reuse the
  `OpTxOutputAmount`/`OpTxOutputSpk` pair already proven in
  `portrait-emit/tests/output_binding_engine.rs`. For the window-sum patterns the
  caller-asserted `now_bucket >= <sum>` compare is retained (it anchors the successor
  time field; that anchor advance stays wallet-assumed). Deferred (documented in
  `library/ENFORCEMENT.md`, not forced): the `finance/Subscription` payout binding
  (coin retype would drop the balance-drawdown model), `attestation/EvidenceLineage`
  (window upper bound not CLTV-expressible), and the spender-arg-amount /
  int-balance-payee payouts. Same maturity caveats carry (opcode-proven-in-isolation /
  composed-compiles / on-engine-spend-pending; P2PK-Schnorr payee only; binds only
  output[k]; DAA-domain + future-deadline ceremony preconditions).
- `after(deadline)` covenant clause (B1) — a `pays`-parallel surface that emits a
  **consensus-enforced time gate**. It lowers to
  `require(tx.time >= <committed deadline>)` → the engine's `OpCheckLockTimeVerify`
  (a bare `tx.locktime` compare is bypassable and is never emitted). The
  no-early-spend guarantee is **two** consensus rules and the emitted opcode is only
  half: (1) CLTV enforces that the tx **commits** a `lock_time >=` the committed
  deadline on a **non-final** input (defeating the final-sequence bypass), reading
  only the spender-set lock_time field; (2) the SEPARATE consensus finalization rule
  `check_tx_is_finalized` bars inclusion until `block_daa_score >` the deadline —
  the load-bearing "time has passed" half, which lives outside txscript.
  `portrait-sema` validates the deadline is committed, int-typed, and time-named; an
  `after(...)` clause also satisfies `invariant temporal_guard`. The CLTV half
  (opcode accept/early-reject/bypass-reject) is proven against the pinned engine
  (`v2.0.0` = `90dbf07`) in `portrait-emit/tests/time_gate_engine.rs`; the
  finalization half is `pub(crate)` (out of txscript scope) and is logged, not
  unit-tested. `custody/TimeVault` is migrated to `after(...)` and its composed
  `TimeVault.sil` compiles under silverc (exit 0). Caveat: a committed deadline of
  0 / ≤ the instantiation DAA score is no gate (ceremony must commit a future
  deadline). See `library/ENFORCEMENT.md`.
- `library/ENFORCEMENT.md` — a central enforcement matrix that inventories every
  pattern in `library/` and classifies each declared invariant/guarantee, grounded
  in the *emitted* `.sil`, into SCRIPT-ENFORCED / MODEL-ONLY / WALLET-ASSUMED /
  ENGINE-ASSUMED buckets.
- `portrait prove --strict` — opt-in strict mode that exits non-zero when the
  prove run does not meet the strict bar (zero VCs generated, solver missing, any
  UNKNOWN outcome, or any transition entrypoint covered by no VC). Default
  (non-strict) behaviour is unchanged.
- `portrait prove` now prints a coverage matrix (entrypoint × property → proved /
  unknown / exempt / unchecked) above the existing footer.

### Changed
- Honest verdict scope tags: the `prove` `[proved]` headline now carries an inline
  `[MODEL-ONLY; not the emitted .sil]` suffix, and the `validate-translation`
  `CORRESPONDS` headline carries `[STRUCTURAL; not behavioural equivalence]`, so
  neither can be misread as a script audit. The full footers are unchanged.
- Scope-labelled the `value_conserved` and `temporal_guard` invariant
  doc-comments in `portrait-sema` to state their true (model-only / wallet-assumed)
  scope and point at `library/ENFORCEMENT.md`. Invariant tokens and matching logic
  are unchanged.

### Fixed
- Corrected the stale, overclaiming `library/custody/time-vault/README.md`: it
  described nonexistent `schedule`/`settle`/`cancel` entrypoints and a
  consensus-enforced "committed payout" the emitted `TimeVault.sil` does not
  perform. The README now matches the emitted covenant (`release`/`claw`, no
  payout/value/payee constraint; its time gate is now the consensus `after(...)`
  CLTV gate — see the B1 entry under Added).
