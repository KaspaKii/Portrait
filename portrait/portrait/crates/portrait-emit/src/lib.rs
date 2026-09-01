//! Engraver: lower covenant models to silverscript (BUILD_SPEC §6).

use portrait_ir::{CovenantModel, Guard, Mode, SilFile, Transition};
use portrait_syntax::{AfterDeadline, BinOp, Expr, ReturnExpr, Type};
use std::collections::HashSet;

/// Emit one .sil file per covenant model.
///
/// Returns `Err` (fail-loud) if any covenant transition body carries a
/// `Stmt::Raw` guard — an unrecognised/unlowerable guard form (e.g. the `@`
/// age syntax) that parsed to `Stmt::Raw`. Such a statement would otherwise be
/// silently dropped at emit, producing a covenant that LOOKS gated but enforces
/// nothing — a soundness/honesty defect. Naming the offending statement makes
/// the gap impossible to ship unnoticed.
///
/// Also returns `Err` if a state field has no constructor param of the same name
/// (or one of a different type), or if a param name is declared twice — genesis
/// initialisers bind BY NAME, and a field with no initialiser is a compile error
/// rather than a silent `0`. See [`validate_genesis_binding`].
///
/// And returns `Err` for a non-terminal `mode = verification` entrypoint that
/// carries guards: those bodies are not lowered, so the guards would be dropped
/// in silence (L4) — the same "looks gated, enforces nothing" defect as `Raw`.
/// Input index at which the covenant UTXO is ASSUMED to appear when the emitter
/// injects the vProg proof-covenant-id binding
/// (`require(proof_cov_id == OpInputCovenantId(<idx>))`). Default `0` — the
/// covenant is the FIRST input of the spending transaction. A spend where the
/// covenant UTXO is not input 0 must emit a different index (the literal is
/// parametric via [`emit_model`]); the assumption is documented at the emission
/// site and in `library/ENFORCEMENT.md`.
const DEFAULT_PROOF_COV_INPUT_INDEX: usize = 0;

pub fn emit(models: &[CovenantModel]) -> Result<Vec<SilFile>, String> {
    models
        .iter()
        .map(|m| emit_model(m, DEFAULT_PROOF_COV_INPUT_INDEX))
        .collect()
}

/// Prefix for the silverscript constructor parameter that carries a state
/// field's genesis value.
///
/// The author writes the param under the state field's OWN name (`param int
/// balance;` initialises `state { int balance; }` — binding is by name), but the
/// two cannot share an identifier in the emitted contract: silverscript's public
/// `ContractAst::resolve_contract_state_values` — the API a deployer uses to
/// compute an instance's concrete genesis state — rejects a field whose name is
/// already bound by a constructor param with `duplicate contract field name`.
/// Upstream's own convention is a distinct initialiser identifier (their fixture
/// is `contract ResolveState(int initAmount) { int amount = initAmount; }`), so
/// the emitted param is `init_<field>`. `silverc`'s compile path does not call
/// that API, which is why the colliding form still compiled.
const GENESIS_PARAM_PREFIX: &str = "init_";

/// Identifiers the emitter injects into every contract signature. A user-declared
/// param emitted under one of these would SHADOW the covenant's own input/output
/// bound, so they are reserved (`portrait_sema` rejects them at the source level;
/// this list keeps the emitter's own disambiguation honest).
const INJECTED_CONTRACT_PARAMS: &[&str] = &["max_ins", "max_outs"];

/// The silverscript identifier each of `model.params` is emitted under,
/// positionally aligned with `model.params` so `emit_ctor`'s argument order is
/// unchanged.
///
/// A param that backs a state field (same name) is emitted as `init_<name>`; a
/// policy param with no state field of its own (a deadline, a rate, a key) keeps
/// its authored name. If `init_<name>` would collide with a state field, another
/// param, an injected name, or an already-assigned initialiser, underscores are
/// appended to the prefix (`init__<name>`, `init___<name>`, …) until it does not
/// — deterministic, and terminating because each candidate is strictly longer
/// than every name it must avoid colliding with.
fn emitted_param_idents(model: &CovenantModel) -> Vec<String> {
    let mut taken: HashSet<String> = INJECTED_CONTRACT_PARAMS
        .iter()
        .map(|s| s.to_string())
        .collect();
    taken.extend(model.state.iter().map(|(n, _)| n.clone()));
    taken.extend(model.params.iter().map(|(n, _)| n.clone()));

    let mut idents = Vec::with_capacity(model.params.len());
    for (name, _) in &model.params {
        let backs_state_field = model.state.iter().any(|(f, _)| f == name);
        if !backs_state_field {
            idents.push(name.clone());
            continue;
        }
        let mut prefix = GENESIS_PARAM_PREFIX.to_string();
        let mut candidate = format!("{prefix}{name}");
        while taken.contains(&candidate) {
            prefix.push('_');
            candidate = format!("{prefix}{name}");
        }
        taken.insert(candidate.clone());
        idents.push(candidate);
    }
    idents
}

/// Check that every state field has a constructor param to be born from, and
/// that the param list is unambiguous. Separated from emission so `portrait
/// check` can run the same rule without emitting (`portrait_sema::check` cannot
/// carry it — 93 parse/check-only fixtures declare state with no params).
///
/// Genesis binding is by NAME, never by position. The rule this replaced paired
/// `model.state[i]` with `model.params[i]` and fell back to the literal `0` when
/// the param list ran short. Both halves were silent genesis corruption:
/// reordering either list rebound every field to a different param, and a state
/// field past the end of the param list was born zero with no diagnostic. Extra
/// params beyond the state set stay legal — a policy param need not have a state
/// field.
pub fn validate_genesis_binding(model: &CovenantModel) -> Result<(), String> {
    for (i, (name, _)) in model.params.iter().enumerate() {
        if model.params[..i].iter().any(|(prior, _)| prior == name) {
            return Err(format!(
                "covenant `{}`: constructor param `{}` is declared more than once. Genesis binding \
                 is by name, so a duplicate param makes the initialiser for `{}` ambiguous; remove \
                 or rename the duplicate.",
                model.name, name, name
            ));
        }
    }
    for (name, ty) in &model.state {
        let Some((_, pty)) = model.params.iter().find(|(pname, _)| pname == name) else {
            return Err(format!(
                "covenant `{}`: state field `{}` has no constructor param of the same name, so its \
                 genesis value is undefined. Declare `param {} {};` in the role — the compiler \
                 never silently binds a state field to 0.",
                model.name,
                name,
                emit_type(ty),
                name
            ));
        };
        if pty != ty {
            return Err(format!(
                "covenant `{}`: state field `{}` is declared `{}` but the constructor param `{}` \
                 is `{}`; the genesis initialiser must have the field's own type.",
                model.name,
                name,
                emit_type(ty),
                name,
                emit_type(pty)
            ));
        }
    }
    Ok(())
}

