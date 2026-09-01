# Changelog

All notable changes to `kaspa-compliance-patterns` (the Covenant Patterns Library
for Kaspa) are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
with the understanding that **pre-1.0 releases may include breaking changes
between minor versions**.

> **Maturity stamp:** every release before v1.0 is **pre-production, unaudited,
> testnet-only.** On-chain evidence is **perishable by design** — testnets reset.
> Anchor identifiers (covenant_id, tx_id) cited in any release note refer to the
> testnet state at the time of writing and may not resolve on a later testnet.
> See `KNOWN-ISSUES.md` for the full caveats.

## [Unreleased]

### Added

- **Per-pattern threat models — and the README claim they back.** `README.md`
  advertised covenant components "each with tests and a threat model"; no
  pattern crate carried one, so the claim was false. All ten pattern crates now
  ship a `## Threat model` section on a fixed five-heading template (assets /
  assumed attacker capabilities / what consensus enforces / what is trusted
  off-chain / known limits and non-goals), each stamped pre-production,
  unaudited, testnet-only and explicitly **not a security audit**. Two of the
  ten had no README at all — `kcp-governance` and `kcp-csci` — and now have one.
  A gate test (`crates/kcp-cli/tests/pattern_readme_threat_models.rs`) walks the
  workspace `members` list and fails if a pattern crate is missing the section,
  any of its five headings, the not-an-audit stamp, or a minimum body length, so
  the claim cannot silently drift false again; the README says plainly that the
  *content* is human-reviewed rather than mechanically verified. Writing the
  models surfaced real defects, fixed below.

- **`kcp-pq-anchor` ships budget measurement** — `sigop::measure_pq_anchor_units`
  and `sigop::fits_pq_verify_budget` (behind the crate's new **`wrpc`** feature,
  since they run the real consensus VM) plus the always-available constant
  `sigop::MAX_COMMITTABLE_SCRIPT_UNITS`, with `tests/budget_ceiling.rs`
  (`required-features = ["wrpc"]`) running the shipped reference proof through a
  **real P2SH transaction input** on the pinned v2.0.0 VM. See the fix below for
  why the hardcoded `255` needed a measurement beside it.
  **Dependency posture:** `kaspa-txscript` / `kaspa-consensus-core` are
  **optional** deps enabled by `wrpc`, matching every other crate in the
  workspace — `cargo build -p kcp-pq-anchor` (and `-p kcp-csci`) pulls no
  rusty-kaspa tree, verified with `cargo tree --edges normal`. Because the
  measurement is therefore opt-in and the risk is fund loss, the warning is
  repeated in `build_pq_anchor_redeem`'s rustdoc, the crate README quick start,
  the threat model, and the book page: **measure with `--features wrpc` before
  funding a tag-0x21 address.**

- **The gate now builds and tests every standalone `examples/` project.**
  `examples/` are separate workspaces outside `[workspace] members`, so
  `cargo test --workspace` never compiled them and `_harness/ci.sh` built only
  `hello-vault` — which is how a breaking API change shipped a broken example
  this session (`examples/compliance-workflow`, advertised in the book and the
  crate README, failed to compile). `ci.sh` now iterates every directory under
  `examples/` with a `Cargo.toml` and runs `cargo test` in it (~4m30s warm; still
  skipped by `--fast`), and `.github/workflows/ci.yml` compile-checks each with
  `cargo check --all-targets` (a cold runner would otherwise rebuild the engine
  tree once per project — each has its own target dir).

