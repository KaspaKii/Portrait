//! Keeps the workspace README's per-pattern threat-model claim honest.
//!
//! The workspace README says every pattern crate ships a threat model. This
//! test is what makes the *presence* of one mechanically true: it reads the
//! workspace `members` list, skips the crates that are not patterns, and fails
//! if any remaining crate's README is missing the `## Threat model` section,
//! one of its five fixed sub-headings, the not-an-audit stamp, or has a section
//! too short to say anything. A new pattern crate fails the gate until it
//! carries one.
//!
//! **What this does not do:** judge whether a threat model is *correct*. The
//! content is reviewed by humans (architect / red-team), not verified here.
//! Everything below is a shape check.
use std::path::{Path, PathBuf};

/// Workspace members that are deliberately not covenant patterns: shared
/// plumbing, the scaffolder, and the Solidity-shape translation layer. Adding a
/// crate here is a deliberate act; every other member must carry a threat
/// model. (`kcp-common`'s surface is covered by the models of the patterns that
/// use it — see the workspace README.)
const NON_PATTERN_CRATES: &[&str] = &["kcp-common", "kcp-cli", "kii-solidity-compat"];

/// The five fixed sub-headings every threat model uses, plus the honesty stamp.
const REQUIRED_FRAGMENTS: &[&str] = &[
    "**Assets**",
    "**Attacker capabilities (assumed)**",
    "**What consensus enforces**",
    "**What this assumes / trusts off-chain**",
    "**Known limits and non-goals**",
    "not a security audit",
];

/// A threat model shorter than this says nothing useful.
const MIN_SECTION_BYTES: usize = 1_200;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/kcp-cli; go up two levels → workspace root
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Every workspace member path, read from the root `Cargo.toml` `members` list
/// — not from a directory scan, so a member outside `crates/` cannot evade the
/// gate by living elsewhere.
fn workspace_members() -> Vec<String> {
    let manifest = std::fs::read_to_string(workspace_root().join("Cargo.toml"))
        .expect("workspace Cargo.toml must be readable");
    let members_block = manifest
        .split_once("members = [")
        .expect("workspace Cargo.toml must declare members")
        .1
        .split_once(']')
        .expect("members list must be closed")
        .0;
    members_block
        .split(',')
        .map(|entry| entry.trim().trim_matches('"').to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn pattern_crates() -> Vec<(String, PathBuf)> {
    workspace_members()
        .into_iter()
        .filter_map(|member| {
            let name = Path::new(&member)
                .file_name()?
                .to_string_lossy()
                .into_owned();
            if NON_PATTERN_CRATES.contains(&name.as_str()) {
                return None;
            }
            Some((name, workspace_root().join(member)))
        })
        .collect()
}

/// The body of the `## Threat model` section, up to the next `## ` heading.
fn threat_model_section(readme: &str) -> Option<&str> {
    let start = readme.find("\n## Threat model\n")? + 1;
    let rest = &readme[start..];
    let end = rest[1..]
        .find("\n## ")
        .map(|offset| offset + 2)
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

#[test]
fn every_pattern_crate_readme_carries_a_threat_model() {
    let crates = pattern_crates();
    assert!(
        crates.len() >= 10,
        "expected at least 10 pattern crates, found {} — did the members list \
         parse correctly?",
        crates.len()
    );

    for (name, dir) in crates {
        let readme = dir.join("README.md");
        let text = std::fs::read_to_string(&readme).unwrap_or_else(|err| {
            panic!(
                "{name}: {} must exist and be readable ({err}) — the workspace \
                 README claims every pattern crate ships a threat model",
                readme.display()
            )
        });

        let section = threat_model_section(&text).unwrap_or_else(|| {
            panic!(
                "{name}: README.md must carry a `## Threat model` section — the \
                 workspace README claims every pattern crate has one"
            )
        });

        for fragment in REQUIRED_FRAGMENTS {
            assert!(
                section.contains(fragment),
                "{name}: the threat model is missing `{fragment}` — every model \
                 uses the same five headings and states that it is not an audit"
            );
        }

        assert!(
            section.len() >= MIN_SECTION_BYTES,
            "{name}: the threat model is {} bytes, under the {MIN_SECTION_BYTES}-byte \
             floor — headings alone are not a threat model",
            section.len()
        );
    }
}