/// Lower one covenant model to silverscript. `proof_cov_input_index` is the input
/// index used for the injected vProg proof-covenant-id binding (see
/// [`DEFAULT_PROOF_COV_INPUT_INDEX`]); it is inert for a model without a vProg.
fn emit_model(model: &CovenantModel, proof_cov_input_index: usize) -> Result<SilFile, String> {
    let mut src = String::new();

    src.push_str("pragma silverscript ^0.1.0;\n");
    src.push_str("\n// Generated by portrait build\n");

    validate_genesis_binding(model)?;
    let param_idents = emitted_param_idents(model);

    // Contract declaration.  We always prepend `max_ins` and `max_outs` params
    // for the covenant binding attribute, followed by the user-declared params.
    src.push_str(&format!(
        "contract {}(int max_ins, int max_outs",
        model.name
    ));
    for ((_, ty), ident) in model.params.iter().zip(&param_idents) {
        src.push_str(&format!(", {} {}", emit_type(ty), ident));
    }
    src.push_str(") {\n");

    // State fields: direct field declarations initialised from the constructor
    // param the AUTHOR declared under the SAME NAME, emitted under its distinct
    // `init_`-prefixed silverscript identifier (see `emitted_param_idents`).
    for (name, ty) in &model.state {
        let ident = model
            .params
            .iter()
            .zip(&param_idents)
            .find(|((pname, _), _)| pname == name)
            .map(|(_, ident)| ident)
            .expect("validate_genesis_binding guarantees a same-named param for every state field");
        src.push_str(&format!("    {} {} = {};\n", emit_type(ty), name, ident));
    }

    // Entrypoints — skip NonCovenant (VProg) transitions; they are handled by Atelier.
    for tr in model
        .transitions
        .iter()
        .filter(|t| !matches!(t.mode, Mode::NonCovenant))
    {
        src.push('\n');
        // B3: a TERMINAL transition (mode = transition, no successor state) is a
        // lifecycle-ending spend that RELEASES the coin via `pays(...)` instead of
        // continuing the covenant into a successor. It emits a `binding = auth`
        // verification function with NO return — see `emit_terminal_transition`.
        // The non-terminal path below is left byte-identical.
        if matches!(tr.mode, Mode::Transition) && tr.to.is_none() {
            emit_terminal_transition(&mut src, model, tr)?;
            continue;
        }
        let mode_str = match tr.mode {
            Mode::Transition => "transition",
            Mode::Verification => "verification",
            Mode::NonCovenant => "non_covenant",
        };
        // Transition functions with the return-type declaration require `to = 1` (literal)
        // per silverscript c46e0e2+. Non-transition functions keep `to = max_outs`.
        let is_transition = matches!(tr.mode, Mode::Transition) && tr.to.is_some();
        let to_str = if is_transition { "1" } else { "max_outs" };
        src.push_str(&format!(
            "    #[covenant(binding = cov, from = max_ins, to = {}, mode = {})]\n",
            to_str, mode_str
        ));
        // Build extra arg string from M1 args.
        // When the role has a CSCI VProg binding, append `proof_cov_id: byte[32]` so
        // the caller can pass the covenant_id from the STARK journal for on-chain binding.
        let mut extra_args: String = tr
            .args
            .iter()
            .map(|(name, ty)| format!(", {} {}", emit_type(ty), name))
            .collect();
        if model.has_vprog && matches!(tr.mode, Mode::Transition) {
            extra_args.push_str(", byte[32] proof_cov_id");
        }
        // Transition mode uses the return-type declaration syntax (silverscript c46e0e2+):
        //   function f(State[] prev_states, ...) : (State) { return({ ... }); }
        let return_type_str = if is_transition { " : (State)" } else { "" };
        src.push_str(&format!(
            "    function {}(State[] prev_states{}){} {{\n",
            tr.entry, extra_args, return_type_str
        ));
        // Emit body.
        // Phase B4: walk the typed `ReturnExpr`/`Expr` tree directly via
        // `emit_expr` — no more re-stringify-then-substitute round-trip. Bare
        // state-field `Var`s are lowered to `prev_states[0].field` structurally.
        let return_expr: Option<&ReturnExpr> = tr.body.iter().find_map(|s| match s {
            portrait_syntax::Stmt::Return(expr) => Some(expr),
            _ => None,
        });
        if is_transition {
            // CSCI cross-layer binding check: verify that the STARK journal's
            // covenant_id matches the on-chain covenant's own ID.
            // proof_cov_id is bytes 0..32 of the RISC Zero STARK journal
            // (the covenant_id field; matches the COV_ID const embedded in the vProg guest).
            // OpInputCovenantId(idx) returns the covenant ID of the spending input
            // at `idx`. NOTE: the index ASSUMES the covenant UTXO sits at that input
            // position in the spending transaction; the default is 0 (covenant is the
            // FIRST input — see DEFAULT_PROOF_COV_INPUT_INDEX). For a multi-input spend
            // where the covenant appears elsewhere, `proof_cov_input_index` must be set
            // to that position, or the guard binds the WRONG input. This assumption is
            // documented in library/ENFORCEMENT.md.
            // [PENDING: add OpZkPrecompile(0x21, journal) call when engine support lands]
            if model.has_vprog {
                src.push_str(&format!(
                    "        require(proof_cov_id == OpInputCovenantId({}));\n",
                    proof_cov_input_index
                ));
            }
            // Emit the entrypoint's `require(...)` guards. Each Require statement's
            // expression is lowered by walking the typed `Expr`: bare state field
            // `Var`s are rewritten to `prev_states[0].field`, while constructor
            // params (e.g. issuer, window) and entrypoint args pass through unchanged.
            // Without this the emitted covenant would carry state forward but enforce
            // none of its invariants — the guards are what make the transition reject.
            let field_set: HashSet<&str> = model.state.iter().map(|(n, _)| n.as_str()).collect();
            for stmt in &tr.body {
                match stmt {
                    portrait_syntax::Stmt::Require(expr) => {
                        src.push_str(&format!(
                            "        require({});\n",
                            emit_expr(expr, &field_set)
                        ));
                    }
                    // Fail-loud: an unrecognised guard form parsed to `Stmt::Raw`
                    // (e.g. the `@` age syntax `requires v @ 1;`). Emitting the
                    // covenant while dropping it would yield a contract that LOOKS
                    // gated but enforces nothing. Abort instead, naming the
                    // offending guard so the gap cannot ship silently.
                    portrait_syntax::Stmt::Raw(text) => {
                        return Err(format!(
                            "covenant `{}` entrypoint `{}`: unlowerable guard statement \
                             `{}` would be silently dropped — refusing to emit an \
                             unguarded covenant. Rewrite it as a `requires <expr>;` \
                             clause the emitter can lower, or route the construct to \
                             the vProgs layer.",
                            model.name,
                            tr.entry,
                            text.trim()
                        ));
                    }
                    portrait_syntax::Stmt::Return(_) => {}
                    // B2: a `pays(...)` clause is carried as a `Stmt::Pays` for
                    // provenance but is lowered from `tr.guards` (Guard::OutputPays)
                    // below, so it is inert in the body loop.
                    portrait_syntax::Stmt::Pays { .. } => {}
                    // B1: an `after(...)` clause is carried as a `Stmt::After` for
                    // provenance but is lowered from `tr.guards` (Guard::TimeAtLeast)
                    // below, so it is inert in the body loop.
                    portrait_syntax::Stmt::After { .. } => {}
                }
            }
            // B2: lower each `Guard::OutputPays` to two output-introspection
            // requires that make CONSENSUS enforce the payout. `tx.outputs[k].value`
            // lowers to OpTxOutputAmount (the engine pushes the output value as a
            // script number) and is compared to the committed `amount` int;
            // `tx.outputs[k].scriptPubKey` lowers to OpTxOutputSpk (the full
            // serialized spk: 2-byte big-endian version || script) and is compared
            // to the committed payee's spk, reconstructed with the spk builtin
            // [`payee_spk_builtin`] picks from the payee's DECLARED type. The
            // committed payee/amount are read from genesis state via
            // `prev_states[0].field` (the same lowering the body requires use), so a
            // spender cannot substitute a different destination.
            //
            // PRECONDITIONS / SCOPE (must stay documented in library/ENFORCEMENT.md):
            //   * PAYEE ADDRESS FORM IS THE DECLARED TYPE'S (M4). A `pubkey` payee
            //     binds a 32-byte-Schnorr P2PK spk; a `byte[32]` payee binds a P2SH
            //     script-hash spk. The checker cannot see which form the payee's REAL
            //     settlement address uses, so declaring the wrong one leaves that
            //     path dead for them (funds recoverable only via the covenant's other
            //     paths).
            //   * BINDS ONLY output[k] (L1). Nothing here constrains the OTHER outputs
            //     or checks value conservation / transaction mass (KIP-9) — an
            //     over-funded covenant lets the surplus be spender-routed. Fund the
            //     covenant to exactly `amount + successor + fee`, or the surplus is
            //     spender-controlled.
            //   * COMMITTED-AT-GENESIS (L2). The binding is only as trustworthy as the
            //     instantiation ceremony that committed `payee`/`amount` into covenant
            //     state; that ceremony must be independently trusted/verified.
            for guard in &tr.guards {
                if let Guard::OutputPays { index, to, amount } = guard {
                    let spk_builtin = payee_spk_builtin(model, tr, to)?;
                    let amount_lowered = emit_expr(&Expr::Var(amount.clone()), &field_set);
                    let payee_lowered = emit_expr(&Expr::Var(to.clone()), &field_set);
                    src.push_str(&format!(
                        "        require(tx.outputs[{}].value == {});\n",
                        index, amount_lowered
                    ));
                    src.push_str(&format!(
                        "        require(tx.outputs[{}].scriptPubKey == byte[](new {}({})));\n",
                        index, spk_builtin, payee_lowered
                    ));
                }
            }
            // B1: lower each `Guard::TimeAtLeast` to a `require(tx.time >= <committed
            // deadline>);` time gate. The special TxVar `tx.time` routes through
            // silverc's `compile_time_op_statement` to `OpCheckLockTimeVerify`. A
            // bare `tx.locktime` compare is BYPASSABLE and is deliberately NOT
            // emitted. The committed `deadline` is read from genesis state via
            // `prev_states[0].field` (the same lowering the body requires use), so a
            // spender cannot substitute an earlier deadline.
            //
            // TWO HALVES — WHAT IS AND IS NOT ENFORCED HERE (do not overclaim):
            // The "cannot be spent before the deadline" guarantee is enforced by
            // TWO SEPARATE consensus rules; the emitted CLTV opcode is only HALF.
            //   1. CLTV (the txscript opcode `OpCheckLockTimeVerify`, opcodes/mod.rs
            //      :1039,1057) enforces that the tx COMMITS a `lock_time >= deadline`
            //      (domain-matched) AND the spending input is NON-FINAL (sequence !=
            //      MAX_TX_IN_SEQUENCE_NUM, defeating the final-sequence bypass). It
            //      reads ONLY the spender-set `lock_time` FIELD — it has NO access to
            //      the block DAA score, so it does NOT by itself prove that the
            //      deadline has elapsed. This half is what `time_gate_engine.rs`
            //      exercises on the pinned engine.
            //   2. The actual no-early-INCLUSION rule is the SEPARATE consensus
            //      finalization check `check_tx_is_finalized`
            //      (consensus/.../tx_validation_in_header_context.rs:72-93): a
            //      non-final tx with `lock_time = L` is admissible into the blockDAG
            //      only once `block_daa_score > L`. This is the load-bearing "time
            //      has passed" half — it lives OUTSIDE txscript and is NOT exercised
            //      by the engine tests (it is `pub(crate)`, reachable only through
            //      the full VirtualProcessor pipeline; see ENFORCEMENT.md).
            // Together: CLTV forces the tx to carry `lock_time >= deadline` on a
            // non-final input, and the finalization rule then bars that tx from a
            // block until the DAA score passes the deadline.
            //
            // PRECONDITIONS / SCOPE (must stay documented in library/ENFORCEMENT.md):
            //   * DEADLINE DOMAIN IS THE SPENDER'S/COMMITTER'S CHOICE (L1). A
            //     `deadline` below `LOCK_TIME_THRESHOLD` (500_000_000_000) is a DAA
            //     score, at/above it a Unix time. The committed `deadline` and the
            //     spending tx's lock_time must be in the SAME domain — the covenant
            //     cannot check which domain the committed value is in; that is a
            //     ceremony precondition.
            //   * ENFORCES A LOCK-TIME BOUND, NOT WALL-CLOCK TRUTH (L2). The gate
            //     proves the tx commits a lock_time >= the deadline on a non-final
            //     input; the "deadline has actually passed" step is the separate
            //     finalization rule above, not this opcode. This is the sound
            //     consensus notion of "not before" — nothing more.
            //   * COMMITTED-AT-GENESIS, AND A ZERO/PAST DEADLINE IS NO GATE (L3). The
            //     gate is only as trustworthy as the instantiation ceremony that
            //     committed `deadline`. A `deadline` of `0` maps to
            //     `LockTimeType::Finalized` (finalization returns Ok unconditionally)
            //     and any `deadline <= the instantiation DAA score` opens the gate
            //     fully — the ceremony MUST commit a real future deadline. This
            //     cannot be a compile-time diagnostic (the value is a committed ctor
            //     input, not visible when the covenant is engraved).
            for guard in &tr.guards {
                if let Guard::TimeAtLeast { deadline } = guard {
                    // NOTE: the deadline is read from `prev_states[0]` — the same
                    // single-covenant / input-0 assumption family as
                    // `OpInputCovenantId(0)` above. For a multi-covenant spend where
                    // this covenant is not prev_states[0], the index must be adjusted;
                    // the composed-spend slice inherits this caveat.
                    //
                    // The `Sum(a, b)` window form lowers to `prev_states[0].a +
                    // prev_states[0].b`; silverc routes the same `tx.time >= <expr>`
                    // shape to `OpCheckLockTimeVerify` (the threshold is computed on
                    // stack from the two committed atoms before the CLTV check).
                    let deadline_expr = match deadline {
                        AfterDeadline::Field(f) => Expr::Var(f.clone()),
                        AfterDeadline::Sum(a, b) => Expr::Binary {
                            op: BinOp::Add,
                            lhs: Box::new(Expr::Var(a.clone())),
                            rhs: Box::new(Expr::Var(b.clone())),
                        },
                    };
                    let deadline_lowered = emit_expr(&deadline_expr, &field_set);
                    src.push_str(&format!(
                        "        require(tx.time >= {});\n",
                        deadline_lowered
                    ));
                }
            }
            match return_expr {
                Some(ReturnExpr::Object { fields, .. }) => {
                    // Multi-field state object literal: `Name { f1: v1, f2: v2, ... }`.
                    // Keep each field key bare; lower only the value side (state field
                    // references → prev_states[0].field; args/params pass through).
                    let rendered: Vec<String> = fields
                        .iter()
                        .map(|(key, value)| format!("{}: {}", key, emit_expr(value, &field_set)))
                        .collect();
                    src.push_str(&format!("        return({{ {} }});\n", rendered.join(", ")));
                }
                Some(ReturnExpr::Scalar(expr)) => {
                    // Scalar return: build the full state object, substituting the lowered
                    // expression for the single referenced field and carrying the rest.
                    let rendered: Vec<String> = model
                        .state
                        .iter()
                        .map(|(name, _)| {
                            if expr_references_var(expr, name) {
                                format!("{}: {}", name, emit_expr(expr, &field_set))
                            } else {
                                format!("{}: prev_states[0].{}", name, name)
                            }
                        })
                        .collect();
                    src.push_str(&format!("        return({{ {} }});\n", rendered.join(", ")));
                }
                None => {
                    src.push_str("        return(prev_states[0]);\n");
                }
            }
        } else {
            // L4 — everything above (guards, pays, after, return) is emitted only
            // on the transition path. A non-terminal `mode = verification`
            // entrypoint fell through to an EMPTY body: its `requires
            // checkSig(auth, owner)` was SILENTLY DROPPED while `portrait check`
            // reported ok, leaving a covenant that LOOKS gated and enforces
            // nothing — the exact defect `emit`'s fail-loud contract exists to
            // prevent, and it only failed closed by accident. Refuse to emit it.
            let dropped = tr.body.iter().find_map(|s| match s {
                portrait_syntax::Stmt::Require(expr) => Some(expr.to_silverscript()),
                portrait_syntax::Stmt::Raw(text) => Some(text.trim().to_string()),
                _ => None,
            });
            if let Some(guard) = dropped.or_else(|| {
                (!tr.guards.is_empty()).then(|| "a `pays(...)`/`after(...)` clause".to_string())
            }) {
                return Err(format!(
                    "covenant `{}` entrypoint `{}`: `mode = verification` bodies are not lowered, \
                     so the guard `{}` would be SILENTLY DROPPED — refusing to emit a covenant \
                     that looks gated but enforces nothing. Declare the entrypoint \
                     `#[covenant(mode = transition)]` (its guards are lowered), or make it a \
                     terminal spend via a `terminal` lifecycle edge.",
                    model.name, tr.entry, guard
                ));
            }
        }
        src.push_str("    }\n");
    }

    src.push_str("}\n");

    Ok(SilFile {
        name: format!("{}.sil", model.name),
        source: src,
    })
}