- **The three state-continuity engine proofs are now reproducible in-repo.**
  `kcp-sealed-lineage`, `kcp-transferable-record` and `kcp-ktt-token` each ship
  `tests/covenant_engine.rs` (behind `wrpc`, so it runs in the gate's
  `--all-features` pass). Previously nothing in the published tree loaded or
  executed the committed `covenant/*.sil` artifacts: the "engine-proven" claim
  rested entirely on an archived external harness pinned to a *different*
  rusty-kaspa commit. The new tests load the committed `*.compiled.json`, splice
  per-state scripts into its `state_layout` region via the new
  `kcp_common::covenant` module, and execute them through
  `TxScriptEngine::from_transaction_input` with `covenants_enabled: true` and a
  real `CovenantsContext::from_tx` on the pinned **v2.0.0 (`90dbf07`)** script
  VM — 32 tests in all: ACCEPT baselines plus a REJECT per invariant, covering
  sealed-lineage L-1/L-2/**L-3 (including terminality: a CLOSE state can never be
  spent)**/L-4 and both L-4 bounds; transferable-record TR-1/TR-2/TR-3; and
  KTT-1/KTT-2/KTT-3 **on the 1→2 split shape as well as 1→1** — the split is the
  only shape where KTT-1's sum over covenant outputs is non-degenerate, since
  1→1 collapses it to `a == b`. Every rejection is **two-sided**: the violating
  transition must be refused by the covenant's own `require`, *and* the same
  transition with only the offending field restored must be accepted, which pins
  the failure to the named invariant (a bare `VerifyError` cannot say which
  `require` fired) and survives a reordering of the covenant's checks. Each crate
  additionally re-encodes the artifact's own genesis-template state region and
  requires it to reproduce the committed script byte-for-byte, which pins the
  field order, widths and push encoding. No `silverscript-lang` dependency (it
  would float the engine pin) and no key fixtures — every test derives a
  deterministic, never-funded keypair from a fixed seed and splices the matching
  public key into the state region.

  **Honest scope.** This reproduces the **script-VM** half only.
  - *Not the deployed script.* Nothing in the repo records the deployed script,
    its scriptPubKey, or the `-003` genesis `covenant_id` — and the covenant id
    derives from the funding outpoint rather than the script, so it cannot be
    recomputed from the artifact. That link is still attested by an archived
    out-of-repo capture. The one in-repo corroboration is an execution-cost
    fingerprint: each committed artifact consumes exactly the script-unit count
    recorded for its live preflight — 107 149 (`KCP-SL-003`), 105 047
    (`KCP-TR-003`), 111 410 (`KCP-KTT-003`) — now asserted by
    `*_engine_cost_matches_recorded_live_preflight`. Evidence, not a binding.
  - *Not transaction-level validation.* Only input 0's script runs: no
    transaction mass, no KIP-9 storage mass, no standardness.
  - *Not the live half.* Submitting a transaction is not reproducible from a
    test.
  - *KTT's 2-covenant-input merge shape is not covered.* Logged in
    `KNOWN-ISSUES.md`.

- **`kcp-governance::lineage`** — opt-in binding that maps a `GovernorState`
  run onto a `kcp-sealed-lineage` append-only lineage: one sealed event per
  lifecycle snapshot (event-classed `GENESIS` / `APPEND` / `CLOSE`), the
  `lineage_id` derived from the governor's **immutable config** (proposal id,
  voting window, signatory set, threshold, timelock delay), each event sealing
  the **full canonical `GovernorState`**. `verify_governor_lineage` runs five
  off-chain checks: non-empty, sealed-lineage chain invariants (L-1..L-4),
  config-identity, full-state commitments, and a well-formed lifecycle lattice
  (event-class + status-transition legality + per-state quorum/timelock
  self-consistency). Ships with an offline auditor example
  (`crates/kcp-governance/examples/governance_lineage.rs`) and inline unit tests.
  **Honest scope:** the binding is **pure/offline** and does **not** put anything
  on-chain. On the default plain-pay-to-address anchoring path consensus does not
  introspect the payload, so the chain invariants — and every check above — are
  validated off-chain only; consensus rejects a malformed successor **only if**
  the run is anchored under the separate covenant-id-bound sealed-lineage chain
  (`[KCP-SL-003]`), which **no library API auto-wires**. Even then it enforces
  the lineage *structure*, not the governance *rules* (quorum→`Passed`,
  timelock→`Executed`, signatory legality), which stay off-chain in the value
  types. **No new covenant, no engine change, and no new on-chain evidence is
  claimed.**

### Fixed

- **`kcp-pq-anchor`: the tag-0x21 compute budget was unmeasured, and it is
  0.25% from a hard ceiling.** `sigop_count_for_pq_verify()` returns 255 — the
  `u8` **maximum**, so `255 × 100_000 + 9_999 = 25,509,999` script units is the
  largest budget a version-0 input can express, with nothing above it. Nothing
  in the repo measured the real cost: `tests/engine_accept.rs` runs the redeem
  via `from_script` — no transaction, no P2SH wrap, no compute commitment. The
  new measurement puts the reference proof at **25,446,182 units, leaving
  63,817 (0.25%)**. A bigger seal, a longer control-inclusion path or a
  different guest can cross that line, and since the proof fields live inside
  the redeem script — and therefore inside the P2SH address — an unbudgetable
  spend has no alternative path and the funds are **permanently unrecoverable**.
  Callers must now measure before funding; the ceiling itself is an engine-level
  limit, recorded in `KNOWN-ISSUES.md`.

