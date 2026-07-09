//! Pounce allocation pass (BUILD_SPEC §5): classifies each Cartoon IR
//! transition into a Covenant layer (silverscript) or VProg layer (CSCI prover).
//!
//! Classification rules (M0):
//! - `Mode::Transition` with `to = Some(_)` → Covenant (produces constrained output state)
//! - `Mode::Transition` with `to = None`    → VProg   (terminal; no covenant output)
//! - `Mode::Verification`                   → Covenant (state verification only)
//! - `Mode::NonCovenant`                    → VProg   (explicitly off-chain)

use portrait_ir::{Cartoon, Mode, Transition};

/// The allocated layer for a single transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer {
    Covenant,
    VProg,
}

/// A single transition annotated with its allocation.
#[derive(Debug, Clone)]
pub struct AllocatedTransition<'a> {
    pub transition: &'a Transition,
    pub layer: Layer,
}

/// The result of the Pounce pass over a whole Cartoon.
#[derive(Debug, Clone)]
pub struct PounceResult<'a> {
    pub cartoon: &'a Cartoon,
    pub allocations: Vec<AllocatedTransition<'a>>,
}

/// Run the Pounce allocation pass over a Cartoon.
pub fn allocate(cartoon: &Cartoon) -> PounceResult<'_> {
    let allocations = cartoon
        .roles
        .iter()
        .flat_map(|rg| rg.transitions.iter())
        .map(|tr| AllocatedTransition {
            layer: classify(tr),
            transition: tr,
        })
        .collect();
    PounceResult {
        cartoon,
        allocations,
    }
}

fn classify(tr: &Transition) -> Layer {
    match tr.mode {
        Mode::Transition => {
            if tr.to.is_some() {
                Layer::Covenant
            } else {
                Layer::VProg
            }
        }
        Mode::Verification => Layer::Covenant,
        Mode::NonCovenant => Layer::VProg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portrait_ir::{Cartoon, Mode, RoleGraph, StateNode, Transition};

    fn make_transition(entry: &str, mode: Mode, to: Option<&str>) -> Transition {
        Transition {
            entry: entry.into(),
            from: "S0".into(),
            to: to.map(|s| s.into()),
            mode,
            guards: vec![],
            capability: None,
            args: vec![],
            body: vec![],
        }
    }

    fn make_cartoon(transitions: Vec<Transition>) -> Cartoon {
        Cartoon {
            app: "Test".into(),
            roles: vec![RoleGraph {
                role: "R".into(),
                params: vec![],
                states: vec![StateNode {
                    label: "S0".into(),
                    fields: vec![],
                }],
                transitions,
                channels: vec![],
            }],
            invariants: vec![],
        }
    }

    #[test]
    fn pure_covenant_transition_with_to() {
        let cartoon = make_cartoon(vec![make_transition("step", Mode::Transition, Some("S1"))]);
        let result = allocate(&cartoon);
        assert_eq!(result.allocations.len(), 1);
        assert_eq!(result.allocations[0].layer, Layer::Covenant);
    }

    #[test]
    fn pure_vprog_non_covenant() {
        let cartoon = make_cartoon(vec![make_transition(
            "offchain_verify",
            Mode::NonCovenant,
            None,
        )]);
        let result = allocate(&cartoon);
        assert_eq!(result.allocations.len(), 1);
        assert_eq!(result.allocations[0].layer, Layer::VProg);
    }

    #[test]
    fn mixed_allocations() {
        let cartoon = make_cartoon(vec![
            make_transition("step", Mode::Transition, Some("S1")),
            make_transition("offchain", Mode::NonCovenant, None),
            make_transition("burn", Mode::Transition, None),
        ]);
        let result = allocate(&cartoon);
        assert_eq!(result.allocations.len(), 3);
        assert_eq!(result.allocations[0].layer, Layer::Covenant); // step
        assert_eq!(result.allocations[1].layer, Layer::VProg); // offchain
        assert_eq!(result.allocations[2].layer, Layer::VProg); // burn (no `to`)
    }
}
