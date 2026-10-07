//! Per-crate size aggregates.
//!
//! Native gap: alint probed with zero aggregate kinds — it can cap one
//! file (`file_max_lines`) but cannot sum lines or count files per crate.
//! This check totals token-aware code lines (excluding comments and blank
//! lines) and file counts per workspace package.
//!
//! Rules:
//!
//! - `GAP-SIZE-001`: crate exceeds the code-line budget (default 6000).
//! - `GAP-SIZE-002`: crate exceeds the file-count budget (default 60).
//!
//! Test-classified files are excluded from both aggregates (S7: tests do
//! not count as production code): `tests/**` integration tests,
//! `src/**/tests.rs` canonical unit suites, and `src/**/tests/**`
//! canonical case splits. Production hidden under `tests/` is the
//! tests-module placement rules' business, not the size aggregate's.

use std::path::Path;

use crate::finding::{rel_path, Finding};
use crate::inventory::Inventory;
use crate::meta::Metadata;
use crate::util::read_text;

/// Default per-crate code-line budget.
pub const DEFAULT_MAX_CODE_LINES: usize = 6000;
/// Default per-crate file-count budget.
pub const DEFAULT_MAX_FILES: usize = 60;

/// Check per-crate aggregates at `root`.
pub fn check(
    root: &Path,
    meta: &Metadata,
    inventory: &Inventory,
    max_code_lines: usize,
    max_files: usize,
) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    for package in meta.workspace_packages() {
        let Some(dir) = package.manifest_path.parent() else {
            continue;
        };
        let prefix = rel_path(dir, root);
        let files: Vec<_> = inventory
            .rs_files_under(root, &prefix)
            .into_iter()
            .filter(|file| !is_test_file(dir, file))
            .collect();
        let mut code_lines = 0;
        for file in &files {
            let text = read_text(file)?;
            code_lines += count_code_lines(&text);
        }
        // Aggregate findings attach to the crate manifest at line 1: the
        // budget belongs to the whole crate, not one line.
        let rel = rel_path(&package.manifest_path, root);
        if code_lines > max_code_lines {
            findings.push(Finding::new(
                "GAP-SIZE-001",
                rel.clone(),
                1,
                format!(
                    "crate `{}` has {code_lines} code lines, over the {max_code_lines} budget",
                    package.name
                ),
                "split the crate along module boundaries until each crate fits the budget",
            ));
        }
        if files.len() > max_files {
            findings.push(Finding::new(
                "GAP-SIZE-002",
                rel,
                1,
                format!(
                    "crate `{}` has {} files, over the {max_files} budget",
                    package.name,
                    files.len()
                ),
                "split the crate along module boundaries until each crate fits the budget",
            ));
        }
    }
    Ok(findings)
}

/// True when `file` is test-classified relative to the package `dir`:
/// `tests/**`, `src/**/tests.rs`, or `src/**/tests/**`. Benches,
/// examples, and build scripts stay counted.
fn is_test_file(dir: &Path, file: &Path) -> bool {
    file.strip_prefix(dir).is_ok_and(is_test_path)
}

/// True for package-relative test-classified paths (see [`is_test_file`]).
fn is_test_path(pkg_rel: &Path) -> bool {
    let mut components = pkg_rel.components();
    let Some(top) = components.next() else {
        return false;
    };
    if top.as_os_str() == "tests" {
        return true;
    }
    if top.as_os_str() != "src" {
        return false;
    }
    let rest: Vec<_> = components.collect();
    let Some((name, dirs)) = rest.split_last() else {
        return false;
    };
    name.as_os_str() == "tests.rs" || dirs.iter().any(|d| d.as_os_str() == "tests")
}

