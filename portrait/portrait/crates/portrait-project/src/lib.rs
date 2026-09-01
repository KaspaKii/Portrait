//! Pounce: projection of the global app into per-role covenant models (BUILD_SPEC §5).

use portrait_ir::{Cartoon, CovenantModel, Mode};

/// Project a Cartoon graph into one covenant model per role.
/// For single-role apps the model takes the app name; multi-role apps suffix the role name.
pub fn project(cartoon: &Cartoon) -> Vec<CovenantModel> {
    let single_role = cartoon.roles.len() == 1;
    cartoon
        .roles
        .iter()
        .map(|role| {
            // Union of state fields across all state nodes (de-duplicated).
            let mut state: Vec<(String, portrait_syntax::Type)> = Vec::new();
            for node in &role.states {
                for field in &node.fields {
                    if !state.iter().any(|(n, _)| n == &field.0) {
                        state.push(field.clone());
                    }
                }
            }

            let name = if single_role {
                cartoon.app.clone()
            } else {
                format!("{}{}", cartoon.app, pascal_case(&role.role))
            };

            // Detect whether this role has any VProg (NonCovenant) transitions.
            let has_vprog = role
                .transitions
                .iter()
                .any(|t| matches!(t.mode, Mode::NonCovenant));

            CovenantModel {
                name,
                params: role.params.clone(),
                state,
                transitions: role.transitions.clone(),
                has_vprog,
            }
        })
        .collect()
}

fn pascal_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for ch in s.chars() {
        if ch == '_' {
            cap = true;
        } else if cap {
            out.extend(ch.to_uppercase());
            cap = false;
        } else {
            out.push(ch);
        }
    }
    out
}
