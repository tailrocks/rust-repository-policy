//! Dependency direction per group table.
//!
//! Native gap: alint has no Cargo resolve graph, so it cannot see which
//! workspace package depends on which. This check reads the resolve graph
//! from `cargo metadata` (read-only, never compiles) and enforces the
//! layer direction over the adopter's `policy/package-groups.json` table.
//!
//! Group table (dependent → allowed dependency groups):
//!
//! | group   | may depend on                |
//! |---------|------------------------------|
//! | tool    | tool, library, support, ffi  |
//! | library | library, support, ffi        |
//! | support | support, ffi                 |
//! | ffi     | ffi                          |
//! | lints   | (nothing workspace-internal) |
//!
//! Only `normal` and `build` edges are checked; `dev-dependencies` are
//! exempt (tests may use anything).
//!
//! Rules:
//!
//! - `GAP-DEPS-001`: dependency direction violation.
//! - `GAP-DEPS-002`: workspace package missing from the group table.
//! - `GAP-DEPS-003`: unknown group name in the table.
//! - `GAP-DEPS-004`: the group table file is missing or invalid.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::finding::{rel_path, Finding};
use crate::meta::Metadata;
use crate::util::{dep_entry_line, json_string_line, read_text};

/// Adopter-owned group table, relative to the workspace root.
pub const GROUPS_REL: &str = "policy/package-groups.json";

