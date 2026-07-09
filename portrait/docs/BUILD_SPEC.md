# Portrait — Build Specification (v1)

> **Maturity: pre-production, unaudited, testnet-only.** No external security
> audit or external review has taken place — internal adversarial hardening is
> not external review. Nothing is on mainnet; live evidence is perishable Kaspa
> testnet-10 (TN10) evidence (the testnet resets). Where verification is
> described: Lens proves properties of the covenant **model** under stated
> assumptions — not the emitted `.sil`, and nothing on-chain; composition
> checks are **type-level** safety — not liveness, and not a deployed covenant.

*Implementation-grade. This is the document you hand an engineer to start cutting code. It pins the DSL, the compiler internals, the emission, the Library suite, and the test harness.*

---

## 0. Operating constraints

- **Language.** Compiler core in **Rust** (matches silverscript; integration path to `silverscript-lang`; the team's stack). Override at will.
- **Trusted oracle.** Never re-implement what silverscript already guarantees. The pipeline emits `.sil`, then shells out to `silverscript check` / `silverscript compile` as the source of truth. Portrait's job is *composition, checking, projection, and attestation* — not re-deriving Kaspa Script.

---

## 1. Suite map

| Product | Crate / package | Role | Milestone |
|---|---|---|---|
| `portrait` CLI | `portrait-cli` | The toolchain binary (implemented verbs: `check`/`prove`/`compose`/`validate-translation`/`ship`, …; see §8) | M0 |
| DSL frontend | `portrait-syntax` | Lexer, parser, AST | M0 |
| Core IR + checks | `portrait-ir`, `portrait-sema` | Cartoon IR, linearity, lifecycle, projection | M0–M1 |
| Projection | `portrait-project` | Pounce — app → per-role covenant models | M1 |
| Emission | `portrait-emit` | Engraver — covenant model → `.sil` | M0–M1 |
| Plan | `portrait-plan` | Provenance (covenant-ID lineage) + Easel (tx templates) | M1 |
| Verify | `portrait-verify`, `portrait-lens` | Lens — SMT bridge + Hallmark verification report | M2 |
| Library | `library/` (`.portrait` + `.sil`) | The verified components | M1–M2 |
| Atelier | `portrait-atelier` | vProgs backend (account model + ZK guest) | — |

---

## 2. The compiler workspace (Rust)

```
portrait/                      # compiler workspace root (Cargo workspace) — 12 crates
├── Cargo.toml                 # [workspace] members
└── crates/
    ├── portrait-syntax/       # tokens, parser, AST
    ├── portrait-ir/           # Cartoon IR types
    ├── portrait-sema/         # type stack: linearity, lifecycle/session, capability, refinement
    ├── portrait-project/      # projection scaffolding
    ├── portrait-pounce/       # Pounce: projection engine
    ├── portrait-emit/         # Engraver: silverscript emission
    ├── portrait-plan/         # Provenance (lineage) + Easel (tx templates)
    ├── portrait-compose/      # Composer front-end (type-level composition checks)
    ├── portrait-verify/       # verification driver + Hallmark report
    ├── portrait-lens/         # Lens: SMT (Z3) bridge
    ├── portrait-atelier/      # Atelier: vProgs backend (RISC Zero guest emission)
    └── portrait-cli/          # the `portrait` binary
```

**Data flow:** `source → portrait-syntax::parse → AST → portrait-sema::check → typed AST → lower → portrait-ir::Cartoon → portrait-project::project → Vec<CovenantModel> → portrait-emit::emit → Vec<SilFile> → (silverscript check) → portrait-plan → manifest + templates → portrait-verify → verification report → bundle`.

Keep the scaffold **dependency-light** so it `cargo check`s offline. Add `clap`, `serde`, `z3` only as the relevant milestone needs them.

---

## 3. The DSL, pinned

### 3.1 Lexical

- **Keywords:** `pragma use app role state param flow lifecycle invariant requires ensures via terminal choose or par and repeat when offload proves resource linear cap`.
- **Types:** `int bool pubkey sig bytes32 coin set map` + user `Named`.
- **Literals:** integers, `true/false`, byte strings `0x…`, value literals with units (`1 kas`, `100 sompi`).
- **Comments:** `// line`. **Idents:** `[A-Za-z_][A-Za-z0-9_]*`. **Pragma:** `pragma portrait ^0.1.0;`.

### 3.2 Grammar (EBNF, v1)

```
program    = pragma , { use } , app ;
use        = "use" , path , ";" ;
app        = "app" , Ident , "{" , { role } , [ lifecycle | flow ] , { invariant } , "}" ;
role       = "role" , Ident , [ "=" , compref ] , "{" , { param } , [ state ] , { entry } , "}" ;
compref    = path , "{" , { Ident , "=" , (expr | "param" , type) , "," } , "}" ;
state      = "state" , "{" , { type , Ident , ";" } , "}" ;
entry      = [ attr ] , "entrypoint" , "function" , Ident , "(" , [ args ] , ")" , [ ":" , "(" , type , Ident , ")" ] , block ;
attr       = "#[" , "covenant" , "." , ("singleton"|"") , "(" , "mode" , "=" , ("transition"|"verification") , ")" , "]" ;
lifecycle  = "lifecycle" , "{" , { edge } , "}" ;
edge       = Ident , "->" , Ident , "via" , Ident , "." , Ident , [ "terminal" ] , ;
flow       = "flow" , "{" , step , { ";" , step } , "}" ;
step       = move | "choose" block { "or" block } | "par" block { "and" block } | "repeat" "(" Int ")" block ;
invariant  = "invariant" , ( "value_conserved" | "no_undeclared_state" | Ident ) , ";" ;
```

**v1 decision:** single- and few-contract apps use the explicit **`lifecycle`** block (states + which entrypoint drives each edge). The richer **`flow`** choreography (parallel, choice, recursion across *multiple* roles) is a later generalisation; `lifecycle` is the special case the compiler can also *derive from* a `flow`. Ship `lifecycle` first.

### 3.3 Example — `Counter` (the M0 target)

```portrait
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) { return value + delta; }
  }
  lifecycle { live -> live via counter.bump; }
  invariant no_undeclared_state;
}
```

### 3.4 Example — 2-role `Escrow`

Buyer, Seller, Arbiter as three coordinated covenants linked by a covenant-ID; `flow` with `choose { release } or { refund } or { dispute }`. Used to exercise projection + lineage end to end.

---

## 4. Cartoon IR (concrete types)

These are committed as real Rust in `crates/portrait-ir`. The shape:

```rust
pub struct Cartoon { pub app: String, pub roles: Vec<RoleGraph>, pub invariants: Vec<Inv> }

pub struct RoleGraph {
    pub role: String,
    pub params: Vec<(String, IrType)>,
    pub states: Vec<StateNode>,
    pub transitions: Vec<Transition>,
    pub channels: Vec<Channel>,   // lineage edges to other roles
}

pub struct StateNode { pub label: String, pub fields: Vec<(String, IrType)> }

pub struct Transition {
    pub entry: String,
    pub from: String,
    pub to: Option<String>,            // None = terminal (leaves covenant)
    pub mode: CovenantMode,            // Transition | Verification | NonCovenant
    pub guards: Vec<Guard>,            // assembled from session + linearity + capability + requires
    pub delta: ResourceDelta,         // resources in/out (Ledger)
    pub capability: Option<String>,
}

pub enum Guard {
    Sig { key: String },               // checkSig(sig, key)
    AgeAtLeast(i64),                   // this.age >= n
    Eq(Expr, Expr),                    // prev_state.x == y
    OutputPays { index: usize, to: Commit, amount: Expr },
    Custom(Expr),
}
```

The IR is **more abstract than silverscript** (it knows roles, lineage, capabilities, resources) and lowers down. Everything the backends need is here; nothing role-/session-level survives into the `.sil`.

---

## 5. Projection (Pounce) — the algorithm

Input: the typed app (roles, `lifecycle`/`flow`, invariants). Output: one `CovenantModel` per role.

1. **States.** For each role, collect the declared state labels reachable in its `lifecycle`/projected `flow`.
2. **Ownership check.** Every edge's `via role.entry` must name a real entrypoint of exactly one role; reject otherwise.
3. **Entrypoint synthesis.** For each edge into a role, synthesise one entrypoint.
4. **Mode selection.** `Transition` if the entrypoint returns a new same-role state; `Verification` if it terminates (leaves the covenant) and instead asserts outputs; `NonCovenant` for plain spends.
5. **Guard assembly.** Concatenate, in order: session preconditions (must be in `from` state), linearity/value-conservation (from `ResourceDelta`), capability checks (`Seal`), then developer `requires`.
6. **Lineage channels.** For any edge whose effect crosses to another role (e.g. `Scoreboard.record` consuming `Game.result`), emit a `Channel` → covenant-ID binding (Provenance).
7. **Well-formedness.** Reject if any entrypoint can reach a state not in the declared set (`no_undeclared_state`), if any linear resource is dropped or duplicated, or if a `terminal` target has an outgoing covenant edge.

**v1 (single-role)** projection is near-identity — one covenant — and the value is the *checking* (steps 2, 4, 5, 7). **Multi-role** projection is where real projection earns its keep.

---

## 6. Emission (Engraver) — templates

Construct → silverscript mapping:

| Portrait | silverscript |
|---|---|
| `role r { param T p; state{…} }` | `contract R(T p) { state { … } }` |
| transition entrypoint (mode=transition) | `#[covenant.singleton(mode = transition)] entrypoint function f(State prev, …) : (State next) { … return State{…}; }` |
| verification/terminal entrypoint | `#[covenant.singleton(mode = verification)] entrypoint function f(State prev, …) { require(…); }` |
| `Guard::Sig{key}` | `require(checkSig(sig, key));` |
| `Guard::AgeAtLeast(n)` | `require(this.age >= n);` |
| `Guard::OutputPays{i,to,amt}` | `require(blake2b(tx.outputs[i].script) == to); require(tx.outputs[i].value == amt);` |
| `ResourceDelta` conservation | `require(tx.outputs[…].value (>=|==) tx.inputs[…].value);` |
| `set<T>` / `map<K,V>` | not emitted — collection lowering is not in the current build |

The emitted `Counter.sil` for §3.3 is exactly the silverscript `counter` covenant example. Every emitted file is piped through `silverscript check`; a failing check fails the build.

---

## 7. Provenance + Easel

- **Provenance** computes covenant IDs (KIP-20 BLAKE2b), the genesis tx, and per-output `covenant_binding` (`authorizing_input` + `covenant_id`). Output: `app.manifest.json` (IDs, genesis shape, lineage edges) — what a wallet/indexer needs to follow the app.
- **Easel** emits, per entrypoint, a typed template: inputs to select, outputs to build, witnesses to collect (sigs, preimages, accumulator/ZK witnesses). Output: a small TS/Rust SDK so off-chain signers build accepted spends.

---

## 8. CLI surface

> **Status note (2026-07):** the implemented CLI dispatch is
> `check | build | engrave | atelier-build | ship | verify | prove | compose |
> validate-translation | new | version` (plus `test`/`publish` placeholders that
> only print their milestone). `fmt` is **not implemented**. The table below is
> the original v1 plan, kept for context.

```
portrait new <name>           scaffold a package
portrait check <file>         parse + type/lifecycle/linearity check (no emission)
portrait build <file>         → .sil (+ silverscript check) + manifest + templates + verification report
portrait test                 (planned) run golden + debugger accept/reject vectors
portrait fmt                  (planned) format .portrait
portrait publish <pkg>        (planned) emit a publishable component entry (component + report + threat model + hashes)
```

`build` is the spine; everything else is a slice of it.

---

## 9. The Library — the first verified suite

> **Status note (2026-07):** the shipped covenant-patterns library now stands at
> **35 covenant sources / 10 cross-layer (vProg) patterns**, of which **5 are
> settled live on TN10** (perishable testnet evidence) and 5 are emit-verified
> only. The list below is the original suite order, kept for context.

Ship these first, in order:

1. `access/SingleKey`, `access/DualKey`, `access/MultiSig`
2. `custody/TimeVault` *(drafted)*
3. `token/TrustedToken` (KTT) — the trusted-token standard
4. `token/PaymentSplitter`
5. `commit/HashCommitment`

**Component package format:** a directory with `<Name>.portrait`, the emitted `<Name>.sil`, `tests/` (golden + debugger vectors), `THREAT_MODEL.md`, `README.md`, and a generated `hallmark.json`. Publishable as one unit.

---

## 10. Hallmark verification report

**Hallmark verification-report schema** (`hallmark.json`):

```json
{
  "component": "custody/TimeVault",
  "version": "0.1.0",
  "source_hash": "blake2b:…",
  "sil_hash": "blake2b:…",
  "silverscript_version": "0.1.0",
  "portrait_version": "0.1.0",
  "invariants": [
    {"name": "value_conserved", "method": "linear-type", "status": "proved"},
    {"name": "no_undeclared_state", "method": "lifecycle-check", "status": "proved"},
    {"name": "settle_pays_committed_beneficiary", "method": "smt", "status": "proved"}
  ],
  "threat_model": "THREAT_MODEL.md",
  "issued": "2026-…", "issuer": "kii"
}
```

Verification-report scope starts *absurdly narrow* — only fully-verified component classes (the small, totally-checkable surfaces: vaults, splitters, commitments). A Hallmark report is a machine-checked record over the model — it is not an audit and nothing is certified.

---

## 11. Test / golden harness

Per component: (a) **snapshot** — `.portrait` → expected `.sil` (committed golden); (b) **soundness** — emitted `.sil` passes `silverscript check`; (c) **behaviour** — `cli-debugger` argument sets exercising every entrypoint's *accept* and *reject* paths. CI fails on any snapshot drift, check failure, or vector regression. This harness is also what produces the evidence behind a Hallmark verification report.
