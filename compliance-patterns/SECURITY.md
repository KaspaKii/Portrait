# Security Policy

## Status

This library is **v0 — unaudited — testnet first**. There are no supported
release versions yet. Do not hold mainnet value with these patterns.

## Reporting a vulnerability

Please report vulnerabilities privately:

- Preferred: GitHub private vulnerability reporting ("Report a vulnerability"
  under the repository's Security tab), once the repository is public.
- Otherwise: email `contact@kaspa-kii.org` with subject line
  `[kaspa-compliance-patterns security]`.

Please do not open public issues for suspected vulnerabilities.

We aim to acknowledge reports within 7 days. There is currently no bug
bounty programme.

## Scope notes

- Findings against the covenant patterns themselves (invariant bypass, state
  escalation, lineage forgery, value-conservation breaks) are highest
  priority.
- Upstream issues in SilverScript or rusty-kaspa should be reported upstream;
  we will help route them if you are unsure where a boundary lies.
