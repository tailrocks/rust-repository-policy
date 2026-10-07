//! End-to-end `check-gaps` runs against `fixtures/helper/` roots.
//!
//! Each `fail-*` root is otherwise clean, so tests assert exact rule-ID
//! multisets: any stray finding fails the test.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/helper")
        .join(name)
}

fn run(root: &str, extra: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_rust-repository-policy");
    let mut args = vec!["check-gaps", "--root"];
    let root_path = fixture_root(root);
    let root_str = root_path.to_string_lossy().into_owned();
    args.push(&root_str);
    args.extend(extra.iter());
    Command::new(bin)
        .args(&args)
        .output()
        .expect("helper binary runs")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exit code present")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is utf-8")
}

/// Multiset of rule IDs (first token of each output line).
fn rule_counts(output: &Output) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for line in stdout(output).lines() {
        if let Some(rule) = line.split_whitespace().next() {
            *counts.entry(rule.to_owned()).or_insert(0) += 1;
        }
    }
    counts
}

fn expect_rules(output: &Output, expected: &[(&str, usize)]) {
    let actual = rule_counts(output);
    let expected: BTreeMap<String, usize> = expected
        .iter()
        .map(|(rule, count)| ((*rule).to_owned(), *count))
        .collect();
    assert_eq!(actual, expected, "stdout:\n{}", stdout(output));
}

#[test]
fn pass_root_is_clean() {
    let output = run("pass", &[]);
    assert_eq!(
        code(&output),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout(&output).is_empty());
}

#[test]
fn fail_inherit_reports_all_three_inherit_rules() {
    let output = run("fail-inherit", &[]);
    assert_eq!(code(&output), 1);
    expect_rules(
        &output,
        &[
            ("GAP-INHERIT-001", 3),
            ("GAP-INHERIT-002", 1),
            ("GAP-INHERIT-003", 1),
        ],
    );
    let text = stdout(&output);
    assert!(text.contains("crates/svc/Cargo.toml:3: package key `version`"));
    assert!(text.contains("crates/svc/Cargo.toml:8: dependency `base`"));
    assert!(text.contains("crates/svc/Cargo.toml:10: literal [lints]"));
}

#[test]
fn fail_ci_reports_both_parity_directions() {
    let output = run("fail-ci", &[]);
    assert_eq!(code(&output), 1);
    expect_rules(&output, &[("GAP-CI-002", 1), ("GAP-CI-003", 1)]);
    let text = stdout(&output);
    assert!(text.contains("recipe `ghost` has no matching CI job"));
    assert!(text.contains(".github/workflows/ci.yml:8: CI job `stray`"));
}

#[test]
fn fail_size_reports_both_aggregates_under_small_budgets() {
    let output = run("fail-size", &["--max-code-lines", "10", "--max-files", "1"]);
    assert_eq!(code(&output), 1);
    expect_rules(&output, &[("GAP-SIZE-001", 1), ("GAP-SIZE-002", 1)]);
    // Production only: 12 code lines across lib.rs + extra.rs in 2 files.
    // The tests.rs suite (5 lines) and tests/big.rs (25 lines) are excluded.
    let text = stdout(&output);
    assert!(
        text.contains("has 12 code lines, over the 10 budget"),
        "{text}"
    );
    assert!(text.contains("has 2 files, over the 1 budget"), "{text}");
}

#[test]
fn fail_size_is_clean_at_default_budgets() {
    let output = run("fail-size", &[]);
    assert_eq!(code(&output), 0);
}

#[test]
fn fail_syntax_reports_all_syntax_rules() {
    let output = run("fail-syntax", &[]);
    assert_eq!(code(&output), 1);
    expect_rules(
        &output,
        &[
            ("GAP-TEST-001", 6),
            ("GAP-TEST-002", 3),
            ("GAP-TEST-003", 1),
        ],
    );
    let text = stdout(&output);
    assert!(text.contains("case_attr.rs:1: direct test function"));
    assert!(text.contains("case_async.rs:1: direct test function"));
    assert!(text.contains("case_impl.rs:4: test method"));
    assert!(text.contains("case_include.rs:1: include! of test-named file"));
    assert!(text.contains("case_macro.rs:3: test attribute inside a macro_rules!"));
    assert!(text.contains("case_path.rs:1: test suite module `tests` uses #[path]"));
    assert!(text.contains("orphan.rs:1: orphan Rust file"));
    assert!(text.contains("tests.rs:2: inline child module `inline` in tests.rs"));
    assert!(text.contains("other/tests/stray.rs:1: split-out test file"));
}