/// Allowed dependency groups per dependent group.
fn allowed(group: &str) -> Option<&'static [&'static str]> {
    match group {
        "tool" => Some(&["tool", "library", "support", "ffi"]),
        "library" => Some(&["library", "support", "ffi"]),
        "support" => Some(&["support", "ffi"]),
        "ffi" => Some(&["ffi"]),
        "lints" => Some(&[]),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct GroupsFile {
    version: u32,
    groups: BTreeMap<String, String>,
}

/// Check group-table validity and dependency direction.
pub fn check(root: &Path, meta: &Metadata) -> Result<Vec<Finding>, String> {
    let groups_path = root.join(GROUPS_REL);
    let Ok(groups_text) = read_text(&groups_path) else {
        return Ok(vec![Finding::new(
            "GAP-DEPS-004",
            GROUPS_REL.to_owned(),
            1,
            "missing package group table",
            format!("create {GROUPS_REL} with version 1 and a group per owned package"),
        )]);
    };
    let groups_file: GroupsFile = match serde_json::from_str(&groups_text) {
        Ok(file) => file,
        Err(e) => {
            return Ok(vec![Finding::new(
                "GAP-DEPS-004",
                GROUPS_REL.to_owned(),
                1,
                format!("invalid package group table: {e}"),
                format!("make {GROUPS_REL} valid JSON with version 1 and a groups object"),
            )]);
        }
    };
    if groups_file.version != 1 {
        return Ok(vec![Finding::new(
            "GAP-DEPS-004",
            GROUPS_REL.to_owned(),
            1,
            format!("unsupported group table version {}", groups_file.version),
            "set `version` to 1".to_owned(),
        )]);
    }

    let mut findings = Vec::new();
    let groups = &groups_file.groups;
    for (package, group) in groups {
        if allowed(group).is_none() {
            findings.push(Finding::new(
                "GAP-DEPS-003",
                GROUPS_REL.to_owned(),
                json_string_line(&groups_text, package),
                format!("package `{package}` has unknown group `{group}`"),
                "use one of: tool, library, support, ffi, lints".to_owned(),
            ));
        }
    }
    let names: BTreeSet<String> = meta.workspace_package_names().into_iter().collect();
    for name in &names {
        if !groups.contains_key(name) {
            findings.push(Finding::new(
                "GAP-DEPS-002",
                GROUPS_REL.to_owned(),
                1,
                format!("workspace package `{name}` is missing from the group table"),
                format!("add `{name}` to the groups object in {GROUPS_REL}"),
            ));
        }
    }
    // Phantom table entries (not workspace packages) are the inventory
    // check's business only when they also miss the inventory; here they
    // simply have no edges to check.

    check_direction(root, meta, groups, &mut findings)?;
    Ok(findings)
}

/// Enforce the direction table over resolve-graph edges between grouped
/// workspace packages.
fn check_direction(
    root: &Path,
    meta: &Metadata,
    groups: &BTreeMap<String, String>,
    findings: &mut Vec<Finding>,
) -> Result<(), String> {
    let Some(resolve) = &meta.resolve else {
        return Ok(());
    };
    let id_to_name: BTreeMap<&str, &str> = meta
        .packages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();
    let name_to_manifest: BTreeMap<&str, &Path> = meta
        .packages
        .iter()
        .map(|p| (p.name.as_str(), p.manifest_path.as_path()))
        .collect();
    let workspace_ids: BTreeSet<&str> = meta.workspace_members.iter().map(String::as_str).collect();

    for node in &resolve.nodes {
        if !workspace_ids.contains(node.id.as_str()) {
            continue;
        }
        let Some(from) = id_to_name.get(node.id.as_str()) else {
            continue;
        };
        let Some(from_group) = groups.get(*from) else {
            continue; // Already flagged as GAP-DEPS-002.
        };
        let Some(permitted) = allowed(from_group) else {
            continue; // Already flagged as GAP-DEPS-003.
        };
        for edge in &node.deps {
            if !workspace_ids.contains(edge.pkg.as_str()) || edge.pkg == node.id {
                continue;
            }
            if !is_build_time_edge(edge) {
                continue; // dev-only edges are exempt.
            }
            let Some(to) = id_to_name.get(edge.pkg.as_str()) else {
                continue;
            };
            let to_group = groups.get(*to).map(String::as_str).unwrap_or("?");
            if permitted.contains(&to_group) {
                continue;
            }
            let manifest = name_to_manifest
                .get(from)
                .expect("workspace package has a manifest");
            let text = read_text(manifest)?;
            let line = manifest_line_for_edge(&text, &edge.name, to);
            findings.push(Finding::new(
                "GAP-DEPS-001",
                rel_path(manifest, root),
                line,
                format!(
                    "dependency direction violation: `{from}` (group {from_group}) \
                     depends on `{to}` (group {to_group})"
                ),
                format!(
                    "remove the `{}` dependency or move the shared code into a group \
                     `{from_group}` may depend on",
                    edge.name
                ),
            ));
        }
    }
    Ok(())
}

/// True when the edge has any non-dev kind (missing/empty kinds count as
/// normal, matching cargo's default).
fn is_build_time_edge(edge: &crate::meta::MetaNodeDep) -> bool {
    match &edge.dep_kinds {
        None => true,
        Some(kinds) if kinds.is_empty() => true,
        Some(kinds) => kinds
            .iter()
            .any(|k| matches!(k.kind.as_deref(), None | Some("normal" | "build"))),
    }
}

/// Manifest line for an edge: try the declared name first, then the package
/// name (covers renames either way round).
fn manifest_line_for_edge(text: &str, declared: &str, package: &str) -> usize {
    let line = dep_entry_line(text, declared);
    if line != 1 || declared == package {
        return line;
    }
    dep_entry_line(text, package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{MetaDepKind, MetaNodeDep};

    #[test]
    fn direction_table_shape() {
        assert!(allowed("tool").expect("tool").contains(&"library"));
        assert!(!allowed("library").expect("library").contains(&"tool"));
        assert_eq!(allowed("lints").expect("lints").len(), 0);
        assert!(allowed("nope").is_none());
    }

    #[test]
    fn dev_only_edges_are_exempt() {
        let dev = MetaNodeDep {
            name: "x".to_owned(),
            pkg: "x".to_owned(),
            dep_kinds: Some(vec![MetaDepKind {
                kind: Some("dev".to_owned()),
            }]),
        };
        assert!(!is_build_time_edge(&dev));
        let normal = MetaNodeDep {
            name: "x".to_owned(),
            pkg: "x".to_owned(),
            dep_kinds: Some(vec![MetaDepKind {
                kind: Some("normal".to_owned()),
            }]),
        };
        assert!(is_build_time_edge(&normal));
        let missing = MetaNodeDep {
            name: "x".to_owned(),
            pkg: "x".to_owned(),
            dep_kinds: None,
        };
        assert!(is_build_time_edge(&missing));
    }
}
