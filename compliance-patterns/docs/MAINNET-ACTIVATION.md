# Toccata Mainnet Activation — Readiness Notes

**Toccata activates on Kaspa mainnet at DAA 474,165,565 (~30 Jun 2026).**

TN10 has had the full Toccata bundle live since launch (covenants, KIP-16/17/20/21).
All library evidence tags were captured on TN10. On mainnet activation, the same
pattern scripts become deployable on the live network.

---

## Library readiness status (2026-06-28)

| Check | Status |
|---|---|
| `cargo test --workspace` (420 tests) | GREEN |
| `cargo clippy + cargo fmt` | GREEN |
| mdBook build | GREEN |
| Script bytes embedded in crates | Valid (proven on TN10 via rusty-kaspa v2.0.0) |
| Engine pin (`rusty-kaspa` tag `v2.0.0` = `90dbf07`) | Matches Toccata release |
| silverscript upstream | Updated to c46e0e2 (see §Upstream changes below) |

---

## Operational steps for mainnet

After mainnet activates:

1. **Verify covenants deploy on mainnet** — fund a mainnet wallet, run `hello-vault`
   example, confirm `verify_p2sh_spend_offline` still accepts (engine version check).
2. **Re-capture evidence tags** — TN10 evidence is perishable by design. Once patterns
   are confirmed on mainnet, re-capture `[KCP-P2SH-001m]`, `[KCP-VT-002m]`, etc. with
   mainnet txids. Until then, TN10 evidence remains the record.
3. **CSCI demo** — fund wallet `kaspatest:qqh73x8ur...` on TN10 (or mainnet equivalent),
   run `examples/csci-demo`, update `docs/CSCI-PROVENANCE.json`.

---

## Upstream silverscript changes (2026-06-28, c46e0e2)

Two significant upstream commits since our last silverscript verification:

### c46e0e2 — Typed `checkSigFromStack` builtins
- `checkSigFromStack(sig, data, pubkey)` is now a typed builtin in silverscript.
- Previously documented as "no ZK-verify builtin" — this is separate from ZK but
  enables data-signature verification inline (relevant to CSCI covenant).
- **No impact on existing script bytes.** The embedded bytes in `kcp-ktt-token`,
  `kcp-sealed-lineage`, `kcp-transferable-record` were compiled from the old toolchain
  and are still valid on the Toccata engine.

### faaa074 — `validateOutputStateWithTemplate` nested struct fields
- `validateOutputStateWithTemplate` now supports contracts whose state includes
  nested structs (not just flat fields).
- Enables more expressive covenant state shapes in future patterns.

### Breaking change for source recompilation
The new silverc requires `--constructor-args <CTOR.json>` for any contract with
constructor parameters. Invoking `silverc contract.sil` without a CTOR JSON now
fails with "constructor argument count mismatch" if the contract has params.

**Impact on this library:**
- The `.sil` source files in `crates/*/covenant/` fail to recompile without CTOR JSONs.
- The embedded compiled script bytes are NOT affected — they were compiled once and
  embedded; the library uses those bytes directly at runtime.
- Portrait M1 and the Engraver pipeline need to always pass `--constructor-args`
  when invoking silverc. The `emit_ctor()` helper already generates the CTOR JSON;
  the CLI invocation just needs to use it.
- `kcp-cli`'s `silverscript check` invocation may need updating if it calls silverc
  on covenant-declaration contracts with params.

**Mitigation:** add CTOR JSON files alongside each `.sil` in `crates/*/covenant/`
so source recompilation is possible. CTOR values are the genesis/example values
(same as Portrait's `emit_ctor()` defaults: max_ins=1, max_outs=1, zeroed state).

---

## New silverscript features unlocked

| Feature | What it enables |
|---|---|
| `checkSigFromStack(sig, data, pubkey)` | CSCI covenant can verify data signatures on-chain without ZK; mixed auth paths |
| `validateOutputStateWithTemplate` with nested structs | CSCI state nesting (`KttState` inside `CsciState`) as a covenant state field |
| Official KCC20 + KCC20Minter `.sil` examples | `silverscript-lang/tests/examples/kcc20.sil` — reference implementation we shape-align against |
| Transition return type syntax `f(...) : (State) { return({...}); }` | Portrait M1 fix — emit-level change replaces manual `validateOutputState` call |

---

## What does NOT change on mainnet activation

- Engine pin stays at rusty-kaspa v2.0.0 (= Toccata release).
- KIP-16 tag 0x21 (RISC Zero succinct STARK) is live on TN10 and will be on mainnet.
- Script bytes in `kcp-*` crates compile and run unchanged.
