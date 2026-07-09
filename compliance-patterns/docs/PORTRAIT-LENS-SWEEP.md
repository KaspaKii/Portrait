# portrait-lens (M1–M6) adversarial sweep — full-crate seam audit

Date: 2026-06-29. Engine: `portrait` CLI built from `portrait/portrait`
(`portrait prove`, `portrait validate-translation`). z3: `/opt/homebrew/bin/z3`.
Status: pre-production, unaudited, testnet-only.

## Scope

Real-binary sweep of the FULL `portrait-lens` crate at the seams BETWEEN features,
hunting what the per-increment attacks missed: false-PROVED in any VC class
(conservation / range / refinement / invariant / spend), SAT(T) vacuity bypass,
M4 validation false-CONFIRM, M6 translation false-CORRESPONDS, cross-check
independence overclaim, and honest-scope drift in CLI output. 12 adversarial
covenants (A1–A12) plus 3 library-level translation probes.

## Result

The **`prove` side held on every probe**. No false-PROVED in any VC class, the
SAT(T) vacuity guard caught the contradictory-guard covenant (UNKNOWN, never
PROVED), no false-CONFIRM (every UF-touching witness fell to CANDIDATE,
fail-closed), and the cross-check footer honestly states "same binary, perturbed
config — A3 reduced, not discharged."

**Two genuine soundness defects were found in the M6 STRUCTURAL translation
validator** (`portrait_lens::validate_translation`) — both make it report
`CORRESPONDS` for a `.sil` that drops a guard, contradicting its documented
contract ("catches a dropped guard"). These are reachable only through the public
library API (the CLI subcommand re-emits a faithful `.sil` from the same model, so
it cannot exhibit them), but `validate_translation` is exported and documented as
the model-vs-`.sil` drift check, so a CORRESPONDS for a drifted `.sil` is a real
unsoundness of the check.

### DEFECT 1 (MEDIUM, real) — duplicate-block detector evaded by whitespace

`translation.rs::count_function_blocks` and `extract_function_block` both match the
literal token `function NAME(` (single space). A malicious shadow block spelled
`function  NAME(` (TWO spaces) is counted as zero duplicates AND skipped by the
extractor, which finds the faithful single-space block first. A second
`function  rebalance(` block that drops the overdraw guard and mints `+999` into
`pool_a_balance` is invisible → verdict `CORRESPONDS`. The crate's own
`duplicate_function_block_diverges` test only exercises the single-space spelling,
so this path was never covered.

Evidence (library call):
`validate_translation(InternalSplit, faithful_sil + "\n    function  rebalance(... mint +999, no overdraw guard ...)")` → `Corresponds`.
The single-space spelling of the same shadow correctly `Diverges` (existing test).

### DEFECT 2 (MEDIUM, real) — commented-out `require(...)` parsed as a live guard

`extract_sil_requires` does a raw substring scan for `require(` with brace-balanced
arg capture, with NO comment/lexing awareness. Dropping the live
`require(x + y <= prev_states[0].pool_a_balance);` and leaving a commented
`// require(x + y <= prev_states[0].pool_a_balance);` makes the extractor harvest
the commented text as a present guard whose normalized form equals the model guard
→ verdict `CORRESPONDS` despite the overdraw guard being absent from enforcement
(money-printing overdraw permitted).

Evidence (library call): the commented-guard mutation → `Corresponds`. Note the
robust sub-case: replacing the guard with an unrelated `log_require("...")` line
correctly `Diverges` (the captured arg does not normalize to the model guard), so
the defect is specifically the comment-preserving-the-exact-guard-text case.

Both defects share a root cause: the structural check parses `.sil` by ad-hoc
substring/brace matching rather than a `.sil` tokenizer, so whitespace variation
and comments defeat it. A fix would tokenize/strip comments and normalize the
`function NAME(` match (any inter-token whitespace) before counting/extracting.

### Resolution (2026-06-29) — both defects FIXED

Both MEDIUM defects are now fixed in `translation.rs` (TDD: a failing test was
written for each first):

- DEFECT 1: `count_function_blocks` / `extract_function_block` now go through a
  whitespace-tolerant `find_function_block` (matches `function <ws> NAME <ws> (`
  with an identifier-boundary guard against `function rebalanceX(`), so a
  `function  rebalance(` shadow is counted and rejected as a duplicate. Test:
  `whitespace_shadow_function_block_diverges`.
- DEFECT 2: `validate_translation` now strips `//` and `/* */` comments
  (`strip_sil_comments`) before any structural scan, so a commented-out
  `// require(...)` is no longer harvested as a live guard. Test:
  `commented_out_require_does_not_count_as_a_live_guard`.

The lens suite is green (10/10 translation tests, full crate passes) and the
faithful-covenant CORRESPONDS / dropped-guard DIVERGES headline invariants hold
end-to-end through the real `portrait validate-translation` binary.

## Probes that held (no defect)

- A1 decoy-leak (value into a non-value-bearing `reserve`): spend PROVED "model
  creates no value" — honest-scope correct (V is name-based; `reserve` is not in
  V). Not unsoundness; a documented scope edge.
- A2 two-balance-leg mint (`a+2x`, `b-x`): conservation REFUTED. Correct teeth.
- A3 const-mul overflow (`balance * 1e12`): range REFUTED. Correct.
- A4 `mint_` prefix bypass: conservation exempt (documented) BUT range still
  REFUTES the overflow — the exemption does not escape range. Correct.
- A5 / A10 bounded_supply: A5 sema-rejected (A4 in depth); A10 (sema-passing,
  body advances supply by `2*amount`) → invariant-preservation REFUTED. Correct.
- A6 refinement decoy-arg (`amount` guarded, real debit via `qty`): refinement
  PROVED `amount>=0` (a TRUE statement about `amount`) while range REFUTES the
  `qty` underflow. Honest-scope, not a false-PROVED.
- A7 contradictory guards (`amount>=5 ∧ amount<=1`): every class UNKNOWN
  ("vacuous transition"), never PROVED. SAT(T) guard intact.
- A8 int underflow spend: spend PROVED (no value created — true) + range REFUTED
  (underflow caught). Correct division of labour.
- A9 distinct-amount move: conservation REFUTED. Correct.
- A11 empty-V escape (money field named `funds`): "no VCs generated" with explicit
  reason, never PROVED. Vacuity trap held; CLI is honest.
- A12 monotonic_seq (`seq: seq+1`): invariant-preservation PROVED. Correct.
- M4: every REFUTED over an auth-guarded covenant is CANDIDATE (the witness
  formally touches the `checkSig` UF). Fail-closed — CONFIRMED is essentially
  unreachable whenever an auth guard is present. Conservative, not a false-CONFIRM.
- Cross-check: footer correctly disclaims independence (same binary, perturbed
  config; A3 reduced, not discharged). No overclaim.

## Honest-scope notes (not defects, worth a reviewer's eye)

- `validate-translation` via the CLI re-emits the `.sil` from the same model and
  emitter, so it validates the emitter against itself — it cannot catch real
  engraver drift through the CLI path. The teeth live only at the library API.
- The value-bearing set V is name-based (`balance`/`amount`/`supply`/`*balance`
  /coin). Renaming a real money field outside that predicate silently removes it
  from conservation (A1, A11). Documented, but a soft footgun.