#[test]
fn fail_deps_reports_direction_grouping_and_unknown_group() {
    let output = run("fail-deps", &[]);
    assert_eq!(code(&output), 1);
    expect_rules(
        &output,
        &[
            ("GAP-DEPS-001", 1),
            ("GAP-DEPS-002", 1),
            ("GAP-DEPS-003", 1),
        ],
    );
    let text = stdout(&output);
    assert!(text.contains("`low` (group support) depends on `high` (group tool)"));
}

#[test]
fn fail_inventory_reports_both_parity_directions() {
    let output = run("fail-inventory", &[]);
    assert_eq!(code(&output), 1);
    expect_rules(&output, &[("GAP-INV-002", 1), ("GAP-INV-003", 1)]);
}

#[test]
fn fail_missing_reports_all_three_missing_registries() {
    let output = run("fail-missing", &[]);
    assert_eq!(code(&output), 1);
    expect_rules(
        &output,
        &[("GAP-CI-001", 1), ("GAP-DEPS-004", 1), ("GAP-INV-001", 1)],
    );
}

#[test]
fn boundary_passes_exactly_at_budget_and_fails_one_below() {
    // Production only: lib.rs holds 11 code lines in 1 file; the sibling
    // tests.rs suite (5 lines) is excluded from both aggregates.
    let exact = run("boundary", &["--max-code-lines", "11", "--max-files", "1"]);
    assert_eq!(code(&exact), 0, "stdout:\n{}", stdout(&exact));
    let lines_below = run("boundary", &["--max-code-lines", "10", "--max-files", "1"]);
    assert_eq!(code(&lines_below), 1);
    expect_rules(&lines_below, &[("GAP-SIZE-001", 1)]);
    let files_below = run("boundary", &["--max-code-lines", "11", "--max-files", "0"]);
    assert_eq!(code(&files_below), 1);
    expect_rules(&files_below, &[("GAP-SIZE-002", 1)]);
}

#[test]
fn size_tests_only_package_reports_zero_zero() {
    // The package holds only tests/it.rs + tests/other.rs (25 code lines
    // total) and no src/: both aggregates must be 0.
    let output = run(
        "size-tests-only",
        &["--max-code-lines", "0", "--max-files", "0"],
    );
    assert_eq!(code(&output), 0, "stdout:\n{}", stdout(&output));
    assert!(stdout(&output).is_empty());
}

#[test]
fn size_mixed_package_counts_production_only() {
    // Production: lib.rs (6 lines) + api.rs (3 lines) in 2 files. The
    // src/tests.rs suite, src/tests/case.rs, and both tests/*.rs files
    // (38 excluded lines total) must not move these numbers.
    let exact = run("size-tests", &["--max-code-lines", "9", "--max-files", "2"]);
    assert_eq!(code(&exact), 0, "stdout:\n{}", stdout(&exact));
    let lines_below = run("size-tests", &["--max-code-lines", "8", "--max-files", "2"]);
    assert_eq!(code(&lines_below), 1);
    expect_rules(&lines_below, &[("GAP-SIZE-001", 1)]);
    let text = stdout(&lines_below);
    assert!(
        text.contains("has 9 code lines, over the 8 budget"),
        "{text}"
    );
    let files_below = run("size-tests", &["--max-code-lines", "9", "--max-files", "1"]);
    assert_eq!(code(&files_below), 1);
    expect_rules(&files_below, &[("GAP-SIZE-002", 1)]);
    let text = stdout(&files_below);
    assert!(text.contains("has 2 files, over the 1 budget"), "{text}");
}

#[test]
fn size_edge_empty_tests_dir_and_lone_suite() {
    // Production: lib.rs alone (5 lines, 1 file). The tests/ dir holds no
    // Rust sources and the lone src/tests.rs index (no src/tests/ split)
    // is excluded.
    let exact = run(
        "size-tests-edge",
        &["--max-code-lines", "5", "--max-files", "1"],
    );
    assert_eq!(code(&exact), 0, "stdout:\n{}", stdout(&exact));
    let lines_below = run(
        "size-tests-edge",
        &["--max-code-lines", "4", "--max-files", "1"],
    );
    assert_eq!(code(&lines_below), 1);
    expect_rules(&lines_below, &[("GAP-SIZE-001", 1)]);
    let files_below = run(
        "size-tests-edge",
        &["--max-code-lines", "5", "--max-files", "0"],
    );
    assert_eq!(code(&files_below), 1);
    expect_rules(&files_below, &[("GAP-SIZE-002", 1)]);
}

