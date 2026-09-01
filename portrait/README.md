<div align="center">

# Portrait

**A library and composition layer for secure multi-contract covenant apps on Kaspa.**

*What the established pattern libraries are to Solidity, Portrait aims to be to Silverscript — but adapted for a world with no inheritance, no global state, and where the unit of composition is a coordinated set of UTXO covenants linked by covenant IDs.*

[![status](https://img.shields.io/badge/status-experimental-orange)]()
[![target](https://img.shields.io/badge/target-silverscript%20%5E0.1.0-blue)]()
[![network](https://img.shields.io/badge/network-Kaspa%20TN10-purple)]()
[![license](https://img.shields.io/badge/license-MIT-green)]()

A project of the **Stichting Kii Foundation**, a Dutch non-profit.

</div>

---

## Why Portrait exists

Silverscript ([kaspanet/silverscript](https://github.com/kaspanet/silverscript)) is Kaspa's first high-level covenant language. It compiles **directly to native Kaspa Script** — no VM, no IR — and its covenant-declaration macros (`#[covenant.singleton(...)]`, `#[covenant(from=N, to=M)]`) give you *security-by-construction* for a **single contract's** state transition.

But Silverscript is deliberately single-contract: *"Every Silverscript program defines a single contract."* Real applications are not. A decentralised chess league, a tokenised-property rent waterfall, a frequent-batch auction, a vault with a governance overlay — each is **many small contracts whose state must persist and evolve together** across transactions.

The rail that makes that possible is **KIP-20 Covenant IDs**: a consensus-tracked 32-byte lineage tag plus an output *covenant binding* (`authorizing_input` + `covenant_id`) that carries identity across transitions *without recursive ancestor proofs*. Powerful — and exactly the kind of low-level wiring that is easy to get subtly, catastrophically wrong by hand.

That gap is Portrait's reason to exist:

> **Silverscript gives you one safe contract. Portrait gives you a safe *application* — and a deep catalogue of reusable, threat-modelled pieces to build it from.**

## The two layers

| Layer | What it is | Analogy |
|---|---|---|
| **Portrait Library** (`library/`) | A curated, threat-modelled catalogue of reusable covenant components — vaults, escrows, auctions, registries, token standards, access guards — each shipping as real Silverscript plus a Portrait wrapper, golden tests, and a threat model. | the audited Solidity pattern libraries |
| **Portrait Composer** (`portrait/`) | A thin high-level surface (`.portrait`) that composes several Silverscript contracts into one app: it declares each contract, checks its lifecycle against the contract's covenant transitions, wires the covenant-ID lineage, and emits a typed transaction template so off-chain signers build correct spends. | Hardhat / the missing "app" layer |

You can use the Library alone (drop a vetted `.sil` component into your project) or let the Composer assemble a multi-contract app end-to-end. A mature dApp uses both, and often the vProgs/ZK layer above.

## A taste

A two-key, time-delayed vault — the canonical custody pattern — as a Library component:

```silverscript
// library/custody/time-vault/TimeVault.sil
pragma silverscript ^0.1.0;

contract TimeVault(pubkey owner, pubkey recovery, int delay) {
    state { int pending; bytes32 beneficiary; int amount; }

    #[covenant.singleton(mode = transition)]
    entrypoint function schedule(State prev, sig ownerSig, bytes32 beneficiary, int amount)
        : (State next)
    {
        require(prev.pending == 0);
        require(checkSig(ownerSig, owner));
        require(amount > 0);
        return State { pending: 1, beneficiary: beneficiary, amount: amount };
    }
    // settle (after delay) and cancel (recovery clawback) — see the component README.
}
```

…and composed into an application by the Composer:

```portrait
// examples — using the component
pragma portrait ^0.1.0;
use custody::TimeVault;

app PersonalVault {
    contract vault = TimeVault { owner = param pubkey, recovery = param pubkey, delay = 1440 }

    lifecycle {
        idle    -> pending  via vault.schedule
        pending -> settled  via vault.settle      // terminal
        pending -> idle     via vault.cancel
    }
}
```

The `lifecycle` block is checked against the covenant transitions in the `.sil` — Portrait refuses to compile if an entrypoint can reach a state you never declared. That single guard kills a whole class of stuck-funds and unexpected-transition bugs.

## Repository layout

```
portrait/
├── README.md            ← you are here
├── LICENSE              ← MIT
├── CONTRIBUTING.md      ← the security / threat-model standard every pattern must meet
├── docs/
│   ├── LANGUAGE.md      ← the Portrait Composer language + lowering model
│   ├── ARCHITECTURE.md  ← master architecture & component atlas
│   ├── BUILD_SPEC.md    ← implementation-grade build spec for the suite
│   └── CATALOGUE.md     ← the full pattern catalogue (the magnum-opus index)
├── library/            ← the reusable components (the audited-pattern-catalogue analogue)
│   ├── attestation/  custody/  finance/  governance/  state/  vprog/
│   └── …each pattern: <Name>.sil + <name>.portrait + README.md + THREAT_MODEL
├── examples/           ← full multi-contract apps (chess league, REIT waterfall, …)
└── portrait/           ← the Composer toolchain (Rust workspace; golden tests under crates/portrait-cli/tests)
```

## Status

Experimental and **TN10-only**, tracking silverscript `^0.1.0`. Nothing here is audited or mainnet-safe yet. See `docs/CATALOGUE.md` for the per-pattern status board and `CONTRIBUTING.md` for the bar a pattern must clear before it is marked stable.

## Relationship to Silverscript

Portrait **depends on and targets** Silverscript (`kaspanet/silverscript`, ISC, by Ori Newman and contributors) and the KIP-10/16/17/20/21 primitives landing in **Toccata**. Silverscript is community infrastructure and is not Portrait's IP; Portrait's contribution is the library and composition layer built on top, with the substrate dependency disclosed throughout.

## License

MIT © 2026 Stichting Kii Foundation — see [`LICENSE`](LICENSE). (Open to ISC for parity with Silverscript.)
