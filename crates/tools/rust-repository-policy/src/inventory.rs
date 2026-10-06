//! Authoritative file inventory for a checked root.
//!
//! Native gap (proven): the alint file walker skips dotfiles entirely, so
//! no native rule can reliably enumerate repository files. This inventory
//! is built from `git ls-files` (tracked plus untracked-but-visible files,
//! hidden dirs included), which is the authoritative source; a filesystem
//! walk is used only when the root is not inside a git work tree or git is
//! unavailable.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Sorted set of `/`-separated paths relative to the checked root.
#[derive(Debug, Clone)]
pub struct Inventory {
    files: BTreeSet<String>,
}

impl Inventory {
    /// Collect the inventory for `root`: git-first, filesystem fallback.
    pub fn collect(root: &Path) -> Inventory {
        git_inventory(root).unwrap_or_else(|| fs_inventory(root))
    }

    /// Inventory from an explicit file list (tests).
    #[cfg(test)]
    pub fn from_files(files: impl IntoIterator<Item = String>) -> Inventory {
        Inventory {
            files: files.into_iter().collect(),
        }
    }

    /// True when the relative path is present.
    #[cfg(test)]
    pub fn contains(&self, rel: &str) -> bool {
        self.files.contains(rel)
    }

    /// `.rs` files under the relative dir `prefix` (recursive), excluding
    /// any path with a `target/` segment. Sorted.
    pub fn rs_files_under(&self, root: &Path, prefix: &str) -> Vec<PathBuf> {
        let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
        self.files
            .iter()
            .filter(|p| {
                (prefix.is_empty() || p.starts_with(&format!("{prefix}/")))
                    && p.ends_with(".rs")
                    && !p.split('/').any(|seg| seg == "target")
            })
            .map(|p| root.join(p))
            .collect()
    }

    /// Workflow files directly under `.github/workflows/`. Sorted.
    pub fn workflow_files(&self, root: &Path) -> Vec<PathBuf> {
        self.files
            .iter()
            .filter(|p| {
                p.starts_with(".github/workflows/")
                    && !p[".github/workflows/".len()..].contains('/')
                    && (p.ends_with(".yml") || p.ends_with(".yaml"))
            })
            .map(|p| root.join(p))
            .collect()
    }
}

/// `git ls-files` inventory: tracked + untracked-but-visible, NUL-separated.
/// Paths come out relative to `root`. `None` when git is unusable.
fn git_inventory(root: &Path) -> Option<Inventory> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let files = output
        .stdout
        .split(|b| *b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect();
    Some(Inventory { files })
}

/// Filesystem fallback: all files including dotfiles, skipping `.git/`
/// and `target/` directories.
fn fs_inventory(root: &Path) -> Inventory {
    let mut files = BTreeSet::new();
    fs_inner(root, root, &mut files);
    Inventory { files }
}

fn fs_inner(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    let entries = std::fs::read_dir(dir)
        .map(|r| r.collect::<Vec<_>>())
        .unwrap_or_default();
    let mut paths: Vec<PathBuf> = entries
        .into_iter()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == ".git" || n == "target")
            {
                continue;
            }
            fs_inner(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.insert(
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Inventory {
        Inventory::from_files([
            ".alint.yml".to_owned(),
            ".github/workflows/ci.yml".to_owned(),
            ".github/workflows/nested/extra.yml".to_owned(),
            "crates/a/src/lib.rs".to_owned(),
            "crates/a/target/debug/x.rs".to_owned(),
            "crates/a/tests/it.rs".to_owned(),
        ])
    }

    #[test]
    fn rs_filter_excludes_target_segments() {
        let files = sample().rs_files_under(Path::new("/r"), "crates/a");
        let names: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["/r/crates/a/src/lib.rs", "/r/crates/a/tests/it.rs"]
        );
    }

    #[test]
    fn workflows_are_top_level_only() {
        let files = sample().workflow_files(Path::new("/r"));
        assert_eq!(files, vec![PathBuf::from("/r/.github/workflows/ci.yml")]);
    }

    #[test]
    fn dotfiles_are_present() {
        assert!(sample().contains(".alint.yml"));
    }
}