/// Copy of a helper fixture under a unique temp dir, removed on drop.
struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn from_fixture(name: &str, tag: &str) -> TempRoot {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("rrp-inventory-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        copy_tree(&fixture_root(name), &path);
        TempRoot { path }
    }

    fn run(&self, extra: &[&str]) -> Output {
        let bin = env!("CARGO_BIN_EXE_rust-repository-policy");
        let mut args = vec!["check-gaps", "--root"];
        let root_str = self.path.to_string_lossy().into_owned();
        args.push(&root_str);
        args.extend(extra.iter());
        Command::new(bin)
            .args(&args)
            .output()
            .expect("helper binary runs")
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .status()
            .expect("git runs");
        assert!(
            status.success(),
            "git {args:?} failed in {}",
            self.path.display()
        );
    }

    /// Ignored-but-present wired defect: a production `#[test]` the git
    /// leg cannot see (`--others --exclude-standard` skips ignored
    /// files), which the worktree union must still surface.
    fn arm_ignored_source(&self) {
        std::fs::write(
            self.path.join("crates/lib-a/src/evil.rs"),
            "\n#[test]\nfn evil_probe() {}\n",
        )
        .expect("write ignored source");
        let lib = self.path.join("crates/lib-a/src/lib.rs");
        let mut text = std::fs::read_to_string(&lib).expect("read lib.rs");
        text.push_str("mod evil;\n");
        std::fs::write(&lib, text).expect("wire evil module");
        std::fs::write(self.path.join(".gitignore"), "crates/lib-a/src/evil.rs\n")
            .expect("write gitignore");
        let ignored = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["check-ignore", "-q", "crates/lib-a/src/evil.rs"])
            .status()
            .expect("git check-ignore runs");
        assert!(ignored.success(), "probe file must be git-ignored");
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create temp dir");
    let mut entries: Vec<_> = std::fs::read_dir(src)
        .expect("read fixture dir")
        .collect::<Result<_, _>>()
        .expect("read fixture entries");
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy fixture file");
        }
    }
}

#[test]
fn stale_deleted_file_does_not_exit_two() {
    let root = TempRoot::from_fixture("pass", "stale");
    root.git(&["init", "-q"]);
    root.git(&["add", "-A"]);
    // Delete from disk without staging: the index still lists the file.
    std::fs::remove_file(root.path.join("crates/lib-a/src/tests/extra.rs"))
        .expect("remove fixture file");
    let output = root.run(&[]);
    assert_eq!(
        code(&output),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout(&output).is_empty());
}

#[test]
fn ignored_wired_source_still_flags() {
    let root = TempRoot::from_fixture("pass", "ignored");
    root.git(&["init", "-q"]);
    root.git(&["add", "-A"]);
    root.arm_ignored_source();
    let output = root.run(&[]);
    assert_eq!(code(&output), 1);
    expect_rules(&output, &[("GAP-TEST-001", 1)]);
    let text = stdout(&output);
    assert!(text.contains("crates/lib-a/src/evil.rs:2: direct test function"));
}

#[test]
fn git_and_nogit_agree_on_identical_content() {
    let root = TempRoot::from_fixture("pass", "parity");
    root.git(&["init", "-q"]);
    root.git(&["add", "-A"]);
    root.arm_ignored_source();
    let with_git = root.run(&[]);
    std::fs::remove_dir_all(root.path.join(".git")).expect("remove .git");
    let without_git = root.run(&[]);
    assert_eq!(code(&with_git), 1);
    assert_eq!(code(&without_git), code(&with_git));
    assert_eq!(stdout(&without_git), stdout(&with_git));
}

#[test]
fn usage_errors_exit_two() {
    let bin = env!("CARGO_BIN_EXE_rust-repository-policy");
    let no_args = Command::new(bin).output().expect("binary runs");
    assert_eq!(code(&no_args), 2);
    let bad_flag = Command::new(bin)
        .args(["check-gaps", "--bogus"])
        .output()
        .expect("binary runs");
    assert_eq!(code(&bad_flag), 2);
    let no_manifest = Command::new(bin)
        .args(["check-gaps", "--root", "/definitely/not/a/workspace"])
        .output()
        .expect("binary runs");
    assert_eq!(code(&no_manifest), 2);
}
