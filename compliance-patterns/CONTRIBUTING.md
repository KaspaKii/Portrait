# Contributing

Thanks for your interest. This library is early (pre-v0.1.0) and its scope is
deliberately narrow: five covenant compliance patterns for Kaspa, plus shared
plumbing. Proposals for new patterns are welcome — open an issue first so
scope can be discussed before code.

## Ground rules

- **Honest maturity.** Status lines in docs must describe what the code
  actually does today. "Built (pre-production, unaudited)" is a fine thing to
  say; inflated claims are reverted.
- **Tests carry the claim.** Any change to a pattern needs tests; any claim of
  testnet behaviour needs recorded testnet transaction evidence.
- **Terminology.** KCC20, KRC-20, and KTT are distinct (see README). Patterns
  are "KCC20-shape-aligned", never "KCC20-conformant".

## Development

- Stable Rust (MSRV 1.88; workspace builds on current stable). The `wrpc`
  feature pulls the rusty-kaspa dependency tree from git (pinned to `tag = "v2.0.0"`).
- Before pushing: `cargo fmt --all`, then
  `cargo clippy --workspace --all-targets -- -D warnings`,
  then `cargo test --workspace`, then the same clippy + test passes with
  `--all-features`. CI enforces all of them, as does the local gate
  `_harness/ci.sh`. `--all-features` matters because `wrpc` is where the
  real-engine tests live; it builds and tests clean as of 2026-09-01 (the
  earlier upstream `cc-1.2.63` build issue no longer reproduces).
- Keep changes surgical; one pattern per PR where possible.

## Security issues

Never open a public issue for a vulnerability — see [SECURITY.md](SECURITY.md).

## Licence

MIT. By contributing you agree your contribution is licensed under the
repository's MIT licence.
