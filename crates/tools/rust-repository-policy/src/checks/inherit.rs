//! Open-ended workspace-inheritance completeness.
//!
//! Native gap: alint `toml_path_equals` can only pin enumerated keys to
//! `{workspace: true}`. It cannot express "every key the workspace defines
//! must be inherited", because that needs Cargo's inheritable-key knowledge
//! plus a join across manifests. This check iterates the actual
//! `[workspace.package]` keys, `[workspace.dependencies]` names, and
//! `[workspace.lints]` presence, so newly added workspace keys are covered
//! without editing any rule.
//!
//! Rules:
//!
//! - `GAP-INHERIT-001`: member `[package]` key defined literally while the
//!   workspace defines it.
//! - `GAP-INHERIT-002`: member dependency entry defined literally while the
//!   workspace defines that dependency.
//! - `GAP-INHERIT-003`: member lints not inherited while the workspace
//!   defines `[workspace.lints]`.

use std::collections::BTreeSet;
use std::path::Path;

use toml::Value;

use crate::finding::{rel_path, Finding};
use crate::meta::Metadata;
use crate::util::{dep_entry_line, lints_header_line, read_text, toml_key_line};

/// Dependency table names that participate in workspace inheritance.
const DEP_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

/// Check every workspace member manifest against the root workspace tables.
pub fn check(root: &Path, meta: &Metadata) -> Result<Vec<Finding>, String> {
    let root_text = read_text(&root.join("Cargo.toml"))?;
    let root_doc: Value =
        toml::from_str(&root_text).map_err(|e| format!("parsing Cargo.toml: {e}"))?;
    let workspace = root_doc.get("workspace");
    let package_keys: BTreeSet<String> = workspace
        .and_then(|w| w.get("package"))
        .and_then(Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    let dep_names: BTreeSet<String> = workspace
        .and_then(|w| w.get("dependencies"))
        .and_then(Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    let workspace_lints = workspace.and_then(|w| w.get("lints")).is_some();

    let mut findings = Vec::new();
    for package in meta.workspace_packages() {
        let text = read_text(&package.manifest_path)?;
        let doc: Value = toml::from_str(&text)
            .map_err(|e| format!("parsing {}: {e}", package.manifest_path.display()))?;
        let rel = rel_path(&package.manifest_path, root);
        check_package_keys(&doc, &text, &rel, &package_keys, &mut findings);
        check_dep_tables(&doc, &text, &rel, &dep_names, &mut findings);
        if workspace_lints {
            check_lints(&doc, &text, &rel, &mut findings);
        }
    }
    Ok(findings)
}

fn check_package_keys(
    doc: &Value,
    text: &str,
    rel: &str,
    package_keys: &BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    let Some(table) = doc.get("package").and_then(Value::as_table) else {
        return;
    };
    for (key, value) in table {
        if package_keys.contains(key) && !is_inherited(value) {
            findings.push(Finding::new(
                "GAP-INHERIT-001",
                rel.to_owned(),
                toml_key_line(text, "package", key),
                format!("package key `{key}` is defined literally but the workspace defines it"),
                format!("replace with `{key}.workspace = true`"),
            ));
        }
    }
}

fn check_dep_tables(
    doc: &Value,
    text: &str,
    rel: &str,
    dep_names: &BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    if dep_names.is_empty() {
        return;
    }
    walk_dep_tables(doc, &mut |entry, value| {
        let reference = dep_reference(entry, value);
        if dep_names.contains(&reference) && !is_inherited(value) {
            findings.push(Finding::new(
                "GAP-INHERIT-002",
                rel.to_owned(),
                dep_entry_line(text, entry),
                format!(
                    "dependency `{entry}` is defined literally but the workspace defines `{reference}`"
                ),
                format!("replace with `{entry}.workspace = true`"),
            ));
        }
    });
}

/// Visit every `(entry name, value)` inside dependency tables, descending
/// through `[target.*]` wrappers but not into entries themselves.
fn walk_dep_tables(value: &Value, visit: &mut impl FnMut(&str, &Value)) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        if DEP_TABLES.contains(&key.as_str()) {
            if let Some(entries) = child.as_table() {
                for (entry, entry_value) in entries {
                    visit(entry, entry_value);
                }
            }
        } else if !matches!(
            key.as_str(),
            "package" | "workspace" | "lints" | "patch" | "features"
        ) {
            walk_dep_tables(child, visit);
        }
    }
}

/// The workspace-dependency name an entry refers to: the `package` rename
/// target when present, else the entry name.
fn dep_reference(entry: &str, value: &Value) -> String {
    value
        .get("package")
        .and_then(Value::as_str)
        .unwrap_or(entry)
        .to_owned()
}

fn check_lints(doc: &Value, text: &str, rel: &str, findings: &mut Vec<Finding>) {
    match doc.get("lints") {
        None => findings.push(Finding::new(
            "GAP-INHERIT-003",
            rel.to_owned(),
            1,
            "missing [lints] while the workspace defines [workspace.lints]",
            "add `[lints]` with `workspace = true`",
        )),
        Some(lints) if !is_inherited(lints) => findings.push(Finding::new(
            "GAP-INHERIT-003",
            rel.to_owned(),
            lints_header_line(text),
            "literal [lints] table while the workspace defines [workspace.lints]",
            "replace with `[lints]` and `workspace = true`",
        )),
        Some(_) => {}
    }
}

/// True for `{ workspace = true }` (possibly with extra keys such as
/// `features`, which Cargo allows alongside inheritance).
fn is_inherited(value: &Value) -> bool {
    value
        .get("workspace")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_table_with_features_still_counts() {
        let doc: Value = toml::from_str("[dep]\nworkspace = true\nfeatures = [\"x\"]\n")
            .expect("test toml parses");
        assert!(is_inherited(&doc["dep"]));
        let literal: Value = toml::from_str("[dep]\nversion = \"1\"\n").expect("test toml parses");
        assert!(!is_inherited(&literal["dep"]));
        assert!(!is_inherited(&Value::String("1".to_owned())));
    }

    #[test]
    fn dep_reference_prefers_package_rename() {
        let doc: Value = toml::from_str("[dep]\npackage = \"real\"\nversion = \"1\"\n")
            .expect("test toml parses");
        assert_eq!(dep_reference("alias", &doc["dep"]), "real");
        assert_eq!(
            dep_reference("plain", &Value::String("1".to_owned())),
            "plain"
        );
    }

    #[test]
    fn walk_finds_target_wrapped_tables_but_not_entries() {
        let doc: Value = toml::from_str(
            "[dependencies]\na = \"1\"\n[target.'cfg(unix)'.dependencies]\nb = \"1\"\n[dependencies.c]\nversion = \"1\"\n",
        )
        .expect("test toml parses");
        let mut seen = Vec::new();
        walk_dep_tables(&doc, &mut |entry, _| seen.push(entry.to_owned()));
        // `version` inside `[dependencies.c]` is not a dependency.
        assert_eq!(seen, vec!["a".to_owned(), "c".to_owned(), "b".to_owned()]);
    }
}
