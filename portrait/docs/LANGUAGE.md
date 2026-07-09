# The Portrait Composer Language

> **Maturity: pre-production, unaudited, testnet-only.** No external security
> audit or external review has taken place — internal adversarial hardening is
> not external review. Nothing is on mainnet; live evidence is perishable Kaspa
> testnet-10 (TN10) evidence (the testnet resets). Composer checks are
> **type-level** safety — they do not prove liveness on the DAG and do not mean
> a covenant is deployed.

`.portrait` is a thin orchestration surface over Silverscript. It does **not** replace Silverscript — you still write transition logic in `.sil` (or pull it from `library/`). What Portrait adds is everything that lives *between and around* contracts: app composition, lifecycle verification, covenant-ID lineage wiring, and off-chain transaction templates.

Status: **design draft** for the full multi-role `flow` surface. The syntax the compiler *implements today* is the single-role `role`/`lifecycle`/`entrypoint` form shown in [GETTING-STARTED.md](GETTING-STARTED.md) and pinned in `BUILD_SPEC.md` §3; the richer surface below is the planned generalisation and remains provisional.

---

## 1. The problem it solves

Silverscript's own words: *"Every Silverscript program defines a single contract."* So the moment your app needs two contracts that share evolving state — a game and a scoreboard, a vault and its governance, a pool and its fee sink — you are hand-wiring:

- **Covenant-ID genesis & inheritance** (KIP-20): computing the id, setting each output's `covenant_binding` (`authorizing_input` + `covenant_id`), and proving lineage on every spend.
- **Cross-contract introspection**: contract A asserting something about contract B's output in the same transaction.
- **The off-chain half**: building the actual transaction with the right inputs, outputs, and witnesses so the on-chain covenant accepts it.

Hand-wiring all three, per transition, is where covenant apps break. Portrait generates them from a declaration.

## 2. Top-level shape

```portrait
pragma portrait ^0.1.0;

use custody::TimeVault;     // pull a component from the library
use access::MultiSig;

app <Name> {
    // 1. Declare the contracts the app is made of.
    contract <id> = <Component> { <param> = <value | param T>, ... }
    contract <id> { /* inline silverscript-style body */ }

    // 2. Declare the legal lifecycle (optional but recommended).
    lifecycle { <state> -> <state> via <contract>.<entrypoint> ; ... }

    // 3. Declare cross-contract links (multi-contract apps only).
    link <contractA>.<entrypoint> requires <contractB> at <relation>
}
```

A single-contract `app` is legal and still buys you lifecycle checking + covenant-ID genesis + tx templates. Multi-contract apps add `link`.

## 3. Lowering

For each `app`, the Composer emits:

1. **One `.sil` per contract** — either copied from the library component (with params bound) or generated from the inline body. These are compiled by `silverscript` unchanged, so the security-by-construction guarantees of the covenant macros are preserved, not re-implemented.
2. **A covenant manifest** (`<app>.manifest.json`) — the covenant IDs, the genesis transaction shape, and which output of which entrypoint inherits which id. This is what an indexer or wallet needs to follow the app's lineage.
3. **A transaction template per entrypoint** — a typed description (inputs to select, outputs to build, witnesses to collect) so an off-chain signer constructs a spend the covenant will accept. Generated, not hand-rolled.

Portrait deliberately emits **no new on-chain bytecode of its own** beyond the Silverscript it composes. The trust surface on L1 is exactly the Silverscript compiler's output.

## 4. Lifecycle checking

The `lifecycle` block is a state machine. The Composer cross-checks it against the covenant transitions actually present in each contract:

- every `via <c>.<entrypoint>` must correspond to a real covenant entrypoint in `c`;
- the post-state a transition can produce (from its returned `State` in `transition` mode, or its asserted outputs in `verification` mode) must be a state you declared as reachable;
- a `terminal` target must have no outgoing covenant edge (funds leave the lineage).

If any covenant entrypoint can drive the UTXO into a state not in the diagram, compilation **fails**. This turns "did I forget a transition?" — the single most common stuck-funds bug in hand-written covenants — into a compile error.

## 5. Timelocks

Kaspa time is **relative UTXO age** via `this.age` (confirmed idiom; see Silverscript's Mecenas example), and partial absolute height via KIP-10 introspection. Portrait surfaces `delay = N` as a relative-age requirement and will not silently fabricate absolute-height semantics. Absolute-time patterns are gated until the introspection field is pinned. See `library/custody/time-vault/` for a worked relative-delay vault.

## 6. Types (pass-through)

Portrait does not invent a type system; it passes Silverscript types through: `int`, `bool`, `pubkey`, `sig`, `bytes32`, and arrays (`State[]`, `sig[]`). Component parameters are typed with these. The `param T` form marks a value supplied at deployment rather than baked in source.

## 7. Open design questions

- **File extension**: `.portrait` for now; `.prt` short-form under consideration.
- **Inline vs library-only contracts**: whether the Composer should allow arbitrary inline Silverscript or only parameterised library components (the latter is safer and more auditable).
- **Cross-contract atomicity**: the exact `link` semantics for "A's spend is only valid if B transitions in the same tx" — needs to be expressed in terms of Cov-binding introspection (`OpCov*`) and confirmed against `silverscript-lang/std`.
- **Whether the Composer is a separate binary or a `silverscript` subcommand.** Leaning separate (`portrait build`) that shells out to `silverscript compile`.

These are tracked as the Composer toolchain is implemented under `portrait/`.
