# site/ — portrait.kaspa-kii.org

The static site for **Portrait** — the unified home for the language,
toolchain, the Covenant Patterns Library, and the visual wizard.
Self-contained: no CDN, no build step, no external requests.

```
site/
├── index.html        the landing page (Portrait · Library · Verification · Evidence)
├── wizard.html       the visual covenant scaffolder
├── wizard.js         the wizard's pure generator (browser + node identical)
├── style.css         layout/components — references ONLY brand tokens
└── brand/tokens.css  the single brand override point (placeholder values)
```

The only JavaScript is the wizard's local generator (`wizard.js`); it makes no
network requests. The landing page is static.

## Deploy

From this directory:

```sh
netlify deploy --prod --dir=. 
```

targeting the site bound to **portrait.kaspa-kii.org**. Local-only until
explicitly deployed — nothing here auto-publishes.

## Brand kit

`brand/tokens.css` is the single source of truth for brand (same token names as
the shared Kii site design system). To apply the real Kii brand kit, **replace
`brand/tokens.css`** — keep the variable names, change only the values. Do not
edit `style.css` for brand changes; it must reference tokens only.

## Content rules (constitution)

Any edit must keep these lines intact:

- Maturity: **pre-production, unaudited, testnet-only; nothing on mainnet**;
  testnet evidence is perishable.
- Lens proves the covenant **model** under stated assumptions (A1–A4) — not the
  emitted `.sil`, not on-chain. Composer proves **type-level** protocol
  safety — not liveness, not a deployed covenant.
- Internal adversarial hardening is **not** external review.
- Counts: **35 covenants / 10 vProg, 5 settled live on TN10** (never "10 live").
- The Stichting Kii Foundation is a non-profit: it does not bill, audit, or
  certify.
- Keep every public sentence inside the honest-maturity and evidence caveats the
  bundled `compliance-patterns/` library documents before deploying.