/// The silverscript spk builtin a `pays(...)` payee lowers through, chosen from
/// the payee's ALREADY-DECLARED type — no new surface syntax, no second `pays`
/// variant, and no raw spk bytes committed into state:
///
///   * `pubkey`    → `ScriptPubKeyP2PK`  — a 32-byte-Schnorr pay-to-pubkey spk;
///   * `byte[32]`  → `ScriptPubKeyP2SH`  — a pay-to-script-hash spk over the
///     committed redeem-script hash (multisig, a timelock script, another
///     covenant, …).
///
/// Any other declared type is an EMIT ERROR rather than a guessed lowering: a
/// payee that is not an address is not bindable to an output scriptPubKey, and
/// silently picking one of the two forms would emit a covenant whose happy path
/// can never be satisfied.
///
/// HONEST SCOPE: this picks the spk FORM from the declared type; it does not and
/// cannot verify that the payee's real settlement address uses that form. A payee
/// who actually settles to a script hash but was committed as a `pubkey` still has
/// a dead path — that ceremony fact is a documented precondition (M4 in
/// `library/ENFORCEMENT.md`), which this dispatch makes EXPRESSIBLE rather than
/// unreachable.
fn payee_spk_builtin(
    model: &CovenantModel,
    tr: &Transition,
    payee: &str,
) -> Result<&'static str, String> {
    let ty = model
        .state
        .iter()
        .chain(model.params.iter())
        .find(|(name, _)| name == payee)
        .map(|(_, ty)| ty)
        .ok_or_else(|| {
            format!(
                "covenant `{}` entrypoint `{}`: `pays(...)` payee `{}` names no committed state \
                 field or role param, so its address form cannot be determined",
                model.name, tr.entry, payee
            )
        })?;
    match ty {
        Type::PubKey => Ok("ScriptPubKeyP2PK"),
        Type::Bytes32 => Ok("ScriptPubKeyP2SH"),
        other => Err(format!(
            "covenant `{}` entrypoint `{}`: `pays(...)` payee `{}` is declared `{}`, which is not \
             an address — a payee must be a `pubkey` (lowered to ScriptPubKeyP2PK) or a `byte[32]` \
             script hash (lowered to ScriptPubKeyP2SH); refusing to guess an output scriptPubKey",
            model.name,
            tr.entry,
            payee,
            emit_type(other)
        )),
    }
}

/// Emit a TERMINAL transition (B3): a lifecycle-ending spend that RELEASES the
/// covenant coin to a committed payee via `pays(...)` and CONSUMES the UTXO,
/// instead of continuing the covenant into a successor. The emitted shape is a
/// `binding = auth` VERIFICATION function with NO return:
///
/// ```silverscript
/// #[covenant(binding = auth, from = max_ins, to = 1, mode = verification)]
/// function release(State prev_state, State[] new_states, sig auth) {
///     require(checkSig(auth, prev_state.seller));
///     require(tx.outputs[0].value == prev_state.amount);
///     require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_state.seller)));
/// }
/// ```
///
/// Key differences from the non-terminal successor path:
///   * `binding = auth` (not `cov`): the spend is authorized by the signature the
///     body checks, NOT by covenant-id inheritance — there is no successor
///     covenant to inherit the ID.
///   * `mode = verification` + NO `return(...)`: the coin is released via `pays`
///     and the UTXO is consumed; the covenant does not continue.
///   * state is read through the SINGULAR `prev_state.<field>` accessor (not
///     `prev_states[0].<field>`).
///
/// The `pays(...)`/`after(...)` guards lower exactly as on the non-terminal path
/// (the same two output-introspection requires / CLTV gate, with the same scope
/// caveats documented at their non-terminal emission sites and in
/// `library/ENFORCEMENT.md`), only through the `prev_state` accessor.
fn emit_terminal_transition(
    src: &mut String,
    model: &CovenantModel,
    tr: &Transition,
) -> Result<(), String> {
    // Honesty over silent mis-binding: a terminal spend produces no successor
    // covenant, so there is nothing to carry a proof-covenant-id binding onto.
    // Refuse a vProg-bearing terminal outright rather than emit a covenant that
    // silently drops the CSCI cross-layer binding. (No library case has one.)
    if model.has_vprog {
        return Err(format!(
            "covenant `{}` entrypoint `{}`: terminal transition cannot carry a vprog — a \
             lifecycle-ending spend produces no successor covenant to carry the \
             proof-covenant-id binding; route the proof to a non-terminal transition",
            model.name, tr.entry
        ));
    }
    // `to = 1`: silverc rejects `to = 0` ("`to` must be >= 1"), so 1 is the MINIMUM
    // literal it accepts for the output-count binding. A terminal release, however,
    // produces ZERO covenant-successor outputs (the coin leaves to the payee; no
    // successor covenant is created). Whether the silverscript covenant RUNTIME
    // admits a 0-covenant-successor spend under `binding = auth` + `to = 1` is
    // UNVERIFIED on the mandated `v2.0.0` pin — it is the composed-on-engine-spend
    // -pending piece (blocked by the upstream covenant-ABI pin, same bucket as B2/B1). If the
    // runtime instead REQUIRES a covenant successor, a terminal spend producing none
    // would be rejected and the UTXO would be STUCK. Do NOT deploy a terminal
    // covenant to a value-bearing UTXO until that composed spend is proven. See
    // KNOWN-ISSUES.md (load-bearing pending item) and library/ENFORCEMENT.md.
    src.push_str("    #[covenant(binding = auth, from = max_ins, to = 1, mode = verification)]\n");
    let extra_args: String = tr
        .args
        .iter()
        .map(|(name, ty)| format!(", {} {}", emit_type(ty), name))
        .collect();
    src.push_str(&format!(
        "    function {}(State prev_state, State[] new_states{}) {{\n",
        tr.entry, extra_args
    ));

    let field_set: HashSet<&str> = model.state.iter().map(|(n, _)| n.as_str()).collect();
    // The terminal state accessor is the SINGULAR `prev_state` (no `[0]` index).
    const ACC: &str = "prev_state";

    for stmt in &tr.body {
        match stmt {
            portrait_syntax::Stmt::Require(expr) => {
                src.push_str(&format!(
                    "        require({});\n",
                    emit_expr_with(expr, &field_set, ACC)
                ));
            }
            // Same fail-loud as the non-terminal path: an unlowerable guard that
            // parsed to `Stmt::Raw` must abort rather than be silently dropped.
            portrait_syntax::Stmt::Raw(text) => {
                return Err(format!(
                    "covenant `{}` entrypoint `{}`: unlowerable guard statement `{}` would be \
                     silently dropped — refusing to emit an unguarded covenant. Rewrite it as a \
                     `requires <expr>;` clause the emitter can lower, or route the construct to \
                     the vProgs layer.",
                    model.name,
                    tr.entry,
                    text.trim()
                ));
            }
            // A terminal transition emits NO successor; sema rejects a `return` on
            // a terminal path upstream, so ignore it here (defensive: never emit
            // one). `Pays`/`After` are lowered from `tr.guards` below.
            portrait_syntax::Stmt::Return(_)
            | portrait_syntax::Stmt::Pays { .. }
            | portrait_syntax::Stmt::After { .. } => {}
        }
    }
    // pays(...) → the two output-introspection requires that make CONSENSUS enforce
    // the payout (identical lowering + scope caveats to the non-terminal path),
    // reading the committed payee/amount via the `prev_state` accessor.
    for guard in &tr.guards {
        if let Guard::OutputPays { index, to, amount } = guard {
            let spk_builtin = payee_spk_builtin(model, tr, to)?;
            let amount_lowered = emit_expr_with(&Expr::Var(amount.clone()), &field_set, ACC);
            let payee_lowered = emit_expr_with(&Expr::Var(to.clone()), &field_set, ACC);
            src.push_str(&format!(
                "        require(tx.outputs[{}].value == {});\n",
                index, amount_lowered
            ));
            src.push_str(&format!(
                "        require(tx.outputs[{}].scriptPubKey == byte[](new {}({})));\n",
                index, spk_builtin, payee_lowered
            ));
        }
    }
    // after(...) → the `require(tx.time >= <committed deadline>);` CLTV gate
    // (identical lowering + scope caveats to the non-terminal path).
    for guard in &tr.guards {
        if let Guard::TimeAtLeast { deadline } = guard {
            let deadline_expr = match deadline {
                AfterDeadline::Field(f) => Expr::Var(f.clone()),
                AfterDeadline::Sum(a, b) => Expr::Binary {
                    op: BinOp::Add,
                    lhs: Box::new(Expr::Var(a.clone())),
                    rhs: Box::new(Expr::Var(b.clone())),
                },
            };
            let deadline_lowered = emit_expr_with(&deadline_expr, &field_set, ACC);
            src.push_str(&format!(
                "        require(tx.time >= {});\n",
                deadline_lowered
            ));
        }
    }
    src.push_str("    }\n");
    Ok(())
}

/// Stable marker recorded in a Hallmark for an instance whose constructor
/// arguments are [`emit_ctor`]'s all-zero defaults rather than real ones.
pub const GENESIS_PLACEHOLDER_ZERO: &str = "placeholder-zero";

/// The loud warning owed to any author whose covenant just got a defaulted CTOR
/// JSON from [`emit_ctor`], or `None` when the covenant takes no user params
/// (nothing was defaulted, so there is nothing to warn about).
///
/// `emit_ctor` fills EVERY user param with a type-shaped zero — `0` for
/// int/coin, 32 zero bytes for a pubkey, 64 for a signature — and there is no
/// flag for supplying real values. That artifact is a compile fixture, not a
/// deployment: a covenant whose committed `owner` pubkey is all-zero has a
/// `checkSig` that can never be satisfied, i.e. permanently locked funds. The
/// KovId derived from it identifies the PLACEHOLDER instance, so it must never
/// be read as the deployment's identity.
pub fn placeholder_ctor_warning(model: &CovenantModel) -> Option<String> {
    if model.params.is_empty() {
        return None;
    }
    Some(format!(
        "warning: {}_ctor.json is an ALL-ZERO PLACEHOLDER genesis ({} constructor arg(s) \
         defaulted); any KovId derived from it identifies the placeholder instance, NOT your \
         deployment. An all-zero pubkey is a key nobody holds — deploying this would lock the \
         funds. Supply real constructor args before deploying.",
        model.name,
        model.params.len()
    ))
}

/// Emit a default CTOR JSON for `silverc --ctor` compilation.
/// Returns (filename, json_content).  Expr format: {"kind":"int","data":N}.
///
/// Every user param is filled with a type-shaped ZERO — see
/// [`placeholder_ctor_warning`], which callers must surface.
pub fn emit_ctor(model: &CovenantModel) -> (String, String) {
    // Always prepend max_ins=1 and max_outs=1.
    let mut exprs: Vec<String> = vec![
        r#"{"kind":"int","data":1}"#.to_string(),
        r#"{"kind":"int","data":1}"#.to_string(),
    ];
    for (_, ty) in &model.params {
        exprs.push(default_expr(ty));
    }
    (
        format!("{}_ctor.json", model.name),
        format!("[{}]", exprs.join(",")),
    )
}

fn default_expr(ty: &Type) -> String {
    match ty {
        Type::Int | Type::Coin => r#"{"kind":"int","data":0}"#.to_string(),
        Type::Bool => r#"{"kind":"bool","data":false}"#.to_string(),
        // silverc expects fixed-width byte arrays as {"kind":"array","data":[{"kind":"byte",...}]}.
        // It rejects {"kind":"hex",...} ("unknown variant `hex`"), so emit zero-filled byte arrays:
        // 32 bytes for pubkey/byte[32], 64 bytes for a signature.
        Type::PubKey | Type::Bytes32 => zero_byte_array(32),
        Type::Sig => zero_byte_array(64),
        Type::Set(_) | Type::Map(_, _) => r#"{"kind":"array","data":[]}"#.to_string(),
        Type::Named(_) => r#"{"kind":"int","data":0}"#.to_string(),
    }
}

