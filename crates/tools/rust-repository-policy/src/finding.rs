//! A single gap finding: stable rule ID, relative path, line, correction.
//!
//! Every check emits findings in this shape so output stays greppable and
//! stable across runs. Findings sort by `(path, line, rule)`.

use std::fmt;
use std::path::Path;

/// One proven-native-gap violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Stable rule ID, e.g. `GAP-TEST-001`. Never reused for another meaning.
    pub rule: &'static str,
    /// Path relative to the checked root, `/`-separated.
    pub path: String,
    /// 1-based line number. Aggregate/file-level findings use line 1.
    pub line: usize,
    /// What is wrong.
    pub message: String,
    /// How to fix it.
    pub correction: String,
}

impl Finding {
    pub fn new(
        rule: &'static str,
        path: String,
        line: usize,
        message: impl Into<String>,
        correction: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            path,
            line: line.max(1),
            message: message.into(),
            correction: correction.into(),
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}:{}: {} (correction: {})",
            self.rule, self.path, self.line, self.message, self.correction
        )
    }
}

/// Sort findings into stable output order.
pub fn sort(findings: &mut [Finding]) {
    findings.sort_by(|a, b| (&a.path, a.line, a.rule).cmp(&(&b.path, b.line, b.rule)));
}

/// Render `path` relative to `root` with `/` separators.
pub fn rel_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_shape_holds_rule_path_line_correction() {
        let finding = Finding::new(
            "GAP-TEST-001",
            "crates/a/src/lib.rs".to_owned(),
            12,
            "test fn in production file",
            "move it to tests.rs",
        );
        assert_eq!(
            finding.to_string(),
            "GAP-TEST-001 crates/a/src/lib.rs:12: test fn in production file \
             (correction: move it to tests.rs)"
        );
    }

    #[test]
    fn sort_orders_by_path_line_rule() {
        let mut findings = vec![
            Finding::new("GAP-B", "b.rs".to_owned(), 1, "m", "c"),
            Finding::new("GAP-A", "a.rs".to_owned(), 9, "m", "c"),
            Finding::new("GAP-C", "a.rs".to_owned(), 2, "m", "c"),
        ];
        sort(&mut findings);
        let rules: Vec<&str> = findings.iter().map(|f| f.rule).collect();
        assert_eq!(rules, vec!["GAP-C", "GAP-A", "GAP-B"]);
    }

    #[test]
    fn line_zero_clamps_to_one() {
        let finding = Finding::new("GAP-X", "f".to_owned(), 0, "m", "c");
        assert_eq!(finding.line, 1);
    }
}
