//! Read-only `cargo metadata` access.
//!
//! The helper never compiles the product workspace: it shells out to
//! `cargo metadata` (twice at most) and parses manifests/sources itself.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// Minimal `cargo metadata` package record.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaPackage {
    pub id: String,
    pub name: String,
    pub manifest_path: PathBuf,
    pub targets: Vec<MetaTarget>,
}

/// Minimal target record.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaTarget {
    pub kind: Vec<String>,
    pub src_path: PathBuf,
}

/// Minimal resolve-node dependency edge.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaNodeDep {
    pub name: String,
    pub pkg: String,
    pub dep_kinds: Option<Vec<MetaDepKind>>,
}

/// Minimal dependency-kind record.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaDepKind {
    pub kind: Option<String>,
}

/// Minimal resolve node.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaNode {
    pub id: String,
    pub deps: Vec<MetaNodeDep>,
}

/// Minimal resolve graph.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaResolve {
    pub nodes: Vec<MetaNode>,
}

/// Minimal `cargo metadata` document.
#[derive(Debug, Clone, Deserialize)]
pub struct Metadata {
    pub packages: Vec<MetaPackage>,
    pub workspace_members: Vec<String>,
    pub resolve: Option<MetaResolve>,
}

impl Metadata {
    /// Names of the workspace-member packages.
    pub fn workspace_package_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .workspace_packages()
            .iter()
            .map(|p| p.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Workspace-member packages, matched by exact package ID.
    pub fn workspace_packages(&self) -> Vec<&MetaPackage> {
        self.packages
            .iter()
            .filter(|p| self.workspace_members.iter().any(|m| m == &p.id))
            .collect()
    }
}

/// Run `cargo metadata --format-version 1 [--no-deps]` at `root`.
pub fn run_metadata(root: &Path, no_deps: bool) -> Result<Metadata, String> {
    let manifest = root.join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    cmd.arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(&manifest);
    if no_deps {
        cmd.arg("--no-deps");
    }
    let output = cmd
        .output()
        .map_err(|e| format!("running cargo metadata: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("cargo metadata failed: {}", stderr.trim()));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("parsing cargo metadata output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "packages": [
            {"id": "a 0.1.0 (path+file:///r/a)", "name": "a", "manifest_path": "/r/a/Cargo.toml", "targets": []},
            {"id": "b 0.1.0 (path+file:///r/b)", "name": "b", "manifest_path": "/r/b/Cargo.toml", "targets": []},
            {"id": "registry+https://example.com/ext 1.0.0", "name": "ext", "manifest_path": "/x/ext/Cargo.toml", "targets": []}
        ],
        "workspace_members": ["a 0.1.0 (path+file:///r/a)", "b 0.1.0 (path+file:///r/b)"],
        "resolve": null
    }"#;

    #[test]
    fn workspace_names_match_member_ids() {
        let meta: Metadata = serde_json::from_str(SAMPLE).expect("sample parses");
        assert_eq!(meta.workspace_package_names(), vec!["a", "b"]);
        assert_eq!(meta.workspace_packages().len(), 2);
    }
}
