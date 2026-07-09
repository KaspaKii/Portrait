# Getting Started with Portrait — your first 15 minutes

> **Maturity: pre-production, unaudited, testnet-only.** No external security
> audit or external review has taken place — internal adversarial hardening is
> not external review. Nothing is on mainnet; live evidence is perishable Kaspa
> testnet-10 (TN10) evidence (the testnet resets). `portrait prove` proves the
> covenant **model** under stated assumptions, **not** the emitted `.sil` and
> **not** anything on-chain.

This walkthrough takes you from a clean checkout to a checked, model-proved,
translation-validated, compiled covenant in about 15 minutes. Every command and
every "expected output" block below was run verbatim against the workspace;
only the file paths are trimmed (they will show *your* paths).

The path: **write** a small covenant → **check** it (structural invariants) →
**prove** it (opt-in SMT, model-level) → **validate-translation** (model ↔
`.sil` structural correspondence) → **ship** (emit `.sil` + compile with
`silverc` + Hallmark manifest).

---

## 1. Prerequisites

- **Rust toolchain** (stable, via [rustup](https://rustup.rs/)). Everything
  runs through `cargo`; no other build system is needed.
- **z3 — OPTIONAL** (for step 5, `portrait prove`). Install via
  `brew install z3` (macOS), your distro package manager, or the releases at
  <https://github.com/Z3Prover/z3>. The CLI finds it on `PATH` or via the
  `$PORTRAIT_Z3` env var.
  **Without z3 nothing breaks and nothing lies:** `portrait prove` reports
  every proof obligation as an honest `unknown` — it never fabricates a
  `proved`. Steps 1–4 and 6–7 do not use z3 at all.
- **`silverc` — required only for step 7** (`portrait ship`, the SilverScript
  compile stage). `silverc` is the compiler of
  [kaspanet/silverscript](https://github.com/kaspanet/silverscript), Kaspa's
  covenant language; build/install it from that repository so the `silverc`
  binary is on your `PATH` (a cargo install lands it at `~/.cargo/bin/silverc`).
  Without it, `ship` fails closed at the compile stage (see step 7) — steps
  1–6 work fine.

## 2. Enter the workspace and build

From the repository root, the compiler workspace lives in `portrait/`:

```sh
cd portrait                    # the Rust workspace (Cargo.toml + crates/)
cargo build -p portrait-cli    # first build fetches deps; takes a few minutes
cargo run -q -p portrait-cli -- version
```

Expected output:

```text
portrait 0.1.0
```

All commands below are run from this directory as
`cargo run -q -p portrait-cli -- <command>`. Run it with no arguments to see
the command list (`check`, `prove`, `compose`, `validate-translation`, `ship`,
…).

## 3. Write a small covenant

Make a scratch directory anywhere (the examples below use
`~/portrait-quickstart`) and create `Pool.portrait`:

```sh
mkdir -p ~/portrait-quickstart
```

```portrait
pragma portrait ^0.1.0;

// A tiny two-bucket pool. The committed operator may move value from the
// pool bucket into the fee bucket; the total across the two is conserved.
app Pool {
  role pool {
    param pubkey operator;
    param int    pool_balance;
    param int    fee_balance;
    state { pubkey operator; int pool_balance; int fee_balance; }

    #[covenant(mode = transition)]
    entrypoint function skim(sig auth, int amount) : (pubkey operator, int pool_balance, int fee_balance) {
      requires checkSig(auth, operator);
      requires amount >= 0;
      requires amount <= pool_balance;
      requires pool_balance <= 100000000;
      requires fee_balance >= 0;
      requires fee_balance <= 100000000;
      return Pool {
        operator:     operator,
        pool_balance: pool_balance - amount,
        fee_balance:  fee_balance + amount
      };
    }
  }

  lifecycle { live -> live via pool.skim; }
  invariant value_conserved;
  invariant no_undeclared_state;
}
```

Why each piece is there (the checker genuinely enforces all of this — try
deleting a line and re-running step 4):

- `requires checkSig(auth, operator);` — under `value_conserved`, the checker
  **rejects** any state-mutating transition with no authorization bound to a
  committed key.
- `pool_balance - amount` / `fee_balance + amount` — a conserving return:
  the same `amount` leaves one value-bearing field and arrives in the other.
- `requires amount >= 0; requires amount <= pool_balance;` — the guards the
  conservation and range proofs in step 5 will actually rely on.
- The `<= 100000000` bounds ground the overflow proof: they give the prover an
  upper bound so `fee_balance + amount` provably stays inside the on-chain
  `u64` window. Remove them and step 5 honestly reports a candidate
  counter-example instead of a proof.

(Alternative: `portrait new MyApp --template counter|escrow|csci|treasury`
scaffolds a known-good starter file.)

## 4. `portrait check` — structural invariants

```sh
cargo run -q -p portrait-cli -- check --explain ~/portrait-quickstart/Pool.portrait
```

Expected output:

```text
invariant report — app Pool
  [ok] value_conserved (declared)
  [ok] no_undeclared_state (declared)
  [ok] lifecycle_reachability (structural)
  [ok] flow_integrity (structural)
  [ok] transition_return_consistency (structural)
  [ok] expression_typing (structural)
verdict: ok — all declared invariants and structural checks passed.
```

(Plain `check` without `--explain` prints just the verdict.) These are
**structural, type-level** checks — they catch undeclared state, unreachable
lifecycle states, non-conserving return shapes, missing authorization — they
are not proofs of runtime behaviour and say nothing about liveness on the DAG.

## 5. `portrait prove` — opt-in SMT proof of the MODEL

```sh
cargo run -q -p portrait-cli -- prove ~/portrait-quickstart/Pool.portrait
```

Expected output (with z3 installed):

```text
Prove — ~/portrait-quickstart/Pool.portrait
  [proved] pool.skim value-conservation — negated VC is unsat (cross-checked: confirmed unsat by a second z3 run of the same binary under a perturbed configuration — different random seed + reordered assertions; this catches search-order instability, NOT a second independent solver and NOT a proof certificate, so assumption A3 [trusted solver] is reduced, not discharged)
      unsat-core: a7_eq_pool_balance_p_expr, a8_eq_fee_balance_p_expr, a9_not_expr (assertions the proof relied on)
  [proved] pool.skim range-overflow — negated VC is unsat (cross-checked: confirmed unsat by a second z3 run of the same binary under a perturbed configuration — different random seed + reordered assertions; this catches search-order instability, NOT a second independent solver and NOT a proof certificate, so assumption A3 [trusted solver] is reduced, not discharged)
      unsat-core: a1_ge_amount_0, a2_le_amount_pool_balance, a3_le_pool_balance_100000000, a4_ge_fee_balance_0, a5_le_fee_balance_100000000, a7_eq_pool_balance_p_expr, a8_eq_fee_balance_p_expr, a9_or_expr_expr (assertions the proof relied on)
  boundary: Proven over the Portrait MODEL under assumptions A1-A4; NOT a proof of the emitted .sil script or the deployed covenant. pre-production, unaudited, testnet-only.
```

How to read it:

- Two verification conditions were generated for `pool.skim` and both were
  discharged: **value-conservation** (the two balances always sum to the same
  total) and **range-overflow** (no result leaves the `u64` window). A line
  reads `proved` **only** when z3 returns `unsat` for the negated condition.
- The `unsat-core` lists which of your `requires` guards the proof actually
  relied on — note the range proof uses all five guards.
- **The boundary footer always prints.** These are proofs about the Portrait
  *model* under assumptions A1–A4 — **not** about the emitted `.sil` script,
  and **not** about any deployed covenant.

**Without z3** the same command prints honest UNKNOWNs and exits 0
(UNKNOWN is first-class, not a failure — and never a fake `proved`):

```text
Prove — ~/portrait-quickstart/Pool.portrait
  [unknown] pool.skim value-conservation — z3 not found on PATH or $PORTRAIT_Z3; install z3 to discharge VCs — reporting UNKNOWN, never PROVED
  [unknown] pool.skim range-overflow — z3 not found on PATH or $PORTRAIT_Z3; install z3 to discharge VCs — reporting UNKNOWN, never PROVED
  note: z3 not found on PATH or $PORTRAIT_Z3; install z3 to discharge VCs — reporting UNKNOWN, never PROVED
  boundary: Proven over the Portrait MODEL under assumptions A1-A4; NOT a proof of the emitted .sil script or the deployed covenant. pre-production, unaudited, testnet-only.
```

## 6. `portrait validate-translation` — model ↔ `.sil` correspondence

Step 5 proved the model. This step checks, structurally, that the lowered
`.sil` still says the same thing as the model:

```sh
cargo run -q -p portrait-cli -- validate-translation ~/portrait-quickstart/Pool.portrait
```

Expected output:

```text
Validate-translation — ~/portrait-quickstart/Pool.portrait
  CORRESPONDS — every model guard maps to a .sil require, and model and .sil agree on every value-bearing write (none added, altered, or dropped)
  note: STRUCTURAL correspondence between the Portrait MODEL and the emitted .sil (catches a dropped guard, or a value-bearing write added/altered/dropped between model and .sil); NOT a semantic refinement proof — it does NOT establish behavioural equivalence (that needs a .sil semantics + an SMT refinement obligation). pre-production, unaudited, testnet-only.
```

Honest scope, as the output itself says: this catches a *dropped guard* or a
*value-bearing write added/altered/dropped* between model and `.sil`. It is
**not** a refinement proof and does not establish behavioural equivalence.

## 7. `portrait ship` — emit the `.sil` and compile it

The end-to-end command: check → emit `.sil` + constructor JSON → compile with
the real `silverc` → write a re-derivable Hallmark manifest.

```sh
cargo run -q -p portrait-cli -- ship ~/portrait-quickstart/Pool.portrait
```

Expected output:

```text
[pounce] skim → Covenant
[emit]   Pool.sil
[silverc] ok Pool.sil
Shipped — Pool (~/portrait-quickstart/Pool.portrait)
  [pass] parse — parser returned Ok
  [pass] sema — all structural checks passed
  [pass] emit — emitted 1 covenant(s): Pool
  [pass] silverc-accepts[Pool] — silverc exited 0
  covenants: 1
  KovId: 4e466cbaff83dd59a9e46387fa5324f7ccb8a9067a5acc0e09e5e0fcc2617d6f
  manifest: ~/portrait-quickstart/Pool.hallmark.json
  maturity: pre-production, unaudited, testnet-only
  rederive: portrait verify ~/portrait-quickstart/Pool.portrait   (or: cargo run -p portrait-cli -- verify ~/portrait-quickstart/Pool.portrait)
  next: deploy to TESTNET is opt-in and handled by the settlement workflow (pre-production, unaudited, testnet-only — never mainnet).
verdict: ok — all stages passed.
```

Beside your source you now have:

```text
Pool.portrait        # your source
Pool.sil             # the emitted SilverScript covenant
Pool_ctor.json       # constructor args for silverc
Pool.json            # silverc's compiled output (script bytes)
Pool.hallmark.json   # re-derivable manifest: every claim maps to a re-runnable check
```

The `KovId` is `sha256` of the compiled script bytes — the covenant identity.
Note what `ship` does **not** do: it does not deploy anything. Deploy is
opt-in, testnet-only, and handled by a separate settlement workflow — never
mainnet.

If `silverc` is not on your `PATH`, `ship` fails closed at the compile stage
(exit 1) rather than pretending:

```text
[pounce] skim → Covenant
[emit]   Pool.sil
error: silverc not found: No such file or directory (os error 2)
```

> Reproduction note: if you *do* have `silverc` installed under `~/.cargo/bin`,
> invoking via `cargo run` will still find it even with a stripped `PATH`
> (cargo re-injects `~/.cargo/bin` into the child environment). To demo the
> fail-closed behaviour in that case, run the built binary directly:
> `target/debug/portrait ship …`.

## 8. Where to go next

- **[LANGUAGE.md](LANGUAGE.md)** — the `.portrait` surface: roles, lifecycles,
  invariants, covenant-ID lineage.
- **[CATALOGUE.md](CATALOGUE.md)** — the pattern taxonomy, organised pattern-library-style.
- **`library/`** — the shipped covenant-patterns library: **35 covenant
  sources / 10 cross-layer (vProg) patterns**, of which 5 are settled live on
  TN10 (perishable testnet evidence; the other 5 vProg patterns are
  emit-verified only). Every file compiles through the pipeline you just ran.
  Expectation-setting for `portrait prove` on library files: not every verdict
  is `[proved]`, and that is honest behaviour, not breakage — some patterns
  report `[refuted?] … CANDIDATE` (an *unvalidated* counter-example that
  depends on an uninterpreted value such as `checkSig`; a possible
  over-approximation artifact, flagged rather than trusted), and some report
  "(no VCs generated)" with an explanation (no value-bearing arithmetic to
  prove). Only `[proved]` over a satisfiable model means proved.
- **`portrait compose`** — opt-in multi-role protocol composition (Composer
  M1): type-level projection/duality/linearity checks over a multi-role flow.
  Model-level only — not a runtime, and not a liveness guarantee.
- **`portrait check --explain` on a broken file** — delete the `checkSig`
  guard from `Pool.portrait` and re-run step 4 to watch the checker reject it
  with a named diagnostic. The fastest way to learn what the invariants mean.
