# portrait-compose (M1–M5) adversarial sweep — findings

Target: `portrait/portrait/crates/portrait-compose` (FULL crate).
Method: 17 adversarial protocols driving `Score::check` / `lift` / `realize` /
`emit_real_covenants` / `execute`, via scratch tests (since removed; workspace
left green: 67 tests pass, clippy + fmt clean). Date: 2026-06-29.
Pre-production, unaudited, testnet-only.

## Verdict

**No in-scope unsoundness found.** Every attack on the load-bearing seams —
vacuous-accept in `check()`, unfaithful lift that check-accepts, vacuous
not-stranded escape, false-real emit, false-Completion on a deadlock — was either
correctly rejected or fell outside the crate's honestly-stated scope. The
per-increment red-team fixes (non-vacuous escape `reclaim ⊇ escrowed`,
strandable-Value-in-loop, projectability merge) all held.

## Seams probed and held

- **Vacuous-accept in `check()`.** A branch-dependent receive on a non-decider
  (recv on one arm, `end` on the other) is correctly rejected `NotProjectable`.
  A `Value` escrowed-and-abandoned at a recursion loop-back is correctly
  `ResourceStranded`. `Par` resource overlap (Continuation reuse across
  concurrent sides) is rejected `ParResourceOverlap`.
- **Unfaithful lift.** A `choose` with branches led by *different* roles
  (A vs B) does not slip through: it is rejected (the divergent branch leaders
  yield a linearity break on `step`). A mixed-context `choose` whose branches are
  both genuinely led by the same decider lifts to a faithful internal choice with
  the non-decider informed by distinct labels. Trailing steps after an unbounded
  `repeat` are refused (`TrailingStepsAfterUnboundedRepeat`) rather than silently
  dropped.
- **False-real emit.** Labels that are valid idents but adversarial — equal to
  the role/app name (`A`), equal to the generated arg name (`next`) — emit
  covenants that genuinely `portrait_syntax::parse` Ok AND `portrait_sema::check`
  Ok. Pure-receiver roles emit a valid empty-body covenant. No false "real".
- **False-Completion / deadlock.** A statically-rejected broken handoff (a
  non-holder re-carrying a live `Continuation`) drives the executor to
  `Status::Stuck` with a precise diagnostic — not a false `Completed`.

## Honest-scope notes (NOT unsoundness — boundary is documented)

1. **`check_duality` is label-count-only (documented belt-and-suspenders).**
   Hand-built locals where A sends `m` to B while B receives `m` from a
   non-sending C pass the independent duality guard (sends/recvs matched by label
   existence, not by `(sender,receiver)` pairing). This is **unreachable via the
   real pipeline**: `project()` is dual-by-construction, and `lib.rs` explicitly
   states duality "follows by construction from the projection of one well-formed
   global type — we do not re-derive it by exploring the product automaton." A
   caller who feeds the guard externally-authored locals is outside the stated
   contract. Hardening option (defensive, low priority): pair by
   `(sender,receiver,label)`.

2. **`lift` synthesises a receiver for a terminal `Step::Move`.** The surface
   `Step::Move { role, entry }` carries no receiver, so a terminal move's
   counterparty is chosen deterministically as the first declared role `≠` actor
   (`counterparty()`). E.g. flow `[A.s1, B.s2]` lifts to `A→B . B→A`, inventing
   `B→A`. This is documented ("the role the step hands off to before the protocol
   ends") and does not turn an *unsafe* protocol into a safe-looking one — it only
   fixes a partner the surface left unspecified. Faithfulness caveat worth a
   doc-line; not a soundness defect.

3. **`execute()` takes branch[0] only.** A `Completed` from the default schedule
   is a single-path statement, and the module says so loudly (SIMULATION, not a
   liveness verdict; `execute_with_choices` drives other arms). The *soundness*
   verdict for all paths is `Score::check` (path-exhaustive), which rejects the
   bad-branch protocols anyway. No overclaim.

All scope footers (`HONEST_BOUNDARY_FOOTER`, `NOT_STRANDED_BEYOND_T`,
`EXECUTOR_FOOTER`, `happy_path_completion_guaranteed() == false`) are accurate and
non-inflating.
