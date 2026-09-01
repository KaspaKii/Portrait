# Contributing to Portrait

Portrait is meant to be the *trusted* place Kaspa builders copy covenant code from. That only works if every pattern clears a high bar before it is called stable. This file is that bar.

## What "a pattern" means here

A pattern is a self-contained directory under `library/<group>/<name>/` containing **all** of:

1. **`<Name>.sil`** — the Silverscript component. Compiles cleanly with `silverscript check`.
2. **`<name>.portrait`** — the Portrait wrapper showing idiomatic use in an `app`.
3. **`tests/`** — golden tests: the expected compiled Kaspa Script for fixed parameters, plus debugger run-vectors (`cli-debugger` arg sets) covering each entrypoint's accept and reject paths.
4. **`THREAT_MODEL.md`** — see below. Non-negotiable.
5. **`README.md`** — intent, parameters, lifecycle diagram, usage, and known limitations.

A PR missing any of these is incomplete by definition.

## The threat-model standard (required)

Every pattern ships a `THREAT_MODEL.md` that red-teams the component **before** it is presented as usable, classifying findings **CRITICAL / HIGH / MEDIUM / LOW** across these axes:

- **Technical** — can funds be stolen, frozen, or double-moved? Unintended transitions? Loop-unroll / script-size limits? Introspection-field assumptions that don't hold?
- **Covenant-lineage** — can the covenant ID be forged, replayed, or orphaned? Genesis griefing? Can an attacker inject a UTXO that impersonates the lineage?
- **Economic** — fee/dust attacks, value-conservation gaps (`outputs >= inputs` checks), griefing that locks honest users out.
- **Operational** — key-management assumptions, recovery paths, what happens if a transition is never called.
- **Regulatory / legal** — only where a pattern implies a financial instrument or claim (e.g. token, vesting, REIT waterfall); flag, don't adjudicate.
- **Competitive** — does an equivalent exist in CashScript or the Solidity pattern libraries that we should match or learn from.

**Gate:** a pattern may not be marked 🟢 in `docs/CATALOGUE.md` until every CRITICAL and HIGH finding is either resolved in code or explicitly documented as an accepted limitation with rationale. MEDIUM/LOW may remain open if listed.

This is the established pattern-library posture (audited, documented, conservative) fused with a standing "red-team first, then ship" discipline. The draft → red-team → revised-final loop is the *only* path to stable here.

## Style

- Mirror Silverscript's own idioms (the Mecenas / counter examples in the silverscript repo are the reference for naming and structure).
- Prefer `verification`-mode covenants when validation is simpler than computation; `transition` mode when the new state is a clean function of the old.
- Never hide a value-conservation check. Every spend that moves funds states explicitly what the outputs must be.
- Comment every `require` with the invariant it protects.
- No absolute-time assumptions until the introspection field is pinned against `silverscript-lang/std`.

## Provenance

Patterns frequently mirror prior art (CashScript, Bitcoin covenant research, the Solidity pattern libraries). Credit it in the pattern README. Portrait targets `kaspanet/silverscript` (ISC) — keep the substrate dependency visible and do not vendor Silverscript source into this repo.
