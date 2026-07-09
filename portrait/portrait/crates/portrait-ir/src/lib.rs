//! Cartoon IR: the typed resource-transition graph (BUILD_SPEC §4), plus the
//! covenant-model and emitted-file types shared by projection and emission.

use portrait_syntax::{CovenantMode, Invariant, Program, Type};

#[derive(Debug, Clone)]
pub struct Cartoon {
    pub app: String,
    pub roles: Vec<RoleGraph>,
    pub invariants: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RoleGraph {
    pub role: String,
    pub params: Vec<(String, Type)>,
    pub states: Vec<StateNode>,
    pub transitions: Vec<Transition>,
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone)]
pub struct StateNode {
    pub label: String,
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone)]
pub struct Transition {
    pub entry: String,
    pub from: String,
    pub to: Option<String>,
    pub mode: Mode,
    pub guards: Vec<Guard>,
    pub capability: Option<String>,
    /// Entrypoint arguments beyond `State[] prev_states` (M1+).
    pub args: Vec<(String, Type)>,
    /// Entrypoint body statements (M1+).
    pub body: Vec<portrait_syntax::Stmt>,
}

#[derive(Debug, Clone)]
pub enum Mode {
    Transition,
    Verification,
    NonCovenant,
}

#[derive(Debug, Clone)]
pub enum Guard {
    Sig {
        key: String,
    },
    AgeAtLeast(i64),
    Eq(String, String),
    OutputPays {
        index: usize,
        to: String,
        amount: String,
    },
    Custom(String),
}

/// A lineage edge to another role, realised on-chain as covenant-ID inheritance.
#[derive(Debug, Clone)]
pub struct Channel {
    pub to_role: String,
    pub authorizing_entry: String,
}

/// Per-role result of projection: one silverscript contract's worth of model.
#[derive(Debug, Clone)]
pub struct CovenantModel {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub state: Vec<(String, Type)>,
    pub transitions: Vec<Transition>,
    /// True when this role has at least one VProg (NonCovenant) transition — i.e.
    /// the portrait file produces both a .sil covenant and a RISC Zero guest for the
    /// same role. When set, the engraver adds a `proof_cov_id` arg + OpInputCovenantId
    /// binding check to covenant transition functions.
    pub has_vprog: bool,
}

/// An emitted silverscript file.
#[derive(Debug, Clone)]
pub struct SilFile {
    pub name: String,
    pub source: String,
}

