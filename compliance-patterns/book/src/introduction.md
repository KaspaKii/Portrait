# Introduction

**`kaspa-compliance-patterns`** is a small, honest library of covenant
compliance patterns for Kaspa, written in Rust against the released Toccata
engine (`rusty-kaspa` tag `v2.0.0` = commit `90dbf07`, mainnet activation at
DAA 474,165,565, ~30 June 2026).

Each pattern is a worked, testable building block — not a product, not a
standard, not audited. The library is a Foundation public good, stewarded
by **Stichting Kii Foundation** (Dutch non-profit, incorporated 2026-06-21);
it carries no token and makes no investment claim.

## What you'll find here

- **Five patterns**, one Rust crate each, plus shared plumbing in
  `kcp-common`. Three of the four state-continuity covenants are live on
  testnet-10 (KCP-RE/SL/TR/KTT-003).
- A **10-minute quick-start** that builds a vault covenant from a fresh
  checkout, signs a synthetic spend with throwaway keys, and runs the
  real `rusty-kaspa` script engine over it offline — no node, no funds.
- Per-pattern documentation (status, scope, examples, security caveats).
- An **on-chain evidence index** with every reproducible testnet-10
  transaction the library has produced.
- A **launch note** + **known-issues catalogue** stating
  what the library is and is not.

## Status

**Pre-production · unaudited · testnet-first.** On-chain evidence is
perishable by design — testnets reset. See the [known issues](./known-issues.md)
for the full caveats catalogue.

## What this site is

A rendered view of the in-repo Markdown. The source of truth remains the
repository itself; this site is a navigation aid for readers who prefer
docs-site browsing to raw repo browsing.
