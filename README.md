# Portrait — a covenant language + covenant pattern library for Kaspa

**Kaspa covenant tooling, in one repo.** Two first-class components:

- **The Covenant Patterns Library** (`compliance-patterns/`) — a vetted catalogue of
  reusable Kaspa covenant components, the **OpenZeppelin-equivalent** pattern
  library for Kaspa.
- **Portrait** (`portrait/`) — a high-level language + compiler that turns concise
  `.portrait` source into silverscript covenants, with two verification engines.

Open-source (MIT), stewarded by the **Stichting Kii Foundation** — a Dutch
non-profit that publishes public goods and never bills.

> **Maturity:** pre-production, unaudited, testnet-only. Nothing is on mainnet;
> testnet evidence is perishable (testnets reset by design). The Foundation does
> not itself certify or attest.

---

## Layout

```
portrait/                  ← this repo (github.com/kaspakii/portrait)
├── site/                  the website — landing + the visual covenant wizard
├── compliance-patterns/   THE COVENANT PATTERNS LIBRARY (OpenZeppelin-equivalent)
│                          reusable primitives, the mdBook, a Solidity→Kaspa
│                          migration guide, examples — its own Cargo workspace
└── portrait/              THE PORTRAIT LANGUAGE + COMPILER
                           12-crate Rust workspace, Lens + Composer, docs, and a
                           35-pattern `.portrait` catalogue (portrait/library/)
```

The two components each keep their own Cargo workspace, README, CHANGELOG and
versioning — they build independently. This README is the front door; each
component's own README is its home page.

---

## `compliance-patterns/` — the Covenant Patterns Library

The reusable covenant pattern library for Kaspa: the Kaspa answer to the audited
Solidity pattern libraries. Vetted primitives (ownable, pausable, timelock, a
yield-vault profile and more), each with tests and a threat model; a
Solidity→Kaspa migration guide; and the full mdBook documentation.

→ **Start here: [`compliance-patterns/README.md`](compliance-patterns/README.md)**
(browse the book under `compliance-patterns/book/`, the crates under
`compliance-patterns/crates/`).

## `portrait/` — the Portrait language + toolchain

A small, explicit high-level language that compiles to silverscript covenants.
One Rust workspace, twelve crates — parser, checker, IR, projector, emitter, the
vProg / RISC Zero backend — plus two verification engines:

- **Lens** — an SMT model-proof engine (z3, dep-free) for value-conservation,
  range/overflow, refinement, invariant-preservation and spend properties of a
  covenant **model** (not the emitted `.sil`, not on-chain).
- **Composer** — a session-type checker for multi-party protocols (type-level
  safety, not liveness).

It ships its own `.portrait` pattern catalogue in `portrait/library/` —
**35 covenant sources, 10 cross-layer (vProg)**; 5 vProg settled live on TN10
(perishable testnet evidence), 5 emit-verified.

→ **Start here: [`portrait/README.md`](portrait/README.md)** ·
first 15 minutes: [`portrait/docs/GETTING-STARTED.md`](portrait/docs/GETTING-STARTED.md).

---

## Which do I want?

| You want to… | Go to |
|---|---|
| use a vetted covenant pattern / OpenZeppelin-style component | `compliance-patterns/` (or scaffold one in the site wizard) |
| author a covenant in a high-level language | `portrait/` → `portrait/docs/GETTING-STARTED.md` |
| see the evidence (live TN10 receipts, verification records) | `compliance-patterns/` (`PROVENANCE.json`) |
| contribute | [`CONTRIBUTING.md`](CONTRIBUTING.md) |

---

*The substrate moves weekly — re-verify substrate facts before
activation-sensitive work. Toccata's mainnet activation point is
DAA 474,165,565 (~30 June 2026). Nothing here is deployed on mainnet; all live
evidence is TN10, and testnet evidence is perishable.*
