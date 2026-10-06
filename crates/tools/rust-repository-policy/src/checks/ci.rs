//! CI recipe-to-job set parity.
//!
//! Native gap: alint YAML rules address enumerated paths
//! (`yaml_path_equals` / `yaml_path_absent`), but JSONPath cannot extract
//! the key names of a `jobs:` mapping, so no native rule can compare the
//! *set* of CI jobs against a registry. This check parses every workflow,
//! extracts job IDs, and diffs them against `policy/ci-recipes.json`.
//!
//! Rules:
//!
//! - `GAP-CI-001`: the recipe registry is missing or invalid.
//! - `GAP-CI-002`: a registered recipe has no matching CI job.
//! - `GAP-CI-003`: a CI job has no registered recipe.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use crate::finding::{rel_path, Finding};
use crate::inventory::Inventory;
use crate::util::{json_string_line, read_text};

/// Adopter-owned recipe registry, relative to the workspace root.
pub const REGISTRY_REL: &str = "policy/ci-recipes.json";

#[derive(Debug, Deserialize)]
struct Registry {
    version: u32,
    recipes: Vec<String>,
}

/// Check recipe/job set parity at `root`.
pub fn check(root: &Path, inventory: &Inventory) -> Result<Vec<Finding>, String> {
    let registry_path = root.join(REGISTRY_REL);
    // A missing/invalid registry is a finding, not a hard error, so report
    // it through the normal channel.
    let Ok(registry_text) = read_text(&registry_path) else {
        return Ok(vec![Finding::new(
            "GAP-CI-001",
            REGISTRY_REL.to_owned(),
            1,
            "missing recipe registry",
            format!("create {REGISTRY_REL} with version 1 and the expected job IDs"),
        )]);
    };
    let registry: Registry = match serde_json::from_str(&registry_text) {
        Ok(registry) => registry,
        Err(e) => {
            return Ok(vec![Finding::new(
                "GAP-CI-001",
                REGISTRY_REL.to_owned(),
                1,
                format!("invalid recipe registry: {e}"),
                format!("make {REGISTRY_REL} valid JSON with version 1 and a recipes array"),
            )]);
        }
    };
    if registry.version != 1 {
        return Ok(vec![Finding::new(
            "GAP-CI-001",
            REGISTRY_REL.to_owned(),
            1,
            format!("unsupported registry version {}", registry.version),
            "set `version` to 1".to_owned(),
        )]);
    }

    let workflows = inventory.workflow_files(root);
    let mut jobs: BTreeSet<String> = BTreeSet::new();
    let mut job_lines: Vec<(String, String, usize)> = Vec::new();
    for workflow in &workflows {
        let text = read_text(workflow)?;
        let doc: serde_yaml::Value = serde_yaml::from_str(&text)
            .map_err(|e| format!("parsing {}: {e}", workflow.display()))?;
        for job in job_ids(&doc) {
            let line = job_key_line(&text, &job);
            job_lines.push((rel_path(workflow, root), job.clone(), line));
            jobs.insert(job);
        }
    }

    let mut findings = Vec::new();
    let recipes: BTreeSet<&str> = registry.recipes.iter().map(String::as_str).collect();
    for recipe in &registry.recipes {
        if !jobs.contains(recipe) {
            findings.push(Finding::new(
                "GAP-CI-002",
                REGISTRY_REL.to_owned(),
                json_string_line(&registry_text, recipe),
                format!("recipe `{recipe}` has no matching CI job"),
                format!(
                    "add a `{recipe}` job under `jobs:` in .github/workflows/ or remove the recipe"
                ),
            ));
        }
    }
    for (path, job, line) in &job_lines {
        if !recipes.contains(job.as_str()) {
            findings.push(Finding::new(
                "GAP-CI-003",
                path.clone(),
                *line,
                format!("CI job `{job}` has no registered recipe"),
                format!("register `{job}` in {REGISTRY_REL} or remove the job"),
            ));
        }
    }
    Ok(findings)
}

/// Keys of the top-level `jobs:` mapping.
fn job_ids(doc: &serde_yaml::Value) -> Vec<String> {
    doc.get("jobs")
        .and_then(serde_yaml::Value::as_mapping)
        .map(|jobs| {
            jobs.keys()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// 1-based line of a job key: the `jobs:` header's direct-child key.
/// Returns 1 when the shape cannot be located.
fn job_key_line(text: &str, job: &str) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    let mut jobs_indent: Option<usize> = None;
    let mut child_indent: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if jobs_indent.is_none() {
            if is_mapping_key(stripped, "jobs") {
                jobs_indent = Some(indent);
            }
            continue;
        }
        let base = jobs_indent.expect("set above");
        if indent <= base {
            break;
        }
        if child_indent.is_none() {
            child_indent = Some(indent);
        }
        if Some(indent) == child_indent
            && let Some(key) = mapping_key(stripped)
            && key == job
        {
            return index + 1;
        }
    }
    1
}

/// True when a trimmed line is exactly the `key:` mapping key (no inline value).
fn is_mapping_key(stripped: &str, key: &str) -> bool {
    stripped == format!("{key}:")
}

/// Extract the mapping key of a trimmed `key:` / `key: value` / `"key": ...` line.
fn mapping_key(stripped: &str) -> Option<String> {
    let (raw, _) = stripped.split_once(':')?;
    let key = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if key.is_empty() {
        return None;
    }
    Some(key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = r#"name: ci
on: push
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
  "lint":
    runs-on: ubuntu-latest
"#;

    #[test]
    fn job_ids_read_mapping_keys() {
        let doc: serde_yaml::Value = serde_yaml::from_str(WORKFLOW).expect("yaml parses");
        assert_eq!(job_ids(&doc), vec!["check".to_owned(), "lint".to_owned()]);
    }

    #[test]
    fn job_lines_point_at_keys() {
        assert_eq!(job_key_line(WORKFLOW, "check"), 4);
        assert_eq!(job_key_line(WORKFLOW, "lint"), 8);
        assert_eq!(job_key_line(WORKFLOW, "nope"), 1);
    }

    #[test]
    fn missing_jobs_mapping_yields_no_ids() {
        let doc: serde_yaml::Value = serde_yaml::from_str("name: x\n").expect("yaml parses");
        assert!(job_ids(&doc).is_empty());
    }
}
