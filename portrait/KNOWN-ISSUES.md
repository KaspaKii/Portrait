# Known Issues

> **Maturity: pre-production, unaudited, testnet-only.** No external security
> audit or external review has taken place. Nothing is on mainnet. This file
> records load-bearing correctness caveats that are NOT yet closed by evidence.

## Load-bearing pending items

### KI-1 — Terminal covenant runtime admitting 0 covenant-successor outputs is UNVERIFIED (B3)

**Severity:** load-bearing (a wrong assumption here strands funds).

A B3 **terminal** transition (`finance/Escrow` `release`/`refund`, and any
`... via role.entry terminal;` lifecycle edge) emits a covenant function with
`#[covenant(binding = auth, from = max_ins, to = 1, mode = verification)]` and
**no successor return** — the coin is released to the committed payee via
`pays(...)` and the spending UTXO is consumed, producing **ZERO
covenant-successor outputs**.

`to = 1` is emitted because silverc rejects `to = 0` (`to` must be `>= 1`); `1`
is the minimum literal it accepts. **Whether the silverscript covenant RUNTIME
actually admits a spend that produces 0 covenant-successor outputs under
`binding = auth` + `to = 1` is UNVERIFIED on the mandated engine pin
(`rusty-kaspa` `v2.0.0` = `90dbf07`).**

* **Proven (on the pin):** the isolated pays-bound terminal spend opcodes
  (`OpTxOutputAmount` / `OpTxOutputSpk`) accept the committed payee and reject a
  wrong payee, with a spending tx that has one payee output and zero covenant
  successors (`portrait-emit/tests/output_binding_engine.rs`); and the composed
  terminal `Escrow.sil` compiles under silverc (exit 0).
* **Pending (NOT proven):** the **composed on-engine terminal spend** — i.e. the
  covenant runtime semantics that `binding = auth` + `to = 1` ADMITS a spend with
  0 covenant-successor outputs. Assembling this needs silverscript-lang's covenant
  sig-script / ABI, which pins a floating pre-release engine branch incompatible with
  the mandated `v2.0.0` pin (the same pin bucket that blocks the B2/B1 composed
  on-engine spends).

**Risk if the assumption is wrong:** if the runtime REQUIRES at least one
covenant successor, a terminal spend producing none is rejected and the terminal
UTXO becomes **permanently unspendable (stuck funds)**.

**Directive:** do **NOT** deploy a terminal covenant to a value-bearing UTXO
until the composed on-engine terminal spend is proven on the mandated pin.

Referenced from: `portrait/crates/portrait-emit/src/lib.rs`
(`emit_terminal_transition`) and the `finance/Escrow` terminal-spend row in
`library/ENFORCEMENT.md`.

### KI-2 — `supply_change` is a CHECKED-MODEL capability, NOT on-chain minted supply (A2-full)

**Severity:** load-bearing (a wrong reading over-claims what a mint enforces).

The `#[covenant(mode = transition, supply_change = <field>)]` capability
(`finance/MintableToken`) is checked by `portrait-sema`: the named authority must
be a COMMITTED key, guaranteed to sign on every satisfying path (a sound,
commutative per-key check — never satisfiable through a `||` branch or a negated
arm), and the entry must release NO coin (no `pays(...)`, not terminal). Given
those checks, the annotation is an AUTHOR-DECLARED capability that WAIVES the entry
from value-conservation checking (a supply change does not conserve) — the
authority-signs check is the guarantee that stands in for conservation on this
path. **What this DOES NOT mean:** it does NOT — and a UTXO covenant CANNOT —
inflate real L1 coin. The `supply` field is the covenant's OWN committed integer,
not a mint of KAS; incrementing it moves committed model state under a signature,
it does not create spendable coin. The guarantee is "the recorded supply counter
only moves under the committed authority's signature", NOT "new coin was created".

* **Enforced (checked-model):** the authority is committed and guaranteed to sign
  (sound per-key analysis); the entry releases no coin (no `pays`, not terminal),
  so `payout_bound` excludes it soundly; the successor carries the new supply;
  conservation is annotation-waived (not name-waived — the old `mint*`/`burn*`
  heuristic is retired).
* **NOT enforced:** any binding between the model `supply` and real coin. Actual
  coin movement is WALLET-ASSUMED, exactly as for every other value leg in the
  library (see `library/ENFORCEMENT.md`).

**Risk if the boundary is misread:** treating a `supply_change` entry as an
on-chain issuance guarantee. It is a model-level authorisation of a counter, not
consensus-enforced coin creation.

Referenced from: the `finance/MintableToken` rows and the `supply_change` bullet
in `library/ENFORCEMENT.md`, and `docs/BUILD_SPEC.md` §3.

### KI-3 — NON-TERMINAL `pays` successor/payee output-index co-existence is UNVERIFIED (B2)

**Severity:** load-bearing (a wrong assumption here makes the charge path
unspendable, stranding the prepaid balance).

`finance/Subscription`'s `charge` is the catalogue's **first NON-TERMINAL
`pays(...)`**: a single spend that carries **BOTH** a covenant successor **AND** a
separately bound payee output. The emitted covenant is a `binding = cov`,
`mode = transition` function with `to = 1` that returns a successor state **and**
carries

```silverscript
require(tx.outputs[1].value == prev_states[0].amount_per_period);
require(tx.outputs[1].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_states[0].provider)));
```

silverc's `to` counts covenant **SUCCESSOR** outputs, not total transaction
outputs, so `to = 1` plus one additional payee output is **well-formed at compile
time** (silverc exit 0). **But WHICH output index the covenant successor occupies
at RUNTIME is UNVERIFIED on the mandated engine pin (`rusty-kaspa` `v2.0.0` =
`90dbf07`).** The `pays` clause binds index **1** on the assumption that the
successor takes index 0; if the runtime places the successor elsewhere, the two
bindings collide.

* **Proven (on the pin):** the output-binding opcodes themselves
  (`OpTxOutputAmount` / `OpTxOutputSpk`) accept the committed amount+payee and
  reject a wrong amount / wrong payee
  (`portrait-emit/tests/output_binding_engine.rs`); and the composed
  `Subscription.sil` — successor return AND bound payee output together — compiles
  under silverc (exit 0,
  `silverc_accepts_the_composed_subscription_sil_with_the_non_terminal_pays_binding`).
* **Pending (NOT proven):** the **composed on-engine spend** — i.e. the covenant
  runtime semantics of a `binding = cov` transition producing one covenant
  successor **plus** one bound payee output, and the index the successor actually
  occupies. Assembling this needs silverscript-lang's covenant sig-script / ABI,
  which pins a floating pre-release engine branch incompatible with the mandated
  `v2.0.0` pin (the same pin bucket as KI-1 and the B2/B1 composed spends).

**Risk if the assumption is wrong:** if the covenant successor occupies output
index 1, the `pays` require and the successor binding contradict each other, every
`charge` spend is rejected, and the subscription UTXO becomes **permanently
unspendable (stuck funds)** — there is no second path out of `Subscription`.

**Directive:** do **NOT** deploy a non-terminal `pays` covenant to a value-bearing
UTXO until the composed on-engine spend, and the successor's runtime output index,
are proven on the mandated pin. The enforcement claim for this row is
*SCRIPT-ENFORCED at emit + silverc exit 0; composed on-engine spend PENDING* — never
plain SCRIPT-ENFORCED.

Referenced from: the `finance/Subscription` `charge` payout row in
`library/ENFORCEMENT.md`, the header note in
`library/finance/subscription/Subscription.portrait`, and
`portrait/crates/portrait-emit/tests/output_binding_engine.rs`.