/// Count lines containing at least one code token: every line except
/// blank lines and lines fully inside comments. String/character content
/// counts as code (it is tokens, not comments).
pub fn count_code_lines(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut state = LexState::Normal;
    let mut block_depth: u32 = 0;
    let mut raw_hashes: usize = 0;
    let mut line_has_code = false;
    let mut count = 0;
    let mut index = 0;

    let end_line = |line_has_code: &mut bool, count: &mut usize| {
        if *line_has_code {
            *count += 1;
        }
        *line_has_code = false;
    };

    while index < chars.len() {
        let c = chars[index];
        let next = chars.get(index + 1).copied();
        match state {
            LexState::Normal => {
                if c == '\n' {
                    end_line(&mut line_has_code, &mut count);
                    index += 1;
                } else if c == '/' && next == Some('/') {
                    state = LexState::LineComment;
                    index += 2;
                } else if c == '/' && next == Some('*') {
                    state = LexState::BlockComment;
                    block_depth = 1;
                    index += 2;
                } else if c == '"' || c == '\'' {
                    if c == '\'' && !looks_like_char_start(&chars, index) {
                        // Lifetime, not a character literal.
                        if !c.is_whitespace() {
                            line_has_code = true;
                        }
                        index += 1;
                    } else {
                        state = if c == '"' {
                            LexState::String
                        } else {
                            LexState::Char
                        };
                        line_has_code = true;
                        index += 1;
                    }
                } else if (c == 'r' || c == 'b' || c == 'c')
                    && let Some(hashes) = raw_open_hashes(&chars, index)
                {
                    state = LexState::RawString;
                    raw_hashes = hashes.0;
                    line_has_code = true;
                    index = hashes.1;
                } else {
                    if !c.is_whitespace() {
                        line_has_code = true;
                    }
                    // A `b"`, `c"`, or `r"`-less prefix still falls through:
                    // the quote itself is handled on its own iteration.
                    index += 1;
                }
            }
            LexState::LineComment => {
                if c == '\n' {
                    state = LexState::Normal;
                    end_line(&mut line_has_code, &mut count);
                }
                index += 1;
            }
            LexState::BlockComment => {
                if c == '\n' {
                    end_line(&mut line_has_code, &mut count);
                    index += 1;
                } else if c == '/' && next == Some('*') {
                    block_depth += 1;
                    index += 2;
                } else if c == '*' && next == Some('/') {
                    block_depth -= 1;
                    if block_depth == 0 {
                        state = LexState::Normal;
                    }
                    index += 2;
                } else {
                    index += 1;
                }
            }
            LexState::String | LexState::Char => {
                let quote = if state == LexState::String { '"' } else { '\'' };
                line_has_code = true;
                if c == '\\' {
                    index += 2;
                } else if c == '\n' {
                    // Newline inside a string: the line held string content.
                    end_line(&mut line_has_code, &mut count);
                    line_has_code = false;
                    index += 1;
                    // Keep string state: Rust strings may span lines only
                    // with `\` continuation, but counting tolerantly here
                    // avoids misclassifying the rest of a file.
                } else if c == quote {
                    state = LexState::Normal;
                    index += 1;
                } else {
                    index += 1;
                }
            }
            LexState::RawString => {
                if c == '\n' {
                    end_line(&mut line_has_code, &mut count);
                    line_has_code = false;
                    index += 1;
                } else if c == '"' && raw_close(&chars, index, raw_hashes) {
                    state = LexState::Normal;
                    index += 1 + raw_hashes;
                } else {
                    line_has_code = true;
                    index += 1;
                }
            }
        }
    }
    end_line(&mut line_has_code, &mut count);
    count
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexState {
    Normal,
    LineComment,
    BlockComment,
    String,
    Char,
    RawString,
}

/// True when `'` at `index` opens a character literal (`'x'`, `'\n'`,
/// `'abc'`-tolerant) rather than a lifetime.
fn looks_like_char_start(chars: &[char], index: usize) -> bool {
    let rest = &chars[index + 1..];
    if rest.first() == Some(&'\\') {
        // Escaped char: closing quote after the escape sequence.
        return rest.iter().skip(1).any(|c| *c == '\'');
    }
    // `'x'`: exactly one char before the closing quote; anything longer
    // (`'a`, `'static`) is a lifetime.
    rest.get(1) == Some(&'\'')
}

/// When `index` opens a raw string (`r"`, `r#"..."#`, `br"..."`),
/// return `(hash count, index after the opening quote)`.
fn raw_open_hashes(chars: &[char], index: usize) -> Option<(usize, usize)> {
    // The opener must not be an identifier tail (`bar"` is not raw).
    if index > 0 && (chars[index - 1].is_alphanumeric() || chars[index - 1] == '_') {
        return None;
    }
    let mut cursor = index;
    if chars[cursor] == 'b' || chars[cursor] == 'c' {
        // `b"`/`c"` are plain prefixed strings, not raw; only `br"` is raw.
        if chars[cursor] == 'c' {
            return None;
        }
        cursor += 1;
        if chars.get(cursor) != Some(&'r') {
            return None;
        }
    }
    if chars.get(cursor) != Some(&'r') {
        return None;
    }
    cursor += 1;
    let mut hashes = 0;
    while chars.get(cursor) == Some(&'#') {
        hashes += 1;
        cursor += 1;
    }
    if chars.get(cursor) == Some(&'"') {
        Some((hashes, cursor + 1))
    } else {
        None
    }
}

/// True when the `"` at `index` closes a raw string with `hashes` hashes.
fn raw_close(chars: &[char], index: usize, hashes: usize) -> bool {
    (0..hashes).all(|h| chars.get(index + 1 + h) == Some(&'#'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_and_comments_do_not_count() {
        let text = "\n// line comment\n   \n/* block */\nfn f() {} // trailing\n";
        assert_eq!(count_code_lines(text), 1);
    }

    #[test]
    fn block_comments_nest_and_span_lines() {
        let text = "fn f() {\n/* outer\n/* inner */\nstill comment */\n}\n";
        assert_eq!(count_code_lines(text), 2);
    }

    #[test]
    fn comment_markers_inside_strings_do_not_start_comments() {
        let text = "let a = \"// not a comment\";\nlet b = \"/* nor this */\";\n";
        assert_eq!(count_code_lines(text), 2);
    }

    #[test]
    fn raw_strings_hide_comment_markers_and_test_text() {
        let text = "let a = r#\"\n// inside raw\n#[test]\n\"#;\nfn f() {}\n";
        // Three raw-content lines plus the opener/closer lines and `fn f`.
        assert_eq!(count_code_lines(text), 5);
    }

    #[test]
    fn byte_raw_strings_and_identifier_tails() {
        let text = "let a = br\"// x\";\nlet bar = 1;\n";
        assert_eq!(count_code_lines(text), 2);
    }

    #[test]
    fn lifetimes_are_not_char_literals() {
        let text = "fn f<'a>(x: &'a str) -> &'a str {\nx\n}\n";
        assert_eq!(count_code_lines(text), 3);
    }

    #[test]
    fn char_literals_count() {
        let text = "let q = 'q';\nlet e = '\\'';\n";
        assert_eq!(count_code_lines(text), 2);
    }

    #[test]
    fn string_with_test_text_counts_as_code_not_comment() {
        let text = "const T: &str = \"#[test] fn fake() {}\";\n";
        assert_eq!(count_code_lines(text), 1);
    }

    #[test]
    fn test_paths_are_excluded() {
        for rel in [
            "tests/it.rs",
            "tests/common/helpers.rs",
            "src/tests.rs",
            "src/foo/tests.rs",
            "src/tests/case.rs",
            "src/foo/tests/bar.rs",
        ] {
            assert!(is_test_path(Path::new(rel)), "{rel} must be excluded");
        }
    }

    #[test]
    fn production_paths_stay_counted() {
        for rel in [
            "src/lib.rs",
            "src/main.rs",
            "src/api.rs",
            "src/foo_tests.rs",
            "src/testing.rs",
            "src/my_tests/mod.rs",
            "src/tests.rs.inc.rs",
            "testing/foo.rs",
            "tests-utils/x.rs",
            "benches/bench.rs",
            "examples/demo.rs",
            "build.rs",
        ] {
            assert!(!is_test_path(Path::new(rel)), "{rel} must stay counted");
        }
    }

    #[test]
    fn exclusion_is_relative_to_the_package_dir() {
        let dir = Path::new("/r/crates/mixed");
        assert!(is_test_file(dir, &dir.join("tests/it.rs"),));
        assert!(is_test_file(dir, &dir.join("src/tests.rs")));
        assert!(!is_test_file(dir, &dir.join("src/lib.rs")));
        assert!(!is_test_file(dir, Path::new("/r/crates/other/tests.rs"),));
    }
}