- **A broken example shipped, and the book documented a dead API.**
  `examples/compliance-workflow` (both `src/main.rs` and `tests/smoke.rs`) still
  called the two-argument `validate_chain`, and
  `book/src/patterns/transferable-record.md` showed the removed parameter as
  current API while claiming an empty event slice is valid — which the change
  below makes false. Both fixed, along with the gate gap that hid them; the book
  page now also states that controller keys are never verified. Swept the rest
  of `book/src/` for claims this session invalidated: the paired-attestation page
  gained the not-custody warning, the pq-anchor page the budget-ceiling warning,
  and the yield-vault page the preview/saturation caveats.

- **`kcp-transferable-record`: `validate_chain` verified custody with nothing.**
  It took a `_genesis_controller` argument it never read while its doc-comment
  claimed it validated "against the genesis controller", and
  `TransferEvent::controller_xonly` was never read either — so a fabricated
  chain with invented or discontinuous controller keys validated clean. It also
  returned `Ok(())` for an **empty** event list, reporting success for having
  checked nothing. The data model cannot express custody continuity (the
  authorising key is recorded nowhere, and the controller is not in the on-chain
  payload — it is the UTXO's locking script), so the dead parameter is **removed**
  rather than left looking like a check, the empty slice is now
  `Error::LineageEmpty`, and the module docs, README and threat model state that
  the UTXO chain is the only custody anchor. A characterisation test pins the
  limitation. **Breaking:** `validate_chain(&events)` drops its first argument.

- **`kcp-yield-vault`: share conversion could wrap.** Both converters ended in a
  truncating `as u64`. `total_assets == 0` with `total_shares > 0` is reachable
  by redeeming into floor-division dust, and there `convert_to_shares` divides by
  the virtual asset alone and can exceed `u64` — wrapping to a small (often
  zero) share count. Both now compute in `u128` and **saturate**, with
  regression tests in both directions.

- **`kcp-governance`: a rejected proposal could be revived.** `refresh_status`
  treated only `Cancelled` and `Executed` as terminal, but `GovernorState`'s
  fields are `pub` and `MultiSigVote::approve` is window-blind, so recording
  approvals directly on `state.vote` after the voting deadline flipped a
  **`Rejected`** proposal to `Passed` on the next refresh. `Rejected` is now
  terminal too — matching `lineage::verify_governor_lineage`, which already
  treated it that way — with a regression test.

- **`kcp-vault`: the documented CLTV threshold was wrong by 1000×, and its unit
  was wrong.** Module docs (and the new threat model) said the DAA/timestamp
  boundary was `500,000,000`; rusty-kaspa's `LOCK_TIME_THRESHOLD` is
  `500_000_000_000`. Above it, `lock_time` is compared against block timestamps,
  which come from `unix_now()` — **milliseconds**, not seconds — so
  `TimelockUnixSeconds` is a misnomer and a deadline given in real unix seconds
  silently becomes a DAA-score timelock. Documented in both places and logged in
  `KNOWN-ISSUES.md`; renaming the public variant is deferred.

- **`kcp-paired-attestation`: the blind-negotiation claim contradicted itself.**
  The README said the XOR "prevents either party from unilaterally choosing a
  blind"; with no commit-then-reveal step the party that reveals second can
  steer the combined blind to any value. Corrected where a reader meets it
  first, and the stale `lib.rs` header (which still said the P2SH spend plumbing
  was out of v0 scope, years after the `onchain` module shipped) now describes
  what the crate actually does — including that the v1 covenant is **not
  custody**.

- **The generated Solidity→Ownable migration scaffold did not compile.**
  `kcp new ownable` wrote a `Cargo.toml` with only `kii-solidity-compat` as a
  dependency, while the `src/main.rs` it generated alongside calls
  `hex::encode` three times — so the very first `cargo run` in a freshly
  scaffolded project failed with `E0433: use of unresolved module or unlinked
  crate 'hex'`. The generated manifest now declares `hex = "0.4"`. Verified by
  generating into a scratch directory and running both `cargo test` (6 passed)
  and `cargo run` there. The defect class is now covered where it occurred:
  `crates/kcp-cli/tests/scaffold_from_solidity_ownable.rs` builds and tests the
  generated ownable project, and separately asserts that every crate the
  generated `main.rs` names is declared in the generated manifest. (The
  pre-existing `cargo check` coverage was on the **vault** scaffold only, so it
  could never have caught this.)

- **`select_smallest_covering` was copy-pasted into four crates.** The identical
  UTXO-selection helper existed in `kcp-sealed-lineage`, `kcp-ktt-token`,
  `kcp-paired-attestation` and `kcp-transferable-record`. It is now
  `kcp_common::tx::select_smallest_covering` (public, with the existing unit
  tests moved onto the real `RpcUtxosByAddressesEntry` type) and imported by all
  four. The duplicated `MIN_CHANGE_SOMPI` constants in `kcp-sealed-lineage`,
  `kcp-ktt-token`, `kcp-transferable-record` and `kcp-paired-attestation` are
  re-exports of the `kcp-common` definition, so existing public paths still
  resolve. The
  `create_*`/`append_*` transaction builders were deliberately **not** unified —
  they genuinely diverge per pattern.

- **`--all-features` emitted 10 `output filename collision` warnings.** Five
  crates each declared an example named `testnet_evidence` and two more an
  `onchain_evidence`, so cargo overwrote the binaries of same-named targets. The
  example targets are now crate-qualified (`sealed_lineage_testnet_evidence`,
  `vault_onchain_evidence`, …); the READMEs, `docs/ENVIRONMENT.md`,
  `SELF-AUDIT.md` and `examples/hello-vault/README.md` name the new targets. The
  warnings are gone.

- **`kcp-cli`'s generated-project compile check no longer skips itself.**
  `scaffold_vault::generated_project_cargo_check` was `#[ignore]`d because a cold
  `cargo check` of a standalone generated project rebuilds the whole rusty-kaspa
  tree (~10 minutes). It now sets `CARGO_TARGET_DIR` to the parent workspace's
  target directory, so the generated project reuses the already-built engine
  artifacts and the whole test file runs in **seconds, not minutes** — cheap enough for the
  gate, which means the vault scaffold's output is now actually compiled on every
  run. Rather than making a nested-`cargo`, network-dependent test unconditional
  in the mandatory gate, it (and the new ownable equivalent) is opted in with
  `KCP_GATE_SCAFFOLD_BUILD=1`, which `_harness/ci.sh` and the GitHub Actions
  workflow both set: an offline contributor on a fresh clone gets a **skip**, not
  a hard failure. The timelock and composite equivalents stay `#[ignore]`d
  (2 ignored tests workspace-wide; an earlier review's figure of 14 was wrong).

- **The `covenant/*_ctor.json` files are not the constructor inputs that produced
  the committed artifacts, and must not be cited as provenance.**
  `ktt_ctor.json` gives `genesisAmount=0, maxCovIns=1, maxCovOuts=1`; the
  committed `ktt.compiled.json` disagrees on all three, three independent ways —
  its state region decodes to `amount = 0x03e8 = 1000`; its program body bounds
  arity with `OpCovInputCount OpDup Op2 OpLessThanOrEqual` (sealed-lineage and
  transferable-record carry `Op1` in the same position); and the pinned engine
  accepts a 1-covenant-input → 2-covenant-output conserving split.
  `sealed-lineage_ctor.json` has the same problem, claiming `genesisTBucket=0`
  where the artifact encodes `1700000000`. The ctor files post-date the artifacts
  they sit beside. Both covenant READMEs now derive their provenance rows from
  the artifact bytes — asserted in `tests/covenant_engine.rs`, which re-encodes
  each artifact's own genesis-template state region and requires it to reproduce
  the committed script byte-for-byte — and carry an explicit warning; the real
  constructor inputs are marked `[FACT-NEEDED]`. Logged in `KNOWN-ISSUES.md`.

  *This corrects a wrong correction:* an earlier pass in this same cycle "fixed"
  the KTT provenance row **to** the ctor file's values, replacing a true
  statement with a false one. The arity claim is security-relevant — reading
  `maxCovOuts=1` tells a reader that fan-out is impossible on a token-supply
  covenant when the deployed artifact permits it up to 2.

- **`kcp-pq-anchor` README — canonical `hashfn` push corrected.** The "Key
  invariant" section claimed `hashfn` (Poseidon2 = 1) must be pushed as a numeric
  `OP_1` (0x51) and never as a 1-byte data push. This was backwards: the code
  (`src/anchor_script.rs`, `push_data(&mut script, &[HASHFN_POSEIDON2])`) and the
  engine's `parse_hashfn` require a 1-byte data push (`0x01 0x01`); a numeric
  `OP_1` would be rejected. The README now matches the code.

- **`docs/EVIDENCE.md` transaction ids were not lookup-able.** The index claims
  every on-chain item is "independently verifiable on a Kaspa testnet explorer",
  but 14 of the 17 headline ids appeared only as 8-hex prefixes, which no
  explorer or API accepts. All 14 (`KCP-P2SH-001`, `KCP-VT-002`, `KCP-VT-003`,
  `KCP-PA-002`, `KCP-SL-003`, `KCP-TR-003`, `KCP-KTT-003`) are now listed in full
  64-hex alongside the tables, using the same "Full transaction ids" block the
  file already used for the ERC20→KTT wedge. Values traced to the evidence
  register; the perishability stamp is unchanged — these are testnet-10 ids and
  testnets reset by design.

- **The gate now exercises the real-engine tests.** All ~20 real-engine tests
  sit behind the `wrpc` feature, and both `_harness/ci.sh` and the GitHub
  Actions workflow ran default-features only — so 41 tests, 18 of them
  real-engine, never compiled under the gate. `--all-features` clippy and test
  passes are now run **alongside** (not instead of) the default-feature passes;
  `wrpc` is the only non-default feature in the workspace, so `--all-features`
  == `--features wrpc`. Coverage re-verified 2026-09-01 after the changes below:
  **452 passed / 2 ignored** default, **533 passed / 2 ignored**
  `--all-features`. (Counted from `cargo test --workspace` alone — the gate also
  shells out to a nested `cargo test` on a generated scaffold project, whose 6
  tests belong to that project, not to this workspace.)

- **The `--all-features` `cc` build blocker was stale.** `SELF-AUDIT.md`,
  `CONTRIBUTING.md` and the CI workflow comment all cited a transitive
  `cc-1.2.63` upstream failure as the reason `--all-features` was omitted from
  the gate. It no longer reproduces: `--all-features` builds, lints and tests
  clean as of 2026-09-01. All three notes replaced with the dated current fact.

- **README quickstart "Option B" ran the wrong command.** It told a newcomer
  that `cargo test -p kcp-vault` puts a multisig spend through the real engine
  and that passing it means "you have run the same code path that produced
  `[KCP-VT-002]`". With `default = []` that command does not compile
  `onchain.rs` at all — it runs 54 tests and none of them touch the engine. The
  command is now `cargo test -p kcp-vault --features wrpc` (82 tests, including
  `onchain::tests::multisig_2of2_lock_spend_executes_on_engine`), with a line
  explaining why the engine tests are feature-gated. Both counts verified by
  running both commands on 2026-09-01.

- **Drifted live test counts corrected.** `README.md` and `LAUNCH-NOTE.md` both
  claimed "357 Rust tests" (true at the 2026-07-09 release, stale since); both
  now read 452 default / 533 `--all-features`, dated 2026-09-01. The `357` in
  the `[0.1.0]` release entry below is **left as-is** — it was accurate at that
  release date and rewriting history is not fact discipline.

- **`_harness/portrait-ci.sh` no longer runs half-blind.** The Portrait gate's
  differential layer invokes the real `silverc`; the script never checked that
  `silverc` existed. It now resolves `silverc` on `PATH` then
  `$HOME/.cargo/bin`, and fails hard with `Portrait CI FAIL: silverc not found`
  if absent.

### Documented (not fixed)

- **The `kcp-paired-attestation` v1 two-datasig covenant is not custody.** Its
  satisfier is two CSFS signatures over a fixed `msg_hash` with no `OP_CHECKSIG`
  over the spending transaction, so it is a bearer credential: the moment a
  spend enters a mempool both signatures are public and any observer — trivially
  any miner — can spend the same outpoint to themselves. Adding transaction
  binding changes the redeem shape (invalidating the recorded `[KCP-PA-002]`
  evidence and its measured budget), requires deciding whose key binds the
  spend, and reorders a spend path that currently relies on CSFS being
  sighash-independent — a new consensus-facing covenant shape, not a patch.
  `KNOWN-ISSUES.md` carries the finding, the ship-anyway rationale and the
  directive: attestation signalling at dust value only, and never re-fund the
  address.

- **`kcp-sealed-lineage`'s covenant binds neither the seal nor the publisher
  key.** The `[KCP-SL-003]` covenant state has no `commitment` field, so the
  blinded seal the pattern exists to anchor is never enforced by consensus on
  any path; and `append` leaves `newStates[0].publisherPk` free, so the
  authorised publisher can silently rotate control at any append. Both are
  properties of the committed artifact — changing either recompiles the covenant
  and changes its covenant-id — so both are documented in the crate's threat
  model and `KNOWN-ISSUES.md`. `kcp-governance` inherits both.

- Also newly stated in the threat models: KTT's advertised 1→2 fan-out is the
  KIP-9 storage-mass worst case and the covenant places no floor on output
  value; `kcp-yield-vault` has **no loss/mark-down path**, so a vault whose
  underlying loses value pays early redeemers in full and leaves the loss with
  whoever redeems last; `kcp-governance` commitments seal a very low-entropy
  state under a caller-supplied blind that may be zero or reused;
  `VestingSchedule` derives `Deserialize` with no validation, so the
  `duration == 0` that `new()` rejects round-trips through `serde`; and
  `kcp-vault` timelocks carry a single deadline, so an enforced vesting schedule
  means one timelocked UTXO per tranche.

### Changed

- **`.gitattributes` added** — `* text=auto eol=lf` plus an explicit
  `*.hex text eol=lf` for the byte-exact script and proof fixtures, and `*.zip
  binary`. Preventative: a reviewer on a Windows/CRLF checkout saw 10 spurious
  golden-fixture failures. This is **not** a reproduced defect — it does not
  reproduce on an LF checkout — so nothing in the library's behaviour changed.

- **Genuinely-unused dependencies dropped:** `kcp-common` from
  `kcp-vesting` and from `kcp-pq-anchor` (neither crate references it).
  **Correction to the review finding that prompted this:** the same review
  listed `hex` in `kcp-governance` as unused. It is **not** — it is used at
  `crates/kcp-governance/src/lineage.rs:115` and `:118`, having landed with the
  governance lineage work. It stays.


- **BREAKING (pre-1.0):** `kii_solidity_compat::OwnershipRecord::transfer_ownership`
  and `renounce_ownership` now take the caller's key and are `onlyOwner`-gated:
  `transfer_ownership(by, new_owner) -> Result<Self>` and
  `renounce_ownership(by) -> Result<Self>`, returning `Err(Error::NotOwner)` when
  `by` is not the current owner and leaving the record unchanged. Previously both
  were infallible and performed **no** ownership check at all, while
  `TimelockController` in the same crate gated its equivalents — an asymmetry that
  invited misuse. The crate is `publish = false` with no external consumers; the
  scaffold generator, the migration guide and the `afternoon-migration` example
  are updated in step. As before, the value type only tracks authority in
  application logic — the covenant script is what enforces it on-chain.

- **BREAKING (pre-1.0):** `kcp_governance::error::GovernanceError` gains seven
  variants — `LineageEmpty`, `LineageChainInvalid`, `LineageIdentityMismatch`,
  `LineageCommitmentMismatch`, `LineageIllegalTransition`,
  `LineageStateInconsistent`, `LineageSerialization`. Any exhaustive `match` on
  `GovernanceError` must add arms. Per the pre-1.0 policy above, additive enum
  variants are a breaking change between minor versions.

### Security

- **`kcp-yield-vault` — first-depositor inflation attack now bounded (not
  eliminated).** The ERC4626-equivalent share accounting used the naive
  `assets × total_shares / total_assets` formula, so an attacker could deposit
  1 sompi, donate a large amount into `total_assets` via `accrue`, and make the
  next depositor's shares round down to **zero** — a total loss. Conversions now
  use OpenZeppelin v5's **virtual assets/shares** with `decimalsOffset = 0`:
  `assets × (total_shares + 1) / (total_assets + 1)` and the inverse, which
  bounds the **rounding** loss and leaves the attacker out of pocket.
  **Honest limit:** with offset 0 the victim in that scenario still takes a real
  loss on the shares they do get — the attack is **bounded, not eliminated**.
  Deployments needing it closed must also seed the vault or enforce a minimum
  initial deposit. **Not audited.**
  **BREAKING (pre-1.0):** conversion results shift by rounding dust (e.g.
  redeeming 500 000 shares from a 2 000 000/1 000 000 vault now returns 999 999,
  not 1 000 000), and the previous "`total_assets == 0` iff `total_shares == 0`"
  invariant no longer holds — a full redeem can leave floor-division dust.

- **`kcp-yield-vault` — a deposit that would mint zero shares is now rejected.**
  `deposit()` returns the new `VaultError::ZeroSharesMinted` (profile unchanged)
  instead of absorbing the assets and handing the depositor nothing. This closes
  a total-loss path that the virtual-offset formula alone does not: after a full
  redeem the vault can hold residual dust with `total_shares == 0`, and there
  every deposit at or below the dust amount rounds to zero shares (measured on
  the dust state `total_assets = 666 667 / total_shares = 0`: deposits of 1,
  1 000 and 666 667 all minted 0; 666 668 mints 1). The removed zero-guard of the
  old formula had masked this. **BREAKING (pre-1.0):** `VaultError` gains a
  variant, so exhaustive `match`es must add an arm, and a previously-accepted
  (silently lossy) deposit now errors.

- **`kcp-yield-vault` — `convert_to_assets` / `preview_redeem` documented for the
  no-shares state.** With `total_shares == 0` the virtual-offset divisor is 1, so
  both return `shares × (total_assets + 1)` — e.g. `preview_redeem(1 000)` on a
  666 667-sompi dust vault returns 666 668 000, a thousand times the vault's
  holdings, for shares that cannot exist. `redeem()` still rejects
  `shares > total_shares`, so this is unreachable through the deposit/redeem
  flow. The formula is left faithful to OpenZeppelin v5 rather than
  special-cased; the method docs and the crate README now state explicitly that
  the figure is a virtual-offset extrapolation and **not** redeemable value.

## [0.1.0] — initial public release

First public release of the **Covenant Patterns Library for Kaspa** — the
OpenZeppelin-equivalent catalogue of reusable, threat-modelled covenant
components — together with **Portrait**, the covenant language and toolchain that
builds them. MIT-licensed, stewarded by the **Stichting Kii Foundation** (a Dutch
non-profit). Pre-production, unaudited, testnet-only.

### Covenant Patterns Library — the Rust crates

- **`kcp-common`** — shared plumbing: `p2sh`, `digest`, `tx`, `wallet`,
  `canonical`, `wrpc`, `error`; offline real-engine preflight
  (`verify_p2sh_spend_offline`); the P2SH covenant spend-path; KIP-14 payload
  helpers; a live round-trip on testnet-10 `[KCP-P2SH-001]`. Also ships the
  reusable `access` (Ownable, Multisig, AccessControl), `security` (Pausable,
  TimelockController), and `cryptography` (tagged hashing, Merkle proofs)
  primitive modules.
- **`kcp-vault`** — covenant-locked custody. v0 (digest-anchor) plus
  **v1 consensus-enforced** multisig + timelock + composite Any/All
  branch-selected P2SH lock+spend `[KCP-VT-001, KCP-VT-002]`.
- **`kcp-ktt-token`** — KCC20-shape-aligned regulated-token profile. v0 4-field
  state machine (issue → transfer → burn) `[KCP-KTT-001]`; state-continuity
  covenant engine-proven `[KCP-KTT-002]` and deployed live on testnet-10
  `[KCP-KTT-003]`.
- **`kcp-sealed-lineage`** — append-only sealed evidence lineage. v0 lineage;
  state-continuity covenant engine-proven `[KCP-SL-002]` and live on testnet-10
  `[KCP-SL-003]`.
- **`kcp-transferable-record`** — transferable registry record with lineage
  continuity. v0 lineage; engine-proven `[KCP-TR-002]` and live on testnet-10
  `[KCP-TR-003]`.
- **`kcp-paired-attestation`** — two-party mutual attestation. v0 off-chain
  mating `[KCP-PA-001]`; **v1 consensus-enforced two-datasig** via
  `OP_CHECKSIGFROMSTACK` (CSFS) on testnet-10 `[KCP-PA-002]`.

Additional crates in the workspace: `kcp-governance`, `kcp-vesting`,
`kcp-yield-vault` (an ERC4626-shaped vault profile), `kcp-pq-anchor` (the
tag-0x21 verifier-script helpers), `kii-solidity-compat` (a Solidity-shaped
Rosetta facade), `kcp-csci`, and `kcp` — a scaffolding CLI that generates
ready-to-run covenant projects.

### Portrait — the covenant language + cross-layer catalogue

- **35 covenant sources** compile through the pipeline (`engrave → silverc`
  exit 0), spanning finance, custody, governance, attestation, and state
  patterns. `DigitalReit.portrait` is the only multi-role source (emits 2 `.sil`).
- **10 of the 35 are cross-layer (vProg) patterns.** **Five are settled live on
  testnet-10** — ProofOfReserves, ComplianceCredential (ZK-KYC),
  ConfidentialTransfer, BatchRollup, PrivateVoting — each a real RISC Zero STARK
  (`RISC0_DEV_MODE=0`) verified in-consensus via the KIP-16 `tag-0x21` precompile,
  each with a per-pattern negative control the live node rejected. **The other
  five are emit-verified only** (MerkleProofOfSolvency, PrivateOrderMatch,
  PrivateVickreyAuction, ZkAllowlistTransfer, ZkExecutionRollup): they compile,
  engrave, and emit a RISC Zero guest, but are **not** settled live.
- **`CsciInstrument`** — the reference covenant that self-enforces its state
  machine on-chain (committed-state auth, seq-monotonicity, covenant-id binding),
  settled live on testnet-10, including a combined two-input cross-layer
  transaction that binds the STARK journal to the engine per-instance
  covenant_id, with negative controls the live node rejected.
- **Two verification engines** ship with the compiler, each making a narrow,
  honestly-scoped claim. **Lens** is an SMT proof engine over the covenant
  *model*, discharging value-conservation / range / refinement / invariant /
  spend verification conditions via z3, fail-closed (a contradictory premise
  returns `UNKNOWN`, never a false `PROVED`). **Composer** is a session-type
  engine that checks several covenants wired together form a well-typed protocol.
  Both prove model-level properties — not the emitted script and not on-chain
  behaviour; `validate-translation` checks the model↔`.sil` correspondence
  structurally.

**Honest residuals (the live vProgs):** the live covenant is the `tag-0x21`
verifier P2SH (image-id-pinned), not yet a SilverScript state machine; inputs are
fixed sample data over small fixed sets (not Merkle-rooted registries; no
persistent nullifier set); commitments are `sha256(value‖blinding)` not Pedersen;
the audit key is a v1 symmetric pad. Full detail in `KNOWN-ISSUES.md`.

### Evidence, on the released Toccata engine (`rusty-kaspa` v2.0.0 = `90dbf07`)

- All on-chain evidence is **testnet-10** `[KCP-NET-001]`; Toccata covenant
  introspection is active there `[KCP-NET-002]`.
- **First live covenant-id-bound deployment** — an anchor-only
  reserve-attestation covenant performed a covenant genesis + append on
  testnet-10, accepted by the live consensus covenant engine (`validateOutputState`
  introspection + oracle `OP_CHECKSIG`) `[KCP-RE-003]`. The three state-continuity
  pattern covenants (sealed-lineage, transferable-record, ktt-token) share the
  covenant-id-bound shape and are each deployed live — `[KCP-SL-003]`,
  `[KCP-TR-003]`, `[KCP-KTT-003]`.
- An **auditor** example independently re-verifies a live lineage head from
  public information alone (`kcp-sealed-lineage/examples/auditor`).

### Tests + build hygiene

- **357 Rust tests pass** across the library workspace (default features,
  `cargo test --workspace`, verified 2026-07-09); the Portrait compiler workspace
  passes **349 tests**. `cargo clippy --workspace --all-targets -- -D warnings`
  clean; `cargo fmt --check` clean. Rust 1.88+, edition 2021.
- The workspace pins `rusty-kaspa` tag **`v2.0.0`** (= `90dbf07`), the released
  Toccata engine, as its consensus reference.
- `examples/hello-vault/` standalone project: `cargo run` exit 0; the real
  `rusty-kaspa v2.0.0` script engine accepts the synthetic 2-of-2 multisig P2SH
  spend offline (no node, no funds).

### Honest non-claims (what v0.1.0 does NOT promise)

- **Not audited** — pre-production, unaudited; external security audit gates v1.0.
- **Not mainnet** — testnet-10 only; testnet evidence is perishable.
- **Not a standard** — these are worked, testable building blocks. The KCC20
  shape (`kcp-ktt-token`) is documented upstream in `kaspanet/silverscript` as
  pre-standard, not a frozen standard.
- **Not a product** — a Foundation public good; no token, no investment claim.
- **Not a covenant-introspection guarantee on every Kaspa client** — patterns are
  written against the released Toccata engine (`rusty-kaspa` v2.0.0); any client
  diverging from v2.0.0 consensus rules is out of scope.

### Steward + licence

- **Steward:** Stichting Kii Foundation — a Dutch non-profit foundation
  (*stichting*), incorporated 2026-06-21 (Stichting Ethereum Foundation
  precedent). The Foundation publishes public goods; it does not bill, audit, or
  certify.
- **Licence:** MIT — see `LICENSE`. Security disclosures: `SECURITY.md`.

---

## Version history

- **v0.1.0** — first published release. Content documented above.

Versions before v0.1.0 were internal workspace iterations and are not itemised
here; the repository's development history lives in `git log`.
