//! Small shared helpers: file IO, manifest line search, directory walks.

use std::fs;
use std::path::Path;

/// Read a UTF-8 file or return a displayable error.
pub fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

/// Find the 1-based line of `key = ...` inside the TOML `section` (exact
/// `[section]` header match). Returns 1 when not found.
pub fn toml_key_line(text: &str, section: &str, key: &str) -> usize {
    let header = format!("[{section}]");
    let mut in_section = false;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == header;
            continue;
        }
        if in_section && is_toml_key_line(trimmed, key) {
            return index + 1;
        }
    }
    1
}

/// True when a trimmed TOML line assigns `key` (`key = ...` or `key.whatever`).
fn is_toml_key_line(trimmed: &str, key: &str) -> bool {
    trimmed.strip_prefix(key).is_some_and(|rest| {
        matches!(rest.chars().next(), Some('=' | '.' | ' ' | '\t')) && rest.contains('=')
    })
}

/// Find the 1-based line of a dependency entry named `name` in a manifest:
/// either `name = ...` under a `*dependencies` section or a
/// `[..dependencies.name]` header. Returns 1 when not found.
pub fn dep_entry_line(text: &str, name: &str) -> usize {
    let mut in_deps = false;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(header) = trimmed.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            let header = header.trim();
            // `[dependencies.name]`-style header for this exact entry.
            if header
                .rsplit('.')
                .next()
                .is_some_and(|last| last.trim() == name)
                && header.contains("dependencies")
            {
                return index + 1;
            }
            let last = header.rsplit('.').next().unwrap_or("").trim();
            in_deps = matches!(
                last,
                "dependencies" | "dev-dependencies" | "build-dependencies"
            );
            continue;
        }
        if in_deps && is_toml_key_line(trimmed, name) {
            return index + 1;
        }
    }
    1
}

/// Find the 1-based line of a `[lints]` (or `[lints.<sub>]`) header.
/// Returns 1 when absent.
pub fn lints_header_line(text: &str) -> usize {
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "[lints]" || trimmed.starts_with("[lints.") {
            return index + 1;
        }
    }
    1
}

/// Find the 1-based line containing `"needle"` (JSON string search for
/// registry files). Returns 1 when not found.
pub fn json_string_line(text: &str, needle: &str) -> usize {
    let quoted = format!("\"{needle}\"");
    for (index, line) in text.lines().enumerate() {
        if line.contains(&quoted) {
            return index + 1;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"[package]
name = "a"
edition = "2021"
version.workspace = true

[dependencies]
foo = "1"
bar.workspace = true

[target.'cfg(unix)'.dependencies]
baz = "2"

[dependencies.qux]
version = "3"
"#;

    #[test]
    fn key_line_matches_exact_section() {
        assert_eq!(toml_key_line(MANIFEST, "package", "edition"), 3);
        assert_eq!(toml_key_line(MANIFEST, "package", "missing"), 1);
        // `version.workspace` is inherited, but the line still resolves.
        assert_eq!(toml_key_line(MANIFEST, "package", "version"), 4);
    }

    #[test]
    fn dep_line_covers_inline_target_and_header_forms() {
        assert_eq!(dep_entry_line(MANIFEST, "foo"), 7);
        assert_eq!(dep_entry_line(MANIFEST, "bar"), 8);
        assert_eq!(dep_entry_line(MANIFEST, "baz"), 11);
        assert_eq!(dep_entry_line(MANIFEST, "qux"), 13);
        assert_eq!(dep_entry_line(MANIFEST, "nope"), 1);
    }

    #[test]
    fn lints_header_line_found_or_one() {
        assert_eq!(lints_header_line("[lints]\nworkspace = true\n"), 1);
        assert_eq!(lints_header_line("[package]\nname = \"a\"\n"), 1);
        assert_eq!(lints_header_line("[package]\n[lints]\n"), 2);
        assert_eq!(lints_header_line("[package]\n[lints.rust]\n"), 2);
    }

    #[test]
    fn json_string_line_finds_quoted_needle() {
        let text = "{\n  \"recipes\": [\n    \"check\",\n    \"lint\"\n  ]\n}";
        assert_eq!(json_string_line(text, "lint"), 4);
        assert_eq!(json_string_line(text, "nope"), 1);
    }
}
