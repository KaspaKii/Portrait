//! portrait-plan — Provenance + Easel (BUILD_SPEC: covenant-ID lineage / tx templates).
//!
//! A multi-role Portrait app projects to **more than one** on-chain covenant
//! (one `.sil` per role, via `portrait-project` → `portrait-emit`). Those
//! covenants are not independent: a child covenant can bind a *parent*
//! covenant's ID (`require(parent_kov_id == OpInputCovenantId(0))`), so the
//! child may only ever fire in a transaction that also spends the parent.
//!
//! That relationship is **covenant-ID lineage**. This crate models it as a
//! directed graph of [`LineageEdge`]s over [`Covenant`] nodes, and produces a
//! [`DeployManifest`] — a topologically ordered, self-describing plan a wallet
//! can follow to deploy the role set in the correct order (a parent must be
//! deployed, and its covenant ID known, before any child that binds it).
//!
//! This is the "Easel": the smallest honest tx-template/manifest layer. It does
//! not move coins or sign — it records *what* to deploy, in *what* order, and
//! *which* parent ID each child must be seeded with. The wallet does the rest.
//!
//! Scope (honest): lineage edges are asserted by the app author (mirroring the
//! `parent_kov_id == OpInputCovenantId(0)` guard the engraver emits). This crate
//! validates the *structure* of the plan (no unknown roles, no cycles, single
//! root) and serialises it; it does not itself read the chain or verify a live
//! covenant ID. A parent's concrete `kov_id` is filled in by the deployer after
//! the parent is mined (see [`Covenant::with_kov_id`]).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

/// A single on-chain covenant in a multi-role app — one emitted `.sil` contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Covenant {
    /// Emitted contract name, e.g. `DigitalReitToken` (matches the `.sil`).
    pub name: String,
    /// The Portrait role this covenant was projected from, e.g. `token`.
    pub role: String,
    /// Concrete 32-byte covenant ID, once known (hex, no `0x`). `None` until the
    /// covenant is deployed and its ID observed on-chain.
    pub kov_id: Option<String>,
}

impl Covenant {
    /// Construct a covenant node with no known ID yet (pre-deploy).
    pub fn new(name: impl Into<String>, role: impl Into<String>) -> Self {
        Covenant {
            name: name.into(),
            role: role.into(),
            kov_id: None,
        }
    }

    /// Return a copy with the concrete covenant ID filled in (post-deploy).
    pub fn with_kov_id(mut self, kov_id: impl Into<String>) -> Self {
        self.kov_id = Some(kov_id.into());
        self
    }
}

/// A covenant-ID lineage edge: `child` binds `parent`'s covenant ID on-chain.
///
/// On-chain this is realised by the child covenant's
/// `require(parent_kov_id == OpInputCovenantId(0))` guard — the parent's UTXO
/// must be the spending input at index 0 of any transaction that fires the
/// child. The edge therefore points parent → child (deploy order direction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageEdge {
    /// Covenant whose ID is bound (deployed first).
    pub parent: String,
    /// Covenant that binds the parent's ID (deployed after the parent).
    pub child: String,
    /// The child state field that holds the parent's covenant ID.
    pub binding_field: String,
}

impl LineageEdge {
    /// Construct a lineage edge `parent → child` via `binding_field`.
    pub fn new(
        parent: impl Into<String>,
        child: impl Into<String>,
        binding_field: impl Into<String>,
    ) -> Self {
        LineageEdge {
            parent: parent.into(),
            child: child.into(),
            binding_field: binding_field.into(),
        }
    }
}

/// Errors a lineage plan can fail validation with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// An edge references a covenant name that is not in the covenant set.
    UnknownCovenant(String),
    /// The lineage graph has a cycle (a covenant cannot depend on itself,
    /// transitively); there is no valid deploy order.
    Cycle,
    /// The app declares no covenants.
    Empty,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::UnknownCovenant(n) => {
                write!(f, "lineage edge references unknown covenant `{n}`")
            }
            PlanError::Cycle => write!(f, "lineage graph has a cycle; no valid deploy order"),
            PlanError::Empty => write!(f, "app declares no covenants"),
        }
    }
}

impl std::error::Error for PlanError {}

/// A multi-role app's covenant set plus its lineage edges — the object the
/// [`DeployManifest`] is derived from.
#[derive(Debug, Clone)]
pub struct LineagePlan {
    /// App name (the Portrait `app` identifier).
    pub app: String,
    /// Every covenant the app projects to.
    pub covenants: Vec<Covenant>,
    /// Covenant-ID lineage edges (parent → child).
    pub edges: Vec<LineageEdge>,
}

