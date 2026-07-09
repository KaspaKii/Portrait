# Contributing

This is a monorepo with two first-class components. Contributions go to the
component they touch:

| Area | Path | Examples |
|---|---|---|
| **Covenant Patterns Library** | `compliance-patterns/` | a new covenant pattern, a fix to a primitive, the migration guide, the book |
| **Portrait language + compiler** | `portrait/` | compiler bugs, language features, Lens/Composer, the `.portrait` catalogue |
| **Website** | `site/` | the landing page, the covenant wizard |

Each component keeps its own Cargo workspace, README, CHANGELOG and gates. Build
and test **within the component you changed** (e.g. `cd compliance-patterns &&
cargo test`, or `cd portrait/portrait && cargo test`).

## Issues

Please use the issue templates — they route your report and apply an area label
automatically. Maintainers use this label scheme (create these on the repo if
they don't exist yet):

- `area:library` · `area:compiler` · `area:docs` · `area:site`
- `type:pattern-request` · `type:bug` · `type:enhancement` · `type:question`
- `good-first-issue` · `help-wanted`

## Pull requests

- Keep the change inside one component where possible; note cross-cutting changes.
- Run that component's gates (fmt + clippy + tests) before opening the PR.
- New covenant patterns must ship with tests and a threat model, and pass
  `portrait check` — see `compliance-patterns/CONTRIBUTING.md` for the pattern bar.

## Honest-maturity rule

This project is **pre-production, unaudited, testnet-only**. Do not add language
that implies audit, certification, mainnet deployment, or a security guarantee.
The Stichting Kii Foundation is a non-profit; it publishes public goods and does
not bill, audit, or certify. Every external-facing figure must trace to evidence
in the repo.

By contributing you agree your contribution is licensed under the repository's
MIT licence.