/// Emit a silverc Expr for a zero-filled fixed-width byte array of length `n`,
/// i.e. {"kind":"array","data":[{"kind":"byte","data":0}, ... n times]}.
fn zero_byte_array(n: usize) -> String {
    let bytes = vec![r#"{"kind":"byte","data":0}"#; n].join(",");
    format!(r#"{{"kind":"array","data":[{}]}}"#, bytes)
}

/// Lower a typed `Expr` to its silverscript surface form, rewriting each bare
/// state-field `Var` to `prev_states[0].field` (the non-terminal successor
/// accessor). Thin wrapper over [`emit_expr_with`]; see it for the full note.
fn emit_expr(expr: &Expr, state_fields: &HashSet<&str>) -> String {
    emit_expr_with(expr, state_fields, "prev_states[0]")
}

/// Lower a typed `Expr` to its silverscript surface form, rewriting each bare
/// state-field `Var` to `<accessor>.field`. This is the AST-walking replacement
/// for the Phase B transitional string substitution (`lower_return_expr`) — field
/// detection is now structural (only a `Var` whose name is a state field is
/// rewritten), so prefix collisions are impossible by construction and there is
/// no re-stringify/re-tokenize round-trip.
///
/// `accessor` is the state-record prefix: `"prev_states[0]"` for the non-terminal
/// successor path (a `State[] prev_states` signature), or `"prev_state"` for a B3
/// terminal transition (a singular `State prev_state` signature). It is the only
/// thing that varies between the two paths, so the emitted .sil for the
/// non-terminal path stays byte-identical to the pre-B3 output.
///
/// Operator spelling and whitespace match `Expr::to_silverscript` exactly.
fn emit_expr_with(expr: &Expr, state_fields: &HashSet<&str>, accessor: &str) -> String {
    match expr {
        Expr::Int(n) => n.to_string(),
        Expr::Bool(b) => b.to_string(),
        Expr::Bytes(bytes) => {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("0x{hex}")
        }
        Expr::Var(name) => {
            if state_fields.contains(name.as_str()) {
                format!("{}.{}", accessor, name)
            } else {
                name.clone()
            }
        }
        Expr::Field { base, field } => {
            format!("{}.{}", emit_expr_with(base, state_fields, accessor), field)
        }
        Expr::Index { base, index } => format!(
            "{}[{}]",
            emit_expr_with(base, state_fields, accessor),
            emit_expr_with(index, state_fields, accessor)
        ),
        Expr::Unary { op, rhs } => {
            // Unary operators bind tighter than every binary operator, so a
            // `Binary` operand is only reachable via authored parentheses and
            // must be re-parenthesized — otherwise `-(a + b)` would emit as
            // `-a + b` and silently regroup to `(-a) + b`. Non-Binary operands
            // (literals/idents/fields/calls/nested unary) never need parens.
            let inner = emit_expr_with(rhs, state_fields, accessor);
            if matches!(rhs.as_ref(), Expr::Binary { .. }) {
                format!("{}({})", op.as_str(), inner)
            } else {
                format!("{}{}", op.as_str(), inner)
            }
        }
        Expr::Binary { op, lhs, rhs } => {
            // Precedence-aware emission: parenthesize a Binary *child* whenever
            // re-parsing the flat string would regroup it differently from the
            // authored tree. Non-Binary children never need parens. All Portrait
            // binary operators are left-associative (parser sets right_bp = left_bp + 1),
            // so: a LEFT child needs parens iff its precedence is strictly lower
            // than the parent's; a RIGHT child needs parens iff its precedence is
            // lower than OR equal to the parent's. Without this, e.g.
            // `(a + b) * c` would emit as `a + b * c` and silently weaken the guard.
            let pp = binop_prec(*op);
            format!(
                "{} {} {}",
                emit_child(lhs, state_fields, pp, false, accessor),
                op.as_str(),
                emit_child(rhs, state_fields, pp, true, accessor),
            )
        }
        Expr::Call { name, args } => {
            let rendered: Vec<String> = args
                .iter()
                .map(|a| emit_expr_with(a, state_fields, accessor))
                .collect();
            format!("{}({})", name, rendered.join(", "))
        }
    }
}

/// Surface precedence of a binary operator, mirroring the Pratt parser's binding
/// powers in `portrait-syntax` (Mul=9, Add/Sub=7, comparisons=5, And=3, Or=1).
/// Higher binds tighter. Kept in sync with `Parser::peek_binop`.
fn binop_prec(op: portrait_syntax::BinOp) -> u8 {
    use portrait_syntax::BinOp;
    match op {
        BinOp::Or => 1,
        BinOp::And => 3,
        BinOp::Eq | BinOp::Ne | BinOp::Ge | BinOp::Le | BinOp::Gt | BinOp::Lt => 5,
        BinOp::Add | BinOp::Sub => 7,
        BinOp::Mul => 9,
    }
}

/// Emit a binary operand, wrapping it in parens only when re-parsing would
/// regroup it. `parent_prec` is the precedence of the enclosing operator;
/// `is_right` is true for the right-hand operand. All operators are
/// left-associative, so a right operand of equal precedence must be parenthesized
/// (e.g. `a - (b - c)`), while a left operand of equal precedence need not be
/// (e.g. `a - b - c`). Only `Binary` children can ever need parens.
fn emit_child(
    expr: &Expr,
    state_fields: &HashSet<&str>,
    parent_prec: u8,
    is_right: bool,
    accessor: &str,
) -> String {
    let inner = emit_expr_with(expr, state_fields, accessor);
    if let Expr::Binary { op, .. } = expr {
        let cp = binop_prec(*op);
        let needs = if is_right {
            cp <= parent_prec
        } else {
            cp < parent_prec
        };
        if needs {
            return format!("({})", inner);
        }
    }
    inner
}

/// Whether `expr` references the variable `name` anywhere as a bare `Var`.
/// Replaces the old `expr.contains(name)` substring heuristic with a structural
/// walk, so e.g. a field `bal` is not considered referenced by `balance`.
fn expr_references_var(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Var(v) => v == name,
        Expr::Int(_) | Expr::Bool(_) | Expr::Bytes(_) => false,
        Expr::Field { base, .. } => expr_references_var(base, name),
        Expr::Index { base, index } => {
            expr_references_var(base, name) || expr_references_var(index, name)
        }
        Expr::Unary { rhs, .. } => expr_references_var(rhs, name),
        Expr::Binary { lhs, rhs, .. } => {
            expr_references_var(lhs, name) || expr_references_var(rhs, name)
        }
        Expr::Call { args, .. } => args.iter().any(|a| expr_references_var(a, name)),
    }
}

fn emit_type(ty: &Type) -> String {
    match ty {
        Type::Int => "int".to_string(),
        Type::Bool => "bool".to_string(),
        Type::PubKey => "pubkey".to_string(),
        Type::Sig => "sig".to_string(),
        Type::Bytes32 => "byte[32]".to_string(),
        // silverscript has no `coin` type — `coin` is a Portrait-level concept
        // (a strictly-conserved value, distinct from an adjustable `int`). It is
        // lowered to silverscript's `int` representation here. silverc rejects a
        // bare `coin` struct field ("unknown type 'coin' in struct field"); the
        // Portrait type checker still treats `coin` as non-arithmetic and
        // non-int-comparable, so the conservation guarantees are enforced at the
        // Portrait layer, not lost by this lowering. See emit_type tests.
        Type::Coin => "int".to_string(),
        Type::Set(inner) => format!("{}[]", emit_type(inner)),
        Type::Map(k, v) => format!("map<{},{}>", emit_type(k), emit_type(v)),
        Type::Named(n) => n.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portrait_ir::{CovenantModel, Mode, Transition};
    use portrait_syntax::{Stmt, Type};

    fn counter_model(body: Vec<Stmt>) -> CovenantModel {
        CovenantModel {
            name: "Counter".into(),
            params: vec![("value".into(), Type::Int)],
            state: vec![("value".into(), Type::Int)],
            transitions: vec![Transition {
                entry: "bump".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![],
                capability: None,
                args: vec![("delta".into(), Type::Int)],
                body,
            }],
            has_vprog: false,
        }
    }

    fn counter_model_with_vprog(body: Vec<Stmt>) -> CovenantModel {
        CovenantModel {
            has_vprog: true,
            ..counter_model(body)
        }
    }

    #[test]
    fn counter_m1_args_in_signature() {
        let model = counter_model(vec![Stmt::Return(
            portrait_syntax::parse_return_expr("value + delta").unwrap(),
        )]);
        let files = emit(&[model]).expect("emit");
        assert_eq!(files.len(), 1);
        let src = &files[0].source;
        assert!(
            src.contains("int delta"),
            "should include delta arg: {}",
            src
        );
        assert!(
            src.contains(": (State)"),
            "should emit return-type annotation: {}",
            src
        );
        assert!(
            src.contains("return({"),
            "should emit return expression: {}",
            src
        );
        assert!(
            src.contains("prev_states[0].value + delta"),
            "should lower expression: {}",
            src
        );
    }

    #[test]
    fn counter_m1_no_return_statement() {
        let model = counter_model(vec![Stmt::Return(
            portrait_syntax::parse_return_expr("value + delta").unwrap(),
        )]);
        let files = emit(&[model]).expect("emit");
        let src = &files[0].source;
        assert!(
            !src.contains("return(prev_states)"),
            "should not have stub return: {}",
            src
        );
    }

    #[test]
    fn emit_covenant_with_vprog_adds_proof_cov_id_arg_and_require() {
        let model = counter_model_with_vprog(vec![Stmt::Return(
            portrait_syntax::parse_return_expr("value + delta").unwrap(),
        )]);
        let files = emit(&[model]).expect("emit");
        let src = &files[0].source;
        // proof_cov_id arg must appear in the function signature.
        assert!(
            src.contains("byte[32] proof_cov_id"),
            "proof_cov_id arg missing: {}",
            src
        );
        // OpInputCovenantId binding check must appear in the function body.
        assert!(
            src.contains("require(proof_cov_id == OpInputCovenantId(0))"),
            "OpInputCovenantId require missing: {}",
            src
        );
    }

    #[test]
    fn emit_proof_cov_binding_uses_configured_input_index() {
        // Finding 2: the injected proof-covenant-id binding index is parametric.
        // A non-default index must be reflected in the emitted OpInputCovenantId,
        // so a spend where the covenant UTXO is not input 0 can bind the right one.
        let model = counter_model_with_vprog(vec![Stmt::Return(
            portrait_syntax::parse_return_expr("value + delta").unwrap(),
        )]);
        let src = emit_model(&model, 2).expect("emit").source;
        assert!(
            src.contains("require(proof_cov_id == OpInputCovenantId(2))"),
            "binding must reflect the configured input index 2: {}",
            src
        );
        assert!(
            !src.contains("OpInputCovenantId(0)"),
            "non-default index must NOT emit the default OpInputCovenantId(0): {}",
            src
        );
    }

    #[test]
    fn emit_covenant_without_vprog_no_proof_cov_id() {
        let model = counter_model(vec![Stmt::Return(
            portrait_syntax::parse_return_expr("value + delta").unwrap(),
        )]);
        let files = emit(&[model]).expect("emit");
        let src = &files[0].source;
        // No VProg: no proof_cov_id or binding check.
        assert!(
            !src.contains("proof_cov_id"),
            "should not have proof_cov_id without vprog: {}",
            src
        );
        assert!(
            !src.contains("OpInputCovenantId"),
            "should not have OpInputCovenantId without vprog: {}",
            src
        );
    }

    #[test]
    fn emit_expr_prefix_collision_safe() {
        // Field "val" must not be lowered when only "value" is referenced.
        // With the AST walk this is structural: a Var("value") never matches the
        // "val" field, so prefix collisions are impossible by construction.
        let fields: HashSet<&str> = ["val", "value"].into_iter().collect();
        let e = portrait_syntax::parse_expr("value + delta").unwrap();
        assert_eq!(
            emit_expr(&e, &fields),
            "prev_states[0].value + delta",
            "val should not contaminate value"
        );
        let e2 = portrait_syntax::parse_expr("val + 1").unwrap();
        assert_eq!(
            emit_expr(&e2, &fields),
            "prev_states[0].val + 1",
            "val standalone"
        );
    }

    #[test]
    fn emit_expr_lowers_only_state_field_vars() {
        // Constructor params / args (not in the field set) pass through unchanged;
        // function-call args and nested operands are walked structurally.
        let fields: HashSet<&str> = ["seq"].into_iter().collect();
        let e = portrait_syntax::parse_expr("checkSig(sig, issuer) && seq <= window").unwrap();
        assert_eq!(
            emit_expr(&e, &fields),
            "checkSig(sig, issuer) && prev_states[0].seq <= window"
        );
    }

    #[test]
    fn emit_expr_lowers_blake2b_hashlock_verbatim() {
        // blake2b(preimage) == hashlock: the call lowers verbatim to the silverc
        // `blake2b(_)` builtin (which emits OpBlake2b, 0xaa); the committed
        // `hashlock` state field is read through prev_states[0]. The caller-
        // supplied `preimage` passes through unchanged.
        let fields: HashSet<&str> = ["hashlock"].into_iter().collect();
        let e = portrait_syntax::parse_expr("blake2b(preimage) == hashlock").unwrap();
        assert_eq!(
            emit_expr(&e, &fields),
            "blake2b(preimage) == prev_states[0].hashlock"
        );
    }

    #[test]
    fn ctor_emits_byte_arrays_not_hex_for_bytes32_and_pubkey() {
        // Regression: silverc rejects {"kind":"hex",...} ("unknown variant `hex`").
        // byte[32]/pubkey constructor args must be 32-element byte arrays; sig 64.
        let model = CovenantModel {
            name: "T".into(),
            params: vec![
                ("k".into(), Type::PubKey),
                ("h".into(), Type::Bytes32),
                ("s".into(), Type::Sig),
            ],
            state: vec![],
            transitions: vec![],
            has_vprog: false,
        };
        let (_, json) = emit_ctor(&model);
        assert!(
            !json.contains("\"hex\""),
            "must not emit hex kind: {}",
            json
        );
        // 32-byte arrays for pubkey + bytes32 = 64 byte entries; sig adds 64 more = 128.
        let byte_count = json.matches(r#"{"kind":"byte","data":0}"#).count();
        assert_eq!(byte_count, 32 + 32 + 64, "byte counts: {}", json);
    }

    #[test]
    fn object_return_emits_one_field_per_pair_via_ast() {
        // Multi-field object returns are now driven by the typed ReturnExpr::Object
        // (one (field, Expr) pair per declared state field). The value side is
        // lowered structurally; the head name is dropped in the emitted return.
        let ret = portrait_syntax::parse_return_expr(
            "EvidenceLineage { seq: seq + 1, subject: subject, commit: next_commit }",
        )
        .unwrap();
        let fields: HashSet<&str> = ["seq", "subject", "commit"].into_iter().collect();
        match ret {
            ReturnExpr::Object {
                fields: pairs,
                name,
            } => {
                assert_eq!(name.as_deref(), Some("EvidenceLineage"));
                assert_eq!(pairs.len(), 3);
                assert_eq!(pairs[0].0, "seq");
                assert_eq!(emit_expr(&pairs[0].1, &fields), "prev_states[0].seq + 1");
                assert_eq!(pairs[1].0, "subject");
                assert_eq!(emit_expr(&pairs[1].1, &fields), "prev_states[0].subject");
                assert_eq!(emit_expr(&pairs[2].1, &fields), "next_commit");
            }
            other => panic!("expected object return, got {other:?}"),
        }
        // A scalar expression parses as ReturnExpr::Scalar, not Object.
        assert!(matches!(
            portrait_syntax::parse_return_expr("value + delta").unwrap(),
            ReturnExpr::Scalar(_)
        ));
    }

    #[test]
    fn emit_lowers_coin_field_to_int_not_coin() {
        // Phase D1: a `coin` state field / param must lower to silverscript `int`
        // (silverc rejects a bare `coin` struct field). The conservation
        // semantics of `coin` live in the Portrait type checker, not the .sil.
        let model = CovenantModel {
            name: "CoinHolder".into(),
            params: vec![
                ("owner".into(), Type::PubKey),
                ("amount".into(), Type::Coin),
            ],
            state: vec![
                ("owner".into(), Type::PubKey),
                ("amount".into(), Type::Coin),
            ],
            transitions: vec![Transition {
                entry: "carry".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![],
                capability: None,
                args: vec![],
                body: vec![Stmt::Return(
                    portrait_syntax::parse_return_expr(
                        "CoinHolder { owner: owner, amount: amount }",
                    )
                    .unwrap(),
                )],
            }],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit");
        let src = &files[0].source;
        assert!(
            !src.contains("coin "),
            "coin must not appear as a silverscript type: {}",
            src
        );
        // Field/param declarations must use `int`, with the genesis param under
        // its distinct `init_` identifier (H1).
        assert!(
            src.contains("int amount = init_amount;"),
            "coin field must lower to int declaration: {}",
            src
        );
        assert!(
            src.contains(", int init_amount)"),
            "coin param must lower to int in the constructor: {}",
            src
        );
    }

    #[test]
    fn emit_type_coin_is_int() {
        assert_eq!(emit_type(&Type::Coin), "int");
    }

    /// Genesis initialisers bind by NAME. Declaring the params in the REVERSE of
    /// the field order must still initialise each field from its own param —
    /// under the old positional rule this silently crossed the two fields.
    #[test]
    fn ctor_init_binds_state_fields_by_name_not_position() {
        let model = CovenantModel {
            name: "Crossed".into(),
            params: vec![("paused".into(), Type::Int), ("balance".into(), Type::Int)],
            state: vec![("balance".into(), Type::Int), ("paused".into(), Type::Int)],
            transitions: vec![],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit");
        let src = &files[0].source;
        assert!(
            src.contains("int balance = init_balance;")
                && src.contains("int paused = init_paused;"),
            "each field must initialise from its OWN param, not the positional one: {}",
            src
        );
    }

    /// A state field with no same-named param has NO genesis value. It must be a
    /// loud error naming the field, never a silent `0`.
    #[test]
    fn ctor_init_errors_when_a_state_field_has_no_matching_param() {
        let model = CovenantModel {
            name: "Unbound".into(),
            params: vec![("balance".into(), Type::Int)],
            state: vec![("balance".into(), Type::Int), ("paused".into(), Type::Int)],
            transitions: vec![],
            has_vprog: false,
        };
        let err = emit(&[model]).expect_err("a field with no initialiser must fail loud");
        assert!(
            err.contains("`paused`") && err.contains("param int paused;"),
            "error must name the field and prescribe the param declaration: {}",
            err
        );
        assert!(
            err.contains("never silently binds a state field to 0"),
            "error must state the no-silent-zero rule: {}",
            err
        );
    }

    /// A same-named param of a different type is not a valid initialiser.
    #[test]
    fn ctor_init_errors_on_a_name_matched_param_of_a_different_type() {
        let model = CovenantModel {
            name: "Mistyped".into(),
            params: vec![("owner".into(), Type::Bytes32)],
            state: vec![("owner".into(), Type::PubKey)],
            transitions: vec![],
            has_vprog: false,
        };
        let err = emit(&[model]).expect_err("a type-mismatched initialiser must fail loud");
        assert!(
            err.contains("`owner`") && err.contains("pubkey") && err.contains("byte[32]"),
            "error must name the field, its type, and the param's type: {}",
            err
        );
    }

    /// Params beyond the state set stay legal — a policy param (a deadline, a
    /// rate, a key) need not have a state field of its own.
    #[test]
    fn ctor_init_permits_params_beyond_the_state_set() {
        let model = CovenantModel {
            name: "Policy".into(),
            params: vec![
                ("balance".into(), Type::Int),
                ("deadline".into(), Type::Int),
                ("rate".into(), Type::Int),
            ],
            state: vec![("balance".into(), Type::Int)],
            transitions: vec![],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("extra policy params are legal");
        let src = &files[0].source;
        assert!(
            src.contains(
                "contract Policy(int max_ins, int max_outs, int init_balance, int deadline, int rate)"
            ),
            "a state-backed param is emitted as `init_<field>`; a policy param keeps its \
             authored name: {}",
            src
        );
        assert!(
            src.contains("int balance = init_balance;") && !src.contains("deadline ="),
            "only state fields get field declarations: {}",
            src
        );
    }

    /// H1 — the emitted contract must never bind a state field under an
    /// identifier a constructor param already binds.
    ///
    /// silverscript's public `ContractAst::resolve_contract_state_values` (the
    /// API a deployer uses to compute an instance's concrete genesis state, and
    /// the one the upstream `cli-debugger` calls) hard-errors with `duplicate
    /// contract field name: <f>` on that collision, so a colliding emit is
    /// unresolvable by every consumer except `silverc`'s own compile path — which
    /// does not call it, which is why the collision compiled clean.
    ///
    /// UPSTREAM-API CHECK OUT OF REACH IN-PROCESS: asserting `Ok` from that API
    /// directly would need `silverscript-lang` as a dev-dependency, and it pins
    /// `kaspa-*` to a floating `rusty-kaspa` **pre-release branch** while this workspace pins
    /// **tag `v2.0.0`** — adding it would float the engine pin. So the invariant
    /// is asserted structurally here (param and field identifiers are disjoint),
    /// and the API itself was verified out-of-band against the upstream
    /// `cli-debugger`: the pre-H1 form returned
    /// `Unsupported("duplicate contract field name: value")`, the post-H1 form
    /// resolves.
    #[test]
    fn emitted_ctor_params_never_collide_with_state_field_names() {
        let model = CovenantModel {
            name: "Disjoint".into(),
            params: vec![
                ("balance".into(), Type::Int),
                ("owner".into(), Type::PubKey),
                ("deadline".into(), Type::Int),
            ],
            state: vec![
                ("balance".into(), Type::Int),
                ("owner".into(), Type::PubKey),
            ],
            transitions: vec![],
            has_vprog: false,
        };
        let idents = emitted_param_idents(&model);
        let files = emit(std::slice::from_ref(&model)).expect("emit");
        let src = &files[0].source;

        for (field, _) in &model.state {
            assert!(
                !idents.contains(field),
                "emitted ctor param `{}` collides with the state field of the same name — \
                 `resolve_contract_state_values` would reject the contract:\n{}",
                field,
                src
            );
        }
        assert_eq!(
            idents,
            vec!["init_balance", "init_owner", "deadline"],
            "state-backed params take the `init_` prefix; policy params do not"
        );
    }

    /// H1 collision fallback: an author who already has a param literally named
    /// `init_<field>` must still get a disjoint identifier, deterministically.
    #[test]
    fn emitted_ctor_param_disambiguates_against_an_authored_init_name() {
        let model = CovenantModel {
            name: "Clash".into(),
            params: vec![
                ("balance".into(), Type::Int),
                ("init_balance".into(), Type::Int),
            ],
            state: vec![("balance".into(), Type::Int)],
            transitions: vec![],
            has_vprog: false,
        };
        let idents = emitted_param_idents(&model);
        assert_eq!(
            idents,
            vec!["init__balance", "init_balance"],
            "the authored `init_balance` keeps its name; the genesis param moves aside"
        );
        let files = emit(&[model]).expect("emit");
        assert!(
            files[0].source.contains("int balance = init__balance;"),
            "the field must bind the disambiguated identifier:\n{}",
            files[0].source
        );
    }

    /// M4 — a duplicate param name makes the by-name genesis lookup ambiguous.
    /// Without this the `.find()` silently took the first, so a `pubkey balance`
    /// shadowing an `int balance` produced a MISLEADING type-mismatch diagnostic.
    #[test]
    fn emit_rejects_duplicate_constructor_param_names() {
        let model = CovenantModel {
            name: "Dup".into(),
            params: vec![
                ("balance".into(), Type::PubKey),
                ("balance".into(), Type::Int),
            ],
            state: vec![("balance".into(), Type::Int)],
            transitions: vec![],
            has_vprog: false,
        };
        let err = emit(&[model]).expect_err("a duplicate param must fail loud");
        assert!(
            err.contains("declared more than once") && err.contains("`balance`"),
            "error must name the duplicated param, not report a misleading type mismatch: {}",
            err
        );
    }

    /// L4 — a non-terminal `mode = verification` entrypoint is emitted with an
    /// EMPTY body: its guards are never lowered. Before this check, a
    /// `requires checkSig(auth, owner)` was silently dropped while `portrait
    /// check` reported ok — a covenant that LOOKS gated and enforces nothing,
    /// failing closed only by accident. Same class as the `Stmt::Raw` hole.
    #[test]
    fn emit_fails_loud_on_verification_entrypoint_whose_guards_would_be_dropped() {
        let model = CovenantModel {
            name: "FakeVerified".into(),
            params: vec![("owner".into(), Type::PubKey)],
            state: vec![("owner".into(), Type::PubKey)],
            transitions: vec![Transition {
                entry: "check_owner".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Verification,
                guards: vec![],
                capability: None,
                args: vec![("auth".into(), Type::Sig)],
                body: vec![Stmt::Require(
                    portrait_syntax::parse_expr("checkSig(auth, owner)").unwrap(),
                )],
            }],
            has_vprog: false,
        };
        let err =
            emit(&[model]).expect_err("dropped guards must fail loud, not emit an empty body");
        assert!(
            err.contains("checkSig(auth, owner)") && err.contains("SILENTLY DROPPED"),
            "error must name the guard that would be dropped: {}",
            err
        );
    }

    /// L4 control: a verification entrypoint with NO guards drops nothing, so it
    /// stays legal (the shape the emitter has always produced for it).
    #[test]
    fn emit_permits_a_guardless_verification_entrypoint() {
        let model = CovenantModel {
            name: "Inert".into(),
            params: vec![("owner".into(), Type::PubKey)],
            state: vec![("owner".into(), Type::PubKey)],
            transitions: vec![Transition {
                entry: "noop".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Verification,
                guards: vec![],
                capability: None,
                args: vec![],
                body: vec![],
            }],
            has_vprog: false,
        };
        emit(&[model]).expect("a guardless verification entrypoint drops nothing");
    }

    /// H2 — the CTOR JSON `emit_ctor` writes is an ALL-ZERO placeholder and the
    /// author must be told, every time.
    #[test]
    fn placeholder_ctor_warning_names_the_covenant_and_the_hazard() {
        let model = CovenantModel {
            name: "Vault".into(),
            params: vec![("owner".into(), Type::PubKey)],
            state: vec![("owner".into(), Type::PubKey)],
            transitions: vec![],
            has_vprog: false,
        };
        let (_, json) = emit_ctor(&model);
        assert!(
            json.contains(r#"{"kind":"byte","data":0}"#),
            "the pubkey arg is an all-zero placeholder: {json}"
        );
        let w = placeholder_ctor_warning(&model).expect("a covenant with params owes a warning");
        assert!(
            w.contains("Vault_ctor.json")
                && w.contains("ALL-ZERO PLACEHOLDER")
                && w.contains("KovId"),
            "warning must name the artifact, the hazard, and the KovId caveat: {w}"
        );
    }

    /// H2 control: a covenant with no user params defaulted nothing, so no
    /// warning is owed (silence must stay meaningful).
    #[test]
    fn no_placeholder_warning_when_there_are_no_user_params() {
        let model = CovenantModel {
            name: "Bare".into(),
            params: vec![],
            state: vec![],
            transitions: vec![],
            has_vprog: false,
        };
        assert!(placeholder_ctor_warning(&model).is_none());
    }

    #[test]
    fn emit_expr_preserves_authored_parens_for_precedence() {
        // Regression: emit_expr previously rendered Binary as "{lhs} {op} {rhs}"
        // with no parens, silently changing precedence when re-parsed by silverc.
        let fields: HashSet<&str> = HashSet::new();

        // (a + b) * c must keep the parens — Mul binds tighter than Add.
        let e = portrait_syntax::parse_expr("(a + b) * c").unwrap();
        assert_eq!(emit_expr(&e, &fields), "(a + b) * c");

        // a + b * c must NOT gain spurious parens — already correctly grouped.
        let e = portrait_syntax::parse_expr("a + b * c").unwrap();
        assert_eq!(emit_expr(&e, &fields), "a + b * c");

        // a - (b - c): right child is same-precedence under a left-assoc op, so
        // it must be parenthesized to preserve the authored grouping.
        let e = portrait_syntax::parse_expr("a - (b - c)").unwrap();
        assert_eq!(emit_expr(&e, &fields), "a - (b - c)");

        // a - b - c: left-assoc chain, parses as (a - b) - c; the left child is
        // same precedence but on the left, so NO parens needed.
        let e = portrait_syntax::parse_expr("a - b - c").unwrap();
        assert_eq!(emit_expr(&e, &fields), "a - b - c");

        // The exact reported repro must round-trip identically.
        let e = portrait_syntax::parse_expr("collateral >= (debt + amount) * min_ratio").unwrap();
        assert_eq!(
            emit_expr(&e, &fields),
            "collateral >= (debt + amount) * min_ratio"
        );

        // A comparison of arithmetic: comparison binds looser than +, so the
        // arithmetic children never need parens.
        let e = portrait_syntax::parse_expr("a + b >= c + d").unwrap();
        assert_eq!(emit_expr(&e, &fields), "a + b >= c + d");
    }

    #[test]
    fn emit_expr_preserves_parens_around_unary_operand() {
        // Regression (same family as the Binary precedence bug): a unary operator
        // binds tighter than every binary operator, so a `Binary` operand of a
        // `Unary` was only reachable via authored parentheses. Emitting it bare
        // (`-a + b`, `!a && b`) silently regroups it to `(-a) + b` / `(!a) && b`.
        let fields: HashSet<&str> = HashSet::new();

        // -(a + b) must keep its parens — else it re-parses as (-a) + b.
        let e = portrait_syntax::parse_expr("-(a + b)").unwrap();
        assert_eq!(emit_expr(&e, &fields), "-(a + b)");

        // !(a && b) must keep its parens — else it re-parses as (!a) && b.
        let e = portrait_syntax::parse_expr("!(a && b)").unwrap();
        assert_eq!(emit_expr(&e, &fields), "!(a && b)");

        // A non-Binary unary operand must NOT gain spurious parens.
        let e = portrait_syntax::parse_expr("-a").unwrap();
        assert_eq!(emit_expr(&e, &fields), "-a");
        let e = portrait_syntax::parse_expr("!flag").unwrap();
        assert_eq!(emit_expr(&e, &fields), "!flag");
    }

    #[test]
    fn emit_lowers_body_require_checksig_guard() {
        // Working-path guarantee: a body `requires checkSig(auth, owner)` clause
        // (parsed to Stmt::Require) MUST lower to a silverscript `require(...)`.
        // This is the path SimpleEscrow/OwnableCounter use; without it the
        // emitted covenant would carry state forward but enforce no authorization.
        let model = CovenantModel {
            name: "Guarded".into(),
            params: vec![("owner".into(), Type::PubKey)],
            state: vec![("owner".into(), Type::PubKey)],
            transitions: vec![Transition {
                entry: "act".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![],
                capability: None,
                args: vec![("auth".into(), Type::Sig)],
                body: vec![
                    Stmt::Require(portrait_syntax::parse_expr("checkSig(auth, owner)").unwrap()),
                    Stmt::Return(
                        portrait_syntax::parse_return_expr("Guarded { owner: owner }").unwrap(),
                    ),
                ],
            }],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit should succeed for a Require guard");
        let src = &files[0].source;
        assert!(
            src.contains("require(checkSig(auth, prev_states[0].owner));"),
            "body Require guard must lower to require(checkSig(...)): {}",
            src
        );
    }

    #[test]
    fn emit_lowers_pays_guard_to_output_introspection_requires() {
        // B2: a `pays(0, seller, amount)` clause (carried as Guard::OutputPays)
        // must lower to the two output-introspection requires that make consensus
        // enforce the payout — value against the committed amount, scriptPubKey
        // against the committed payee's P2PK spk.
        let model = CovenantModel {
            name: "Escrow".into(),
            params: vec![
                ("seller".into(), Type::PubKey),
                ("amount".into(), Type::Coin),
            ],
            state: vec![
                ("seller".into(), Type::PubKey),
                ("amount".into(), Type::Coin),
            ],
            transitions: vec![Transition {
                entry: "release".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![Guard::OutputPays {
                    index: 0,
                    to: "seller".into(),
                    amount: "amount".into(),
                }],
                capability: None,
                args: vec![("auth".into(), Type::Sig)],
                body: vec![
                    Stmt::Require(portrait_syntax::parse_expr("checkSig(auth, seller)").unwrap()),
                    Stmt::Pays {
                        index: 0,
                        payee: "seller".into(),
                        amount: "amount".into(),
                    },
                    Stmt::Return(
                        portrait_syntax::parse_return_expr(
                            "Escrow { seller: seller, amount: amount }",
                        )
                        .unwrap(),
                    ),
                ],
            }],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit should succeed for a pays guard");
        let src = &files[0].source;
        assert!(
            src.contains("require(tx.outputs[0].value == prev_states[0].amount);"),
            "amount binding missing: {src}"
        );
        assert!(
            src.contains(
                "require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_states[0].seller)));"
            ),
            "payee spk binding missing: {src}"
        );
    }

    /// An Escrow-shaped single-transition model whose `pays(0, seller, amount)`
    /// payee carries `payee_ty`, so a test can vary ONLY the payee's declared type.
    fn pays_payee_model(payee_ty: Type) -> CovenantModel {
        CovenantModel {
            name: "Escrow".into(),
            params: vec![
                ("seller".into(), payee_ty.clone()),
                ("amount".into(), Type::Coin),
            ],
            state: vec![("seller".into(), payee_ty), ("amount".into(), Type::Coin)],
            transitions: vec![Transition {
                entry: "release".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![Guard::OutputPays {
                    index: 0,
                    to: "seller".into(),
                    amount: "amount".into(),
                }],
                capability: None,
                args: vec![("auth".into(), Type::Sig)],
                body: vec![
                    Stmt::Pays {
                        index: 0,
                        payee: "seller".into(),
                        amount: "amount".into(),
                    },
                    Stmt::Return(
                        portrait_syntax::parse_return_expr(
                            "Escrow { seller: seller, amount: amount }",
                        )
                        .unwrap(),
                    ),
                ],
            }],
            has_vprog: false,
        }
    }

    #[test]
    fn pays_with_a_bytes32_payee_lowers_to_p2sh_spk() {
        // Type-directed dispatch: a `byte[32]` payee is a SCRIPT HASH, so the spk
        // require is built with `ScriptPubKeyP2SH` — no new `pays` syntax, no
        // `pays_p2sh` variant, no raw spk bytes in committed state.
        let files = emit(&[pays_payee_model(Type::Bytes32)]).expect("emit");
        let src = &files[0].source;
        assert!(
            src.contains(
                "require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2SH(prev_states[0].seller)));"
            ),
            "byte[32] payee must lower to a P2SH spk: {src}"
        );
        assert!(
            !src.contains("ScriptPubKeyP2PK"),
            "a byte[32] payee must not also emit a P2PK spk: {src}"
        );
    }

    #[test]
    fn pays_with_a_pubkey_payee_still_lowers_to_p2pk_spk() {
        // BYTE-IDENTITY: the dispatch must leave the three shipped `pays` clauses
        // (all `pubkey` payees) exactly as they were — zero churn on the catalogue.
        let files = emit(&[pays_payee_model(Type::PubKey)]).expect("emit");
        let src = &files[0].source;
        assert!(
            src.contains(
                "require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_states[0].seller)));"
            ),
            "pubkey payee must still lower to the unchanged P2PK spk: {src}"
        );
        assert!(
            !src.contains("ScriptPubKeyP2SH"),
            "a pubkey payee must not emit a P2SH spk: {src}"
        );
    }

    #[test]
    fn pays_with_a_non_address_typed_payee_is_an_emit_error() {
        // An `int` payee is not an address. Guessing either spk form would emit a
        // covenant whose payout require can never be satisfied, so emit must FAIL
        // loudly and name the payee.
        let err = emit(&[pays_payee_model(Type::Int)])
            .expect_err("a non-address payee must be an emit error");
        assert!(
            err.contains("payee `seller`") && err.contains("not an address"),
            "emit error must name the non-address payee: {err}"
        );
    }

    #[test]
    fn emit_lowers_after_guard_to_tx_time_cltv_require() {
        // B1: an `after(unlock_bucket)` clause (carried as Guard::TimeAtLeast)
        // must lower to a `require(tx.time >= prev_states[0].unlock_bucket);` — the
        // `tx.time` special TxVar silverc routes to OpCheckLockTimeVerify. A bare
        // `tx.locktime` compare (bypassable) must NOT appear.
        let model = CovenantModel {
            name: "Vault".into(),
            params: vec![
                ("owner".into(), Type::PubKey),
                ("unlock_bucket".into(), Type::Int),
            ],
            state: vec![
                ("owner".into(), Type::PubKey),
                ("unlock_bucket".into(), Type::Int),
            ],
            transitions: vec![Transition {
                entry: "release".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![Guard::TimeAtLeast {
                    deadline: AfterDeadline::Field("unlock_bucket".into()),
                }],
                capability: None,
                args: vec![("auth".into(), Type::Sig)],
                body: vec![
                    Stmt::Require(portrait_syntax::parse_expr("checkSig(auth, owner)").unwrap()),
                    Stmt::After {
                        deadline: AfterDeadline::Field("unlock_bucket".into()),
                    },
                    Stmt::Return(
                        portrait_syntax::parse_return_expr(
                            "Vault { owner: owner, unlock_bucket: unlock_bucket }",
                        )
                        .unwrap(),
                    ),
                ],
            }],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit should succeed for an after guard");
        let src = &files[0].source;
        assert!(
            src.contains("require(tx.time >= prev_states[0].unlock_bucket);"),
            "tx.time CLTV gate missing: {src}"
        );
        assert!(
            !src.contains("tx.locktime"),
            "must not emit the bypassable bare tx.locktime compare: {src}"
        );
    }

    #[test]
    fn emit_lowers_after_sum_guard_to_summed_tx_time_cltv_require() {
        // B1 (D1): an `after(last_charged + period)` window clause lowers to a
        // single `require(tx.time >= prev_states[0].last_charged +
        // prev_states[0].period);` — the SUM of the two committed atoms is the CLTV
        // threshold silverc routes to OpCheckLockTimeVerify.
        let model = CovenantModel {
            name: "Sub".into(),
            params: vec![
                ("last_charged".into(), Type::Int),
                ("period".into(), Type::Int),
            ],
            state: vec![
                ("last_charged".into(), Type::Int),
                ("period".into(), Type::Int),
            ],
            transitions: vec![Transition {
                entry: "charge".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![Guard::TimeAtLeast {
                    deadline: AfterDeadline::Sum("last_charged".into(), "period".into()),
                }],
                capability: None,
                args: vec![("auth".into(), Type::Sig)],
                body: vec![
                    Stmt::After {
                        deadline: AfterDeadline::Sum("last_charged".into(), "period".into()),
                    },
                    Stmt::Return(
                        portrait_syntax::parse_return_expr(
                            "Sub { last_charged: last_charged, period: period }",
                        )
                        .unwrap(),
                    ),
                ],
            }],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit should succeed for an after-sum guard");
        let src = &files[0].source;
        assert!(
            src.contains(
                "require(tx.time >= prev_states[0].last_charged + prev_states[0].period);"
            ),
            "summed tx.time CLTV gate missing: {src}"
        );
        assert!(
            !src.contains("tx.locktime"),
            "must not emit the bypassable bare tx.locktime compare: {src}"
        );
    }

    #[test]
    fn emit_fails_loud_on_raw_guard_statement() {
        // Soundness/honesty hazard: an unrecognised guard form (e.g. the `@` age
        // syntax) parses to Stmt::Raw and was previously SILENTLY DROPPED at emit,
        // producing a covenant that LOOKS gated but enforces nothing. Emit must
        // instead fail loud, naming the offending raw statement.
        let model = CovenantModel {
            name: "FakeGated".into(),
            params: vec![("v".into(), Type::Int)],
            state: vec![("v".into(), Type::Int)],
            transitions: vec![Transition {
                entry: "f".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![],
                capability: None,
                args: vec![],
                body: vec![
                    Stmt::Raw("requires v @ 1".into()),
                    Stmt::Return(portrait_syntax::parse_return_expr("v + 1").unwrap()),
                ],
            }],
            has_vprog: false,
        };
        let err = emit(&[model]).expect_err("emit must reject a Raw guard, not drop it");
        assert!(
            err.contains("v @ 1") && err.contains("FakeGated"),
            "error must name the offending raw guard and contract: {}",
            err
        );
    }

    #[test]
    fn emit_multi_field_transition_compiles_to_one_field_per_state() {
        // The emitted return must list each state field exactly once, with values
        // lowered, and must NOT nest the object literal inside a field value.
        let model = CovenantModel {
            name: "Lineage".into(),
            params: vec![("seq".into(), Type::Int), ("subject".into(), Type::Bytes32)],
            state: vec![("seq".into(), Type::Int), ("subject".into(), Type::Bytes32)],
            transitions: vec![Transition {
                entry: "attest".into(),
                from: "live".into(),
                to: Some("live".into()),
                mode: Mode::Transition,
                guards: vec![],
                capability: None,
                args: vec![("next_subject".into(), Type::Bytes32)],
                body: vec![Stmt::Return(
                    portrait_syntax::parse_return_expr(
                        "Lineage { seq: seq + 1, subject: next_subject }",
                    )
                    .unwrap(),
                )],
            }],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit");
        let src = &files[0].source;
        assert!(
            src.contains("return({ seq: prev_states[0].seq + 1, subject: next_subject });"),
            "expected flat one-field-per-state return: {}",
            src
        );
        assert!(
            !src.contains("seq: Lineage {"),
            "must not nest the object literal in a field value: {}",
            src
        );
    }

    #[test]
    fn emit_terminal_transition_emits_auth_verification_no_successor() {
        // B3: a TERMINAL transition (mode = transition, no successor `to`) emits a
        // `binding = auth` VERIFICATION function that RELEASES the coin via pays and
        // carries NO return/successor. State is read via the SINGULAR `prev_state`.
        let release = Transition {
            entry: "release".into(),
            from: "live".into(),
            to: None,
            mode: Mode::Transition,
            guards: vec![Guard::OutputPays {
                index: 0,
                to: "seller".into(),
                amount: "amount".into(),
            }],
            capability: None,
            args: vec![("auth".into(), Type::Sig)],
            body: vec![
                Stmt::Require(portrait_syntax::parse_expr("checkSig(auth, seller)").unwrap()),
                Stmt::Pays {
                    index: 0,
                    payee: "seller".into(),
                    amount: "amount".into(),
                },
            ],
        };
        let model = CovenantModel {
            name: "Escrow".into(),
            params: vec![
                ("seller".into(), Type::PubKey),
                ("amount".into(), Type::Coin),
            ],
            state: vec![
                ("seller".into(), Type::PubKey),
                ("amount".into(), Type::Coin),
            ],
            transitions: vec![release],
            has_vprog: false,
        };
        let files = emit(&[model]).expect("emit terminal transition");
        let src = &files[0].source;
        assert!(
            src.contains(
                "#[covenant(binding = auth, from = max_ins, to = 1, mode = verification)]"
            ),
            "terminal must emit binding = auth / mode = verification: {src}"
        );
        assert!(
            src.contains("function release(State prev_state, State[] new_states, sig auth) {"),
            "terminal signature must be `State prev_state, State[] new_states`: {src}"
        );
        assert!(
            src.contains("require(checkSig(auth, prev_state.seller));"),
            "body require must lower via the SINGULAR prev_state accessor: {src}"
        );
        assert!(
            src.contains("require(tx.outputs[0].value == prev_state.amount);"),
            "pays value require (singular accessor) missing: {src}"
        );
        assert!(
            src.contains(
                "require(tx.outputs[0].scriptPubKey == byte[](new ScriptPubKeyP2PK(prev_state.seller)));"
            ),
            "pays spk require (singular accessor) missing: {src}"
        );
        assert!(
            !src.contains("binding = cov"),
            "terminal must NOT use binding = cov: {src}"
        );
        assert!(
            !src.contains("return("),
            "terminal must NOT emit a successor return: {src}"
        );
        assert!(
            !src.contains("prev_states[0]"),
            "terminal must use the SINGULAR prev_state accessor, never prev_states[0]: {src}"
        );

        // terminal + after(...) lowers to the CLTV gate via the singular accessor.
        let refund = Transition {
            entry: "refund".into(),
            from: "live".into(),
            to: None,
            mode: Mode::Transition,
            guards: vec![Guard::TimeAtLeast {
                deadline: AfterDeadline::Field("deadline".into()),
            }],
            capability: None,
            args: vec![("auth".into(), Type::Sig)],
            body: vec![
                Stmt::Require(portrait_syntax::parse_expr("checkSig(auth, buyer)").unwrap()),
                Stmt::After {
                    deadline: AfterDeadline::Field("deadline".into()),
                },
            ],
        };
        let model = CovenantModel {
            name: "Escrow".into(),
            params: vec![
                ("buyer".into(), Type::PubKey),
                ("deadline".into(), Type::Int),
            ],
            state: vec![
                ("buyer".into(), Type::PubKey),
                ("deadline".into(), Type::Int),
            ],
            transitions: vec![refund],
            has_vprog: false,
        };
        let src = emit(&[model]).expect("emit terminal + after")[0]
            .source
            .clone();
        assert!(
            src.contains("require(tx.time >= prev_state.deadline);"),
            "terminal after(...) CLTV gate (singular accessor) missing: {src}"
        );

        // terminal + has_vprog is a fail-loud Err (a terminal spend has no successor
        // covenant to carry the proof-covenant-id binding).
        let vprog_terminal = CovenantModel {
            name: "Bad".into(),
            params: vec![("seller".into(), Type::PubKey)],
            state: vec![("seller".into(), Type::PubKey)],
            transitions: vec![Transition {
                entry: "release".into(),
                from: "live".into(),
                to: None,
                mode: Mode::Transition,
                guards: vec![],
                capability: None,
                args: vec![("auth".into(), Type::Sig)],
                body: vec![Stmt::Require(
                    portrait_syntax::parse_expr("checkSig(auth, seller)").unwrap(),
                )],
            }],
            has_vprog: true,
        };
        let err =
            emit(&[vprog_terminal]).expect_err("terminal + has_vprog must fail loud, not emit");
        assert!(
            err.contains("terminal transition cannot carry a vprog") && err.contains("Bad"),
            "vprog-terminal error must name the defect and contract: {err}"
        );
    }

    // =====================================================================
    // PROPERTY HARNESS (a) — PRECEDENCE / PAREN FAITHFULNESS
    //
    // The decisive automated answer to "how do you know the emitter is
    // faithful?". The three hand-fixed emitter-drift bugs (Binary precedence
    // regrouping; Unary-operand regrouping; — all in the PRIVATE `emit_expr`
    // below) belong to one class: the emitter erases a grouping the parser will
    // not re-derive. The general invariant that closes the entire class is a
    // round-trip:
    //
    //     parse_expr(emit_expr(e)) == e        for every well-typed Expr `e`.
    //
    // portrait-syntax's own parser uses the SAME Pratt precedence/associativity
    // as silverc (Mul=9, Add/Sub=7, cmp=5, And=3, Or=1, all left-assoc). The AST
    // has NO Paren node — authored parens vanish at parse — so a faithful emitter
    // must re-insert exactly the parens needed for the flat string to re-parse to
    // the SAME tree. If `emit_expr` drops a needed paren, the re-parse regroups
    // (e.g. `(a + b) * c` -> `a + b * c`) and the structural `==` FAILS.
    //
    // The generator below produces WELL-TYPED `Expr` ASTs DIRECTLY (never via a
    // string), so a parser bug cannot smuggle itself into the input. It is a
    // hand-rolled, depth-bounded, SEEDED (LCG) recursive builder — fully
    // deterministic (no proptest/quickcheck; the portrait workspace is
    // deliberately near-zero-dependency), so a failure reprints the exact seed +
    // AST and re-runs identically. It freely nests mixed-precedence Binary
    // children on BOTH sides and Binary operands under Unary — exactly the shapes
    // the three fixed bugs lived in.
    // =====================================================================

    /// Minimal deterministic LCG (Numerical Recipes constants). Seeded; no deps.
    struct Lcg(u64);
    impl Lcg {
        fn new(seed: u64) -> Self {
            // Avoid a degenerate all-zero state.
            Lcg(seed.wrapping_mul(6364136223846793005).wrapping_add(1))
        }
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }
        /// Uniform in `[0, n)` for small `n`.
        fn below(&mut self, n: u32) -> u32 {
            self.next_u32() % n
        }
        fn choice<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
            &xs[self.below(xs.len() as u32) as usize]
        }
    }

    /// The two value types the generator builds over. Comparisons consume `Int`
    /// and produce `Bool`, which keeps every generated tree well-typed.
    #[derive(Clone, Copy)]
    enum GenTy {
        Int,
        Bool,
    }

    /// Identifier pool. SOME of these are state fields (so the state-field
    /// lowering branch of `emit_expr` is exercised), some are plain params/args.
    /// The property uses an EMPTY field set so the round-trip is a pure
    /// precedence check; the lowering branch is exercised separately below.
    const INT_IDENTS: &[&str] = &["value", "delta", "seq", "amount", "limit", "x", "y"];
    const BOOL_IDENTS: &[&str] = &["flag", "allowed", "ok"];

    /// Build a well-typed `Expr` of the requested type, bounded by `depth`.
    fn gen_expr(rng: &mut Lcg, ty: GenTy, depth: u32) -> Expr {
        use portrait_syntax::{BinOp, UnOp};
        if depth == 0 {
            // Leaf.
            return match ty {
                GenTy::Int => {
                    if rng.below(2) == 0 {
                        // NON-NEGATIVE literals only: a literal `Int(-1)` and a
                        // `Unary{Neg, Int(1)}` render to the SAME string `-1`, so
                        // the parser cannot recover which one was authored — that
                        // is a generator ambiguity, not an emitter defect.
                        // Negation is already covered by the `Unary{Neg, ..}` arm.
                        Expr::Int(rng.below(20) as i64)
                    } else {
                        Expr::Var((*rng.choice(INT_IDENTS)).to_string())
                    }
                }
                GenTy::Bool => {
                    if rng.below(2) == 0 {
                        Expr::Bool(rng.below(2) == 0)
                    } else {
                        Expr::Var((*rng.choice(BOOL_IDENTS)).to_string())
                    }
                }
            };
        }
        match ty {
            GenTy::Int => {
                // Int recursive forms: binary arithmetic + unary negate.
                match rng.below(4) {
                    0 => {
                        let op = *rng.choice(&[BinOp::Add, BinOp::Sub, BinOp::Mul]);
                        Expr::Binary {
                            op,
                            lhs: Box::new(gen_expr(rng, GenTy::Int, depth - 1)),
                            rhs: Box::new(gen_expr(rng, GenTy::Int, depth - 1)),
                        }
                    }
                    1 => Expr::Unary {
                        op: UnOp::Neg,
                        rhs: Box::new(gen_expr(rng, GenTy::Int, depth - 1)),
                    },
                    // Bias toward more binary nesting (the precedence-heavy shape).
                    _ => {
                        let op = *rng.choice(&[BinOp::Add, BinOp::Sub, BinOp::Mul]);
                        Expr::Binary {
                            op,
                            lhs: Box::new(gen_expr(rng, GenTy::Int, depth - 1)),
                            rhs: Box::new(gen_expr(rng, GenTy::Int, depth - 1)),
                        }
                    }
                }
            }
            GenTy::Bool => {
                // Bool recursive forms: logical binary, comparison of Ints, unary not.
                match rng.below(4) {
                    0 => {
                        let op = *rng.choice(&[BinOp::And, BinOp::Or]);
                        Expr::Binary {
                            op,
                            lhs: Box::new(gen_expr(rng, GenTy::Bool, depth - 1)),
                            rhs: Box::new(gen_expr(rng, GenTy::Bool, depth - 1)),
                        }
                    }
                    1 => Expr::Unary {
                        op: UnOp::Not,
                        rhs: Box::new(gen_expr(rng, GenTy::Bool, depth - 1)),
                    },
                    // Comparisons consume Int, produce Bool — these mix the two
                    // sub-grammars and exercise comparison precedence (=5).
                    _ => {
                        let op = *rng.choice(&[
                            BinOp::Eq,
                            BinOp::Ne,
                            BinOp::Ge,
                            BinOp::Le,
                            BinOp::Gt,
                            BinOp::Lt,
                        ]);
                        Expr::Binary {
                            op,
                            lhs: Box::new(gen_expr(rng, GenTy::Int, depth - 1)),
                            rhs: Box::new(gen_expr(rng, GenTy::Int, depth - 1)),
                        }
                    }
                }
            }
        }
    }

    /// Number of generated cases per (type, seed) — kept hermetic + fast.
    const GEN_SEEDS: u64 = 400;
    const GEN_DEPTH: u32 = 5;

    #[test]
    fn property_a_emit_expr_roundtrips_modulo_parens() {
        // THE key property. For hundreds of generated well-typed Exprs:
        //   parse_expr(emit_expr(e)) == e.
        // An EMPTY field set makes this a pure precedence/paren faithfulness
        // check (no Var is rewritten), so any failure is a grouping defect.
        let fields: HashSet<&str> = HashSet::new();
        let mut checked = 0u64;
        for seed in 0..GEN_SEEDS {
            // Alternate the top-level type so both sub-grammars are rooted.
            let ty = if seed % 2 == 0 {
                GenTy::Int
            } else {
                GenTy::Bool
            };
            let mut rng = Lcg::new(seed);
            let e = gen_expr(&mut rng, ty, GEN_DEPTH);
            let rendered = emit_expr(&e, &fields);
            let reparsed = portrait_syntax::parse_expr(&rendered).unwrap_or_else(|err| {
                panic!(
                    "seed {seed}: emitted expr did not re-parse.\n  rendered: {rendered}\n  ast: {e:?}\n  err: {err}"
                )
            });
            assert_eq!(
                reparsed, e,
                "seed {seed}: ROUND-TRIP FAIL — emit_expr dropped/added a grouping.\n  \
                 rendered: {rendered}\n  original: {e:?}\n  reparsed: {reparsed:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, GEN_SEEDS, "all generated cases must be checked");
    }

    #[test]
    fn property_a_also_holds_with_state_field_lowering() {
        // Same round-trip, but now a NON-EMPTY field set so the state-field
        // lowering branch (`Var(name)` -> `prev_states[0].field`) is exercised.
        // After lowering, the field reads become `Field`/`Index`-free
        // `prev_states[0].name` accessors; re-parsing must STILL recover an
        // equivalent tree (with the lowered Var replaced by its Field form), so
        // we compare the re-parse against a separately-lowered reference tree.
        let fields: HashSet<&str> = ["value", "seq", "amount", "flag", "allowed"]
            .into_iter()
            .collect();
        for seed in 0..GEN_SEEDS {
            let ty = if seed % 2 == 0 {
                GenTy::Int
            } else {
                GenTy::Bool
            };
            let mut rng = Lcg::new(seed.wrapping_add(1_000_000));
            let e = gen_expr(&mut rng, ty, GEN_DEPTH);
            let rendered = emit_expr(&e, &fields);
            let reparsed = portrait_syntax::parse_expr(&rendered).unwrap_or_else(|err| {
                panic!("seed {seed}: lowered expr did not re-parse: {rendered}\n  err: {err}")
            });
            // The reference is `e` with each state-field Var rewritten to the
            // `prev_states[0].field` AST shape the emitter produces — i.e. the
            // round-trip must be the IDENTITY on the lowered tree.
            let expected = lower_field_vars(&e, &fields);
            assert_eq!(
                reparsed, expected,
                "seed {seed}: lowered round-trip FAIL.\n  rendered: {rendered}\n  \
                 expected: {expected:?}\n  reparsed: {reparsed:?}"
            );
        }
    }

    /// Reference lowering: rewrite each state-field `Var(name)` to the
    /// `prev_states[0].name` `Field`-over-`Index` AST the emitter renders, so the
    /// round-trip can be checked as an exact identity on the lowered tree.
    fn lower_field_vars(e: &Expr, fields: &HashSet<&str>) -> Expr {
        match e {
            Expr::Var(name) if fields.contains(name.as_str()) => Expr::Field {
                base: Box::new(Expr::Index {
                    base: Box::new(Expr::Var("prev_states".into())),
                    index: Box::new(Expr::Int(0)),
                }),
                field: name.clone(),
            },
            Expr::Var(_) | Expr::Int(_) | Expr::Bool(_) | Expr::Bytes(_) => e.clone(),
            Expr::Field { base, field } => Expr::Field {
                base: Box::new(lower_field_vars(base, fields)),
                field: field.clone(),
            },
            Expr::Index { base, index } => Expr::Index {
                base: Box::new(lower_field_vars(base, fields)),
                index: Box::new(lower_field_vars(index, fields)),
            },
            Expr::Unary { op, rhs } => Expr::Unary {
                op: *op,
                rhs: Box::new(lower_field_vars(rhs, fields)),
            },
            Expr::Binary { op, lhs, rhs } => Expr::Binary {
                op: *op,
                lhs: Box::new(lower_field_vars(lhs, fields)),
                rhs: Box::new(lower_field_vars(rhs, fields)),
            },
            Expr::Call { name, args } => Expr::Call {
                name: name.clone(),
                args: args.iter().map(|a| lower_field_vars(a, fields)).collect(),
            },
        }
    }

    // =====================================================================
    // NON-VACUITY — proof the property (a) harness has TEETH.
    //
    // If property (a) passed for a no-op or a broken emitter it would be
    // worthless. The OLD buggy emitter rendered Binary BARE (`{lhs} {op} {rhs}`,
    // no precedence parens) and Unary bare — which is EXACTLY what
    // `Expr::to_silverscript` (portrait-syntax) STILL does today. So we do not
    // need to resurrect the bug: we run property (a)'s round-trip with
    // `to_silverscript` substituted for `emit_expr`, over fixed adversarial trees
    // the generator is guaranteed to produce, and assert the property is
    // VIOLATED for at least one. This proves the harness distinguishes the FIXED
    // emitter from the buggy renderer.
    // =====================================================================

    #[test]
    fn non_vacuity_property_a_rejects_paren_dropping_renderer() {
        // Adversarial trees: each has an authored grouping a bare renderer drops.
        let adversarial = [
            "(a + b) * c", // Mul under Add — left child needs parens
            "a - (b - c)", // same-prec right child under left-assoc Sub
            "-(a + b)",    // Binary operand under Unary Neg
            "!(a && b)",   // Binary operand under Unary Not
        ];
        let mut violations = 0;
        for src in adversarial {
            let e = portrait_syntax::parse_expr(src).expect("adversarial src parses");
            // BUGGY renderer: bare, no precedence parens (== the old emitter).
            let buggy = e.to_silverscript();
            let reparsed = portrait_syntax::parse_expr(&buggy).expect("buggy render re-parses");
            if reparsed != e {
                violations += 1;
            }
            // And the FIXED emitter must PASS the very same round-trip — proving
            // the contrast is real, not an artifact of the trees.
            let fixed = emit_expr(&e, &HashSet::new());
            let fixed_reparsed =
                portrait_syntax::parse_expr(&fixed).expect("fixed render re-parses");
            assert_eq!(
                fixed_reparsed, e,
                "the FIXED emitter must round-trip `{src}` (rendered `{fixed}`)"
            );
        }
        assert!(
            violations > 0,
            "NON-VACUITY FAIL: the property did not reject the paren-dropping \
             renderer on any adversarial tree — it would have no teeth."
        );
    }

    #[test]
    fn non_vacuity_generator_emits_precedence_sensitive_shapes() {
        // Guard against a generator that only emits trivial leaves (which would
        // make property (a) vacuously true). Over the generated corpus, at least
        // one rendered tree must DIFFER from its bare `to_silverscript` form —
        // i.e. the generator actually produces shapes that NEED precedence parens.
        let fields: HashSet<&str> = HashSet::new();
        let mut needed_parens = 0;
        for seed in 0..GEN_SEEDS {
            let ty = if seed % 2 == 0 {
                GenTy::Int
            } else {
                GenTy::Bool
            };
            let mut rng = Lcg::new(seed);
            let e = gen_expr(&mut rng, ty, GEN_DEPTH);
            if emit_expr(&e, &fields) != e.to_silverscript() {
                needed_parens += 1;
            }
        }
        assert!(
            needed_parens > 0,
            "generator never produced a precedence-sensitive shape — property (a) \
             would be vacuous (needed_parens=0 of {GEN_SEEDS})"
        );
    }
}
