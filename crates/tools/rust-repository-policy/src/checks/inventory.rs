//! Owned-package inventory parity.
//!
//! Native gap: alint cannot run Cargo, so it cannot compare a declared
//! package inventory against the real workspace membership. This check
//! diffs the adopter's `policy/owned-packages.json` (shape per
//! `schemas/owned-package-inventory.schema.json`) against
//! `cargo metadata --no-deps` workspace members.
//!
//! Rules:
//!
//! - `GAP-INV-001`: the inventory file is missing or invalid.
//! - `GAP-INV-002`: a workspace package is missing from the inventory.
//! - `GAP-INV-003`: an inventory entry is not a workspace package.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use crate::finding::Finding;
use crate::meta::Metadata;
use crate::util::{json_string_line, read_text};

/// Adopter-owned inventory file, relative to the workspace root.
pub const INVENTORY_REL: &str = "policy/owned-packages.json";

#[derive(Debug, Deserialize)]
struct InventoryFile {
    version: u32,
    packages: Vec<String>,
}

/// Check inventory parity against `--no-deps` metadata.
pub fn check(root: &Path, meta_no_deps: &Metadata) -> Result<Vec<Finding>, String> {
    let inventory_path = root.join(INVENTORY_REL);
    let Ok(inventory_text) = read_text(&inventory_path) else {
        return Ok(vec![Finding::new(
            "GAP-INV-001",
            INVENTORY_REL.to_owned(),
            1,
            "missing owned-package inventory",
            format!("create {INVENTORY_REL} with version 1 and the workspace package names"),
        )]);
    };
    let inventory: InventoryFile = match serde_json::from_str(&inventory_text) {
        Ok(file) => file,
        Err(e) => {
            return Ok(vec![Finding::new(
                "GAP-INV-001",
                INVENTORY_REL.to_owned(),
                1,
                format!("invalid owned-package inventory: {e}"),
                format!("make {INVENTORY_REL} valid JSON with version 1 and a packages array"),
            )]);
        }
    };
    if inventory.version != 1 {
        return Ok(vec![Finding::new(
            "GAP-INV-001",
            INVENTORY_REL.to_owned(),
            1,
            format!("unsupported inventory version {}", inventory.version),
            "set `version` to 1".to_owned(),
        )]);
    }

    let mut findings = Vec::new();
    let actual: BTreeSet<String> = meta_no_deps.workspace_package_names().into_iter().collect();
    let declared: BTreeSet<&str> = inventory.packages.iter().map(String::as_str).collect();
    for name in &actual {
        if !declared.contains(name.as_str()) {
            findings.push(Finding::new(
                "GAP-INV-002",
                INVENTORY_REL.to_owned(),
                1,
                format!("workspace package `{name}` is missing from the owned-package inventory"),
                format!("add `{name}` to the packages array in {INVENTORY_REL}"),
            ));
        }
    }
    for name in &inventory.packages {
        if !actual.contains(name) {
            findings.push(Finding::new(
                "GAP-INV-003",
                INVENTORY_REL.to_owned(),
                json_string_line(&inventory_text, name),
                format!("inventory entry `{name}` is not a workspace package"),
                format!("remove `{name}` from the packages array in {INVENTORY_REL}"),
            ));
        }
    }
    Ok(findings)
}
