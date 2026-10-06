//! End-to-end `check-gaps` runs against `fixtures/helper/` roots.
//!
//! Each `fail-*` root is otherwise clean, so tests assert exact rule-ID
//! multisets: any stray finding fails the test.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output};

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
    let output = run("fail-size", &["--max-code-lines", "10", "--max-files", "2"]);
    assert_eq!(code(&output), 1);
    expect_rules(&output, &[("GAP-SIZE-001", 1), ("GAP-SIZE-002", 1)]);
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
    assert!(text.contains("tests.rs:1: tests.rs declares child modules"));
    assert!(text.contains("tests/extra.rs:1: split-out test file"));
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
    let exact = run("boundary", &["--max-code-lines", "16", "--max-files", "2"]);
    assert_eq!(code(&exact), 0, "stdout:\n{}", stdout(&exact));
    let lines_below = run("boundary", &["--max-code-lines", "15", "--max-files", "2"]);
    assert_eq!(code(&lines_below), 1);
    expect_rules(&lines_below, &[("GAP-SIZE-001", 1)]);
    let files_below = run("boundary", &["--max-code-lines", "16", "--max-files", "1"]);
    assert_eq!(code(&files_below), 1);
    expect_rules(&files_below, &[("GAP-SIZE-002", 1)]);
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