impl LineagePlan {
    /// Construct a plan from an app name, its covenants, and its lineage edges.
    pub fn new(app: impl Into<String>, covenants: Vec<Covenant>, edges: Vec<LineageEdge>) -> Self {
        LineagePlan {
            app: app.into(),
            covenants,
            edges,
        }
    }

    /// Compute the deploy order: a topological sort of the lineage graph so that
    /// every parent precedes its children. Returns covenant names in order.
    ///
    /// Errors if an edge references an unknown covenant, the graph is empty, or
    /// the graph contains a cycle (no valid deploy order exists).
    pub fn deploy_order(&self) -> Result<Vec<String>, PlanError> {
        if self.covenants.is_empty() {
            return Err(PlanError::Empty);
        }
        let names: BTreeSet<&str> = self.covenants.iter().map(|c| c.name.as_str()).collect();

        // Validate edge endpoints and build the in-degree / adjacency structures.
        let mut indegree: HashMap<&str, usize> = names.iter().map(|n| (*n, 0)).collect();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for e in &self.edges {
            if !names.contains(e.parent.as_str()) {
                return Err(PlanError::UnknownCovenant(e.parent.clone()));
            }
            if !names.contains(e.child.as_str()) {
                return Err(PlanError::UnknownCovenant(e.child.clone()));
            }
            adj.entry(e.parent.as_str())
                .or_default()
                .push(e.child.as_str());
            *indegree.get_mut(e.child.as_str()).unwrap() += 1;
        }

        // Kahn's algorithm. Iterate roots in deterministic (sorted) order so the
        // manifest is reproducible regardless of input ordering.
        let mut ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(n, _)| *n)
            .collect();
        ready.sort_unstable();

        let mut order: Vec<String> = Vec::with_capacity(self.covenants.len());
        while let Some(node) = ready.pop() {
            order.push(node.to_string());
            if let Some(children) = adj.get(node) {
                let mut newly_ready: Vec<&str> = Vec::new();
                for &child in children {
                    let d = indegree.get_mut(child).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        newly_ready.push(child);
                    }
                }
                newly_ready.sort_unstable();
                // Push descending so the smallest is popped first (stable order).
                for child in newly_ready.into_iter().rev() {
                    ready.push(child);
                }
                ready.sort_unstable();
            }
        }

        if order.len() != self.covenants.len() {
            return Err(PlanError::Cycle);
        }
        Ok(order)
    }

    /// Build the deploy manifest: the validated, ordered plan a wallet follows.
    pub fn manifest(&self) -> Result<DeployManifest, PlanError> {
        let order = self.deploy_order()?;
        Ok(DeployManifest {
            app: self.app.clone(),
            order,
            covenants: self.covenants.clone(),
            edges: self.edges.clone(),
        })
    }
}

/// A validated, ordered deploy plan — the "Easel" output. Serialisable to JSON
/// for a wallet/operator to follow.
#[derive(Debug, Clone)]
pub struct DeployManifest {
    /// App name.
    pub app: String,
    /// Deploy order (parents before children); covenant names.
    pub order: Vec<String>,
    /// Covenant set (carries any known covenant IDs).
    pub covenants: Vec<Covenant>,
    /// Lineage edges, for the operator to seed child `parent_kov_id` fields.
    pub edges: Vec<LineageEdge>,
}

impl DeployManifest {
    /// Serialise the manifest to a compact, stable JSON string (hand-rolled, no
    /// external deps). Shape:
    /// ```json
    /// {
    ///   "app": "...",
    ///   "deploy_order": ["Parent", "Child"],
    ///   "covenants": [{"name":"...","role":"...","kov_id":null}],
    ///   "lineage": [{"parent":"...","child":"...","binding_field":"..."}]
    /// }
    /// ```
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        s.push_str("{\n");
        let _ = writeln!(s, "  \"app\": {},", json_str(&self.app));

        // deploy_order
        let order: Vec<String> = self.order.iter().map(|n| json_str(n)).collect();
        let _ = writeln!(s, "  \"deploy_order\": [{}],", order.join(", "));