/// Lower a parsed program into the Cartoon IR (BUILD_SPEC §4).
pub fn lower(program: &Program) -> Cartoon {
    let mut roles = Vec::new();

    for role in &program.app.roles {
        let params: Vec<(String, Type)> = role
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect();

        let role_fields: Vec<(String, Type)> = role
            .state
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();

        // Collect unique state labels from lifecycle edges for this role.
        let mut seen_labels: Vec<String> = Vec::new();
        for edge in &program.app.lifecycle {
            if edge.via_role == role.name {
                for label in [&edge.from, &edge.to] {
                    if !seen_labels.contains(label) {
                        seen_labels.push(label.clone());
                    }
                }
            }
        }
        if seen_labels.is_empty() {
            seen_labels.push("init".to_string());
        }

        let states: Vec<StateNode> = seen_labels
            .iter()
            .cloned()
            .map(|label| StateNode {
                label,
                fields: role_fields.clone(),
            })
            .collect();

        // Collect lifecycle-declared transitions.
        let lifecycle_entries: Vec<&str> = program
            .app
            .lifecycle
            .iter()
            .filter(|e| e.via_role == role.name)
            .map(|e| e.via_entry.as_str())
            .collect();

        let mut transitions: Vec<Transition> = program
            .app
            .lifecycle
            .iter()
            .filter(|edge| edge.via_role == role.name)
            .map(|edge| {
                let entry_def = role.entrypoints.iter().find(|e| e.name == edge.via_entry);
                let mode = entry_def
                    .map(|e| match e.mode {
                        CovenantMode::Transition => Mode::Transition,
                        CovenantMode::Verification => Mode::Verification,
                        CovenantMode::NonCovenant => Mode::NonCovenant,
                    })
                    .unwrap_or(Mode::Transition);
                let args: Vec<(String, Type)> = entry_def
                    .map(|e| {
                        e.args
                            .iter()
                            .map(|p| (p.name.clone(), p.ty.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let body = entry_def.map(|e| e.body.clone()).unwrap_or_default();
                Transition {
                    entry: edge.via_entry.clone(),
                    from: edge.from.clone(),
                    to: if edge.terminal {
                        None
                    } else {
                        Some(edge.to.clone())
                    },
                    mode,
                    guards: Vec::new(),
                    capability: None,
                    args,
                    body,
                }
            })
            .collect();

        // Also include NonCovenant entrypoints NOT in the lifecycle.
        // These are VProg-only computations (off-L1 logic settled via STARK).
        for ep in &role.entrypoints {
            if matches!(ep.mode, CovenantMode::NonCovenant)
                && !lifecycle_entries.contains(&ep.name.as_str())
            {
                // `from` is a sentinel for NonCovenant transitions — VProg transitions have no
                // UTXO "from" state; the value is never used by Pounce or Atelier but must
                // satisfy the field.  M2 will use Option<String> here.
                let from_state = seen_labels
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "init".to_string());
                transitions.push(Transition {
                    entry: ep.name.clone(),
                    from: from_state,
                    to: None,
                    mode: Mode::NonCovenant,
                    guards: Vec::new(),
                    capability: None,
                    args: ep
                        .args
                        .iter()
                        .map(|p| (p.name.clone(), p.ty.clone()))
                        .collect(),
                    body: ep.body.clone(),
                });
            }
        }

        roles.push(RoleGraph {
            role: role.name.clone(),
            params,
            states,
            transitions,
            channels: Vec::new(),
        });
    }

    let invariants: Vec<String> = program
        .app
        .invariants
        .iter()
        .map(|inv| match inv {
            Invariant::ValueConserved => "value_conserved".to_string(),
            Invariant::NoUndeclaredState => "no_undeclared_state".to_string(),
            Invariant::Custom(s) => s.clone(),
        })
        .collect();

    Cartoon {
        app: program.app.name.clone(),
        roles,
        invariants,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portrait_syntax::parse;

    #[test]
    fn lower_threads_args() {
        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> live via counter.bump; }
  invariant no_undeclared_state;
}
"#;
        let program = parse(src).unwrap();
        let cartoon = lower(&program);
        let role = &cartoon.roles[0];
        let tr = &role.transitions[0];
        assert_eq!(tr.args.len(), 1, "bump should have 1 arg");
        assert_eq!(tr.args[0].0, "delta");
        assert_eq!(tr.args[0].1, Type::Int);
        assert_eq!(tr.body.len(), 1, "bump should have 1 body stmt");
    }

    /// B2: the IR threads the *typed* `Stmt`/`Expr` AST from B1 through unchanged —
    /// the body is a structured `Return(Scalar(Binary { Add, .. }))`, not a raw string,
    /// and is not a `Stmt::Raw` fallback.
    #[test]
    fn lower_threads_typed_stmt_ast() {
        use portrait_syntax::{BinOp, Expr, ReturnExpr, Stmt};

        let src = r#"
pragma portrait ^0.1.0;
app Counter {
  role counter {
    param int start;
    state { int value; }
    #[covenant(mode = transition)]
    entrypoint function bump(int delta) : (int value) {
      return value + delta;
    }
  }
  lifecycle { live -> live via counter.bump; }
  invariant no_undeclared_state;
}
"#;
        let program = parse(src).unwrap();
        let cartoon = lower(&program);
        let tr = &cartoon.roles[0].transitions[0];

        assert!(
            !tr.body.iter().any(|s| matches!(s, Stmt::Raw(_))),
            "body must be typed, not a Raw fallback"
        );
        match &tr.body[0] {
            Stmt::Return(ReturnExpr::Scalar(Expr::Binary { op, lhs, rhs })) => {
                assert_eq!(*op, BinOp::Add);
                assert_eq!(**lhs, Expr::Var("value".to_string()));
                assert_eq!(**rhs, Expr::Var("delta".to_string()));
            }
            other => panic!("expected typed Return(Scalar(Binary)), got {other:?}"),
        }
    }
}
