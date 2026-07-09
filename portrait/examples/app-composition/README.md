# App-composition examples

These `.portrait` files use the **app-composition grammar** — wiring an existing
covenant into an application:

```portrait
app PersonalVault {
    contract vault = TimeVault {
        owner = param pubkey,
        ...
    }
    lifecycle { ... }
}
```

App-composition sources are **not** covenant sources. They are NOT engraved by
`portrait engrave` (which expects the role/lifecycle/flow/invariant covenant
form). The compiler emits a clear diagnostic if you point `portrait check` /
`portrait engrave` at one:

> app-composition sources (`contract <name> = <Type> { ... }`) are not covenant
> sources; the covenant form uses role/lifecycle/flow/invariant — see
> TimeVault.portrait for the canonical covenant grammar

The canonical, engravable covenant lives at
`../../library/custody/time-vault/TimeVault.portrait`.

- `time-vault.portrait` — idiomatic *use* of the TimeVault covenant in an app.