        // covenants
        s.push_str("  \"covenants\": [\n");
        for (i, c) in self.covenants.iter().enumerate() {
            let kov = match &c.kov_id {
                Some(id) => json_str(id),
                None => "null".to_string(),
            };
            let comma = if i + 1 < self.covenants.len() {
                ","
            } else {
                ""
            };
            let _ = writeln!(
                s,
                "    {{ \"name\": {}, \"role\": {}, \"kov_id\": {} }}{}",
                json_str(&c.name),
                json_str(&c.role),
                kov,
                comma
            );
        }
        s.push_str("  ],\n");

        // lineage
        s.push_str("  \"lineage\": [\n");
        for (i, e) in self.edges.iter().enumerate() {
            let comma = if i + 1 < self.edges.len() { "," } else { "" };
            let _ = writeln!(
                s,
                "    {{ \"parent\": {}, \"child\": {}, \"binding_field\": {} }}{}",
                json_str(&e.parent),
                json_str(&e.child),
                json_str(&e.binding_field),
                comma
            );
        }
        s.push_str("  ]\n");

        s.push('}');
        s.push('\n');
        s
    }
}

/// Minimal JSON string escaper for the manifest serialiser. Handles the escapes
/// that can appear in covenant/role/field identifiers and app names.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DigitalReit app: parent `DigitalReitToken` → child `DigitalReitSplitter`.
    fn reit_plan() -> LineagePlan {
        LineagePlan::new(
            "DigitalReit",
            vec![
                // Deliberately list the child first to prove deploy_order sorts it.
                Covenant::new("DigitalReitSplitter", "splitter"),
                Covenant::new("DigitalReitToken", "token"),
            ],
            vec![LineageEdge::new(
                "DigitalReitToken",
                "DigitalReitSplitter",
                "parent_kov_id",
            )],
        )
    }

    #[test]
    fn deploy_order_puts_parent_before_child() {
        let plan = reit_plan();
        let order = plan.deploy_order().expect("valid plan");
        assert_eq!(order, vec!["DigitalReitToken", "DigitalReitSplitter"]);
        let parent_idx = order.iter().position(|n| n == "DigitalReitToken").unwrap();
        let child_idx = order
            .iter()
            .position(|n| n == "DigitalReitSplitter")
            .unwrap();
        assert!(parent_idx < child_idx, "parent must deploy before child");
    }

    #[test]
    fn unknown_covenant_in_edge_is_rejected() {
        let plan = LineagePlan::new(
            "Broken",
            vec![Covenant::new("A", "a")],
            vec![LineageEdge::new("A", "Ghost", "f")],
        );
        assert_eq!(
            plan.deploy_order(),
            Err(PlanError::UnknownCovenant("Ghost".to_string()))
        );
    }

    #[test]
    fn cycle_is_rejected() {
        let plan = LineagePlan::new(
            "Loop",
            vec![Covenant::new("A", "a"), Covenant::new("B", "b")],
            vec![
                LineageEdge::new("A", "B", "f"),
                LineageEdge::new("B", "A", "g"),
            ],
        );
        assert_eq!(plan.deploy_order(), Err(PlanError::Cycle));
    }

    #[test]
    fn manifest_json_records_lineage_and_order() {
        let plan = reit_plan();
        let manifest = plan.manifest().expect("valid plan");
        let json = manifest.to_json();
        // deploy_order parent-first.
        let order_line = json
            .lines()
            .find(|l| l.contains("deploy_order"))
            .expect("deploy_order present");
        let token_pos = order_line.find("DigitalReitToken").unwrap();
        let splitter_pos = order_line.find("DigitalReitSplitter").unwrap();
        assert!(token_pos < splitter_pos, "manifest order parent-first");
        // lineage edge + binding field present.
        assert!(json.contains("\"binding_field\": \"parent_kov_id\""));
        assert!(json.contains("\"parent\": \"DigitalReitToken\""));
        assert!(json.contains("\"child\": \"DigitalReitSplitter\""));
        // covenant with no known id serialises kov_id as null.
        assert!(json.contains("\"kov_id\": null"));
    }

    #[test]
    fn with_kov_id_threads_into_manifest() {
        let plan = LineagePlan::new(
            "DigitalReit",
            vec![Covenant::new("DigitalReitToken", "token").with_kov_id("ab".repeat(32))],
            vec![],
        );
        let json = plan.manifest().unwrap().to_json();
        assert!(json.contains(&"ab".repeat(32)));
        assert!(!json.contains("\"kov_id\": null"));
    }
}
