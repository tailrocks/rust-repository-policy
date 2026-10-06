//! Rust syntax checks: test placement, suite paths, orphans.
//!
//! Native gap: alint has no Rust parser, so it cannot see `#[test]`
//! attributes, `mod` declarations, or module reachability. Path-shape rules
//! (`file_absent`, `dir_contains`) only match names, never syntax. These
//! checks parse with `syn` (comments and string literals can therefore
//! mention test text without tripping the gate).
//!
//! Parser logic is adapted from the product-workspace gate (flat
//! `crates/*/src` discovery, exact-whitespace suite check, and hard-coded
//! fixture registries dropped; discovery now follows `cargo metadata`
//! packages, and the canonical-suite check is structural):
//!
//! Rules:
//!
//! - `GAP-TEST-001`: test implementation in a production file — a `#[test]`
//!   (or `rstest`/`test_case`/`tokio::test`-style) function, including via
//!   `cfg_attr`, `async` fns, methods in `impl`/`trait` blocks, inline test
//!   modules, non-canonical suite declarations, `macro_rules!` bodies
//!   carrying test attributes, and `include!` of test-named files. Also
//!   used when a production file cannot be parsed.
//! - `GAP-TEST-002`: non-canonical test suite path — `#[path]` on a test
//!   suite module, inline or `#[path]` child modules inside a `tests.rs`
//!   suite, or files under a `<parent>/tests/` case directory with no
//!   sibling `<parent>/tests.rs`. Canonical case splits — external
//!   `mod <case>;` in `tests.rs` resolving to `tests/<case>.rs` — are
//!   clean, so large suites can stay under the per-file size budget.
//! - `GAP-TEST-003`: orphan — an `src/` file unreachable from any crate
//!   root through `mod` declarations.
//!
//! Scope: production files are `src/**/*.rs` except `tests.rs` suites
//! and `tests/` case files. Integration tests (`tests/`), benches, and
//! examples may hold tests. `dev-dependencies` are irrelevant here; only
//! file placement matters.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::punctuated::Punctuated;
use syn::spanned::Spanned as _;
use syn::visit::Visit as _;
use syn::{parse::Parser as _, Attribute, Item, ItemMod, Meta, Token, Visibility};

use crate::finding::{rel_path, Finding};
use crate::inventory::Inventory;
use crate::meta::Metadata;
use crate::util::read_text;

/// Check syntax rules for every workspace package.
pub fn check(root: &Path, meta: &Metadata, inventory: &Inventory) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    for package in meta.workspace_packages() {
        let Some(dir) = package.manifest_path.parent() else {
            continue;
        };
        let src = dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let dir_rel = rel_path(dir, root);
        let prefix = if dir_rel.is_empty() {
            "src".to_owned()
        } else {
            format!("{dir_rel}/src")
        };
        let files = inventory.rs_files_under(root, &prefix);
        let mut parsed: BTreeMap<PathBuf, Option<syn::File>> = BTreeMap::new();
        for file in &files {
            let text = read_text(file)?;
            parsed.insert(file.clone(), syn::parse_file(&text).ok());
            if parsed[file].is_none() {
                findings.push(Finding::new(
                    "GAP-TEST-001",
                    rel_path(file, root),
                    1,
                    "Rust source could not be parsed, so the test gate cannot inspect it",
                    "fix the syntax error",
                ));
            }
        }
        check_test_placement(root, &src, &files, &parsed, &mut findings);
        check_orphans(root, package, &src, &files, &parsed, &mut findings);
    }
    Ok(findings)
}

/// True when `file` sits under a `<parent>/tests/` case directory (any
/// depth under `src/`; the `tests` segment must be a directory, never the
/// file name itself).
fn is_case_dir_file(file: &Path, src: &Path) -> bool {
    let Some(rel) = file.strip_prefix(src).ok() else {
        return false;
    };
    let components: Vec<_> = rel.components().collect();
    components.len() > 1
        && components[..components.len() - 1]
            .iter()
            .any(|c| c.as_os_str() == "tests")
}

/// True when the `<parent>/tests/` case directory holding `file` has a
/// sibling `<parent>/tests.rs` suite file present in the inventory.
fn has_sibling_suite(
    file: &Path,
    src: &Path,
    parsed: &BTreeMap<PathBuf, Option<syn::File>>,
) -> bool {
    let Some(rel) = file.strip_prefix(src).ok() else {
        return false;
    };
    let components: Vec<_> = rel.components().collect();
    let Some(idx) = components[..components.len().saturating_sub(1)]
        .iter()
        .rposition(|c| c.as_os_str() == "tests")
    else {
        return false;
    };
    let mut sibling = src.to_path_buf();
    sibling.extend(components[..idx].iter().map(|c| c.as_os_str()));
    sibling.push("tests.rs");
    parsed.contains_key(&sibling)
}

/// `GAP-TEST-001` / `GAP-TEST-002`: per-file syntax findings.
fn check_test_placement(
    root: &Path,
    src: &Path,
    files: &[PathBuf],
    parsed: &BTreeMap<PathBuf, Option<syn::File>>,
    findings: &mut Vec<Finding>,
) {
    for file in files {
        let rel = rel_path(file, root);
        if is_case_dir_file(file, src) {
            // Case files may hold tests either way; only the suite-less
            // (non-canonical) ones are flagged here. Canonical case files
            // still go through the orphan check below.
            if !has_sibling_suite(file, src, parsed) {
                findings.push(Finding::new(
                    "GAP-TEST-002",
                    rel,
                    1,
                    "split-out test file under src/tests/ instead of a single sibling tests.rs",
                    "move the tests into the canonical sibling tests.rs and delete this file",
                ));
            }
            continue;
        }
        let Some(Some(ast)) = parsed.get(file) else {
            continue; // Parse failure already reported.
        };
        let is_suite = file.file_name().is_some_and(|n| n == "tests.rs");
        if is_suite {
            if let Some(item_finding) = suite_child_violation(&ast.items) {
                findings.push(Finding::new(
                    item_finding.rule,
                    rel,
                    item_finding.line,
                    item_finding.message,
                    item_finding.correction,
                ));
            }
            continue;
        }
        for item_finding in inspect_items(&ast.items) {
            findings.push(Finding::new(
                item_finding.rule,
                rel.clone(),
                item_finding.line,
                item_finding.message,
                item_finding.correction,
            ));
        }
    }
}

/// A syntax finding before the file path is attached.
struct ItemFinding {
    rule: &'static str,
    line: usize,
    message: String,
    correction: String,
}

impl ItemFinding {
    fn test(line: usize, message: impl Into<String>, correction: impl Into<String>) -> Self {
        Self {
            rule: "GAP-TEST-001",
            line,
            message: message.into(),
            correction: correction.into(),
        }
    }

    fn suite_path(line: usize, message: impl Into<String>, correction: impl Into<String>) -> Self {
        Self {
            rule: "GAP-TEST-002",
            line,
            message: message.into(),
            correction: correction.into(),
        }
    }
}

/// Inspect items (and nested inline modules) for test implementations.
fn inspect_items(items: &[Item]) -> Vec<ItemFinding> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                out.push(ItemFinding::test(
                    attr_line(&function.attrs),
                    "direct test function in a production file",
                    "move the test to the sibling tests.rs",
                ));
            }
            Item::Mod(module) => {
                if let Some(finding) = suite_violation(module) {
                    out.push(finding);
                }
                if let Some((_, nested)) = &module.content {
                    out.extend(inspect_items(nested));
                }
            }
            Item::Impl(block) => {
                for impl_item in &block.items {
                    if let syn::ImplItem::Fn(method) = impl_item
                        && has_test_attribute(&method.attrs)
                    {
                        out.push(ItemFinding::test(
                            attr_line(&method.attrs),
                            "test method in a production file",
                            "move the test to the sibling tests.rs",
                        ));
                    }
                }
            }
            Item::Trait(def) => {
                for trait_item in &def.items {
                    if let syn::TraitItem::Fn(method) = trait_item
                        && has_test_attribute(&method.attrs)
                    {
                        out.push(ItemFinding::test(
                            attr_line(&method.attrs),
                            "test method in a production file",
                            "move the test to the sibling tests.rs",
                        ));
                    }
                }
            }
            Item::Macro(mac) => {
                out.extend(inspect_macro(mac));
            }
            _ => {}
        }
    }
    out
}

/// 1-based line of the first attribute, or 1 when there are none.
fn attr_line(attrs: &[Attribute]) -> usize {
    attrs.first().map_or(1, |a| a.span().start().line.max(1))
}

/// Suite-module check: `#[path]` suites are non-canonical paths
/// (`GAP-TEST-002`); other test-gated or suite-named modules must be
/// exactly the canonical external suite or they are test implementations
/// in a production file (`GAP-TEST-001`).
fn suite_violation(module: &ItemMod) -> Option<ItemFinding> {
    let name = module.ident.to_string();
    let test_gated = module.attrs.iter().any(attribute_mentions_test_cfg);
    let suite_named = name == "tests" || name.ends_with("_tests");
    if !(test_gated || suite_named) {
        return None;
    }
    if has_path_attr(module) {
        return Some(ItemFinding::suite_path(
            attr_line(&module.attrs),
            format!("test suite module `{name}` uses #[path] instead of the canonical location"),
            "drop #[path] so the suite resolves to the canonical sibling tests.rs",
        ));
    }
    if is_canonical_suite(module) {
        return None;
    }
    Some(ItemFinding::test(
        attr_line(&module.attrs).max(module.mod_token.span().start().line.max(1)),
        "test suite module must be exactly `#[cfg(test)] mod tests;` \
         (private, external, single cfg(test) attribute)",
        "declare exactly `#[cfg(test)] mod tests;` and move the body to the sibling tests.rs",
    ))
}

/// Structural canonical-suite check (no whitespace requirements).
fn is_canonical_suite(module: &ItemMod) -> bool {
    module.ident == "tests"
        && matches!(module.vis, Visibility::Inherited)
        && module.content.is_none()
        && module.semi.is_some()
        && module.attrs.len() == 1
        && module.attrs.first().is_some_and(is_exact_cfg_test)
}

/// True for exactly `#[cfg(test)]` (token-normalized, whitespace-free).
fn is_exact_cfg_test(attr: &Attribute) -> bool {
    attr.path().is_ident("cfg")
        && matches!(&attr.meta, Meta::List(list) if list.tokens.to_string() == "test")
}

/// True when any attribute (including `cfg_attr` nesting) mentions `cfg(test)`.
fn attribute_mentions_test_cfg(attr: &Attribute) -> bool {
    (attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        && meta_tokens_contain_test(&attr.meta)
}

fn meta_tokens_contain_test(meta: &Meta) -> bool {
    let Meta::List(list) = meta else {
        return false;
    };
    tokens_contain_test(list.tokens.clone())
}

fn tokens_contain_test(tokens: proc_macro2::TokenStream) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => ident == "test",
        proc_macro2::TokenTree::Group(group) => tokens_contain_test(group.stream()),
        _ => false,
    })
}

/// True when any attribute is a test attribute (direct or via `cfg_attr`).
fn has_test_attribute(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| is_test_path(attr.path()) || cfg_attr_adds_test(attr))
}

/// True for `#[test]`, `#[tokio::test]`, `#[rstest]`, `#[test_case]`, etc.:
/// any attribute path whose last segment is a known test marker.
fn is_test_path(path: &syn::Path) -> bool {
    path.segments.last().is_some_and(|segment| {
        matches!(
            segment.ident.to_string().as_str(),
            "test" | "rstest" | "test_case"
        )
    })
}

fn cfg_attr_adds_test(attr: &Attribute) -> bool {
    if !attr.path().is_ident("cfg_attr") {
        return false;
    }
    cfg_attr_meta_adds_test(&attr.meta)
}

fn is_test_meta(meta: &Meta) -> bool {
    is_test_path(meta.path()) || cfg_attr_meta_adds_test(meta)
}

fn cfg_attr_meta_adds_test(meta: &Meta) -> bool {
    if !meta.path().is_ident("cfg_attr") {
        return false;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    let Ok(metas) = parser.parse2(list.tokens.clone()) else {
        return false;
    };
    metas.iter().skip(1).any(is_test_meta)
}

/// True when the module carries a `#[path = ...]` attribute.
fn has_path_attr(module: &ItemMod) -> bool {
    module.attrs.iter().any(|a| a.path().is_ident("path"))
}

/// First non-canonical child `mod` inside a suite file. Canonical case
/// splits are external `mod <case>;` declarations (resolving to
/// `tests/<case>.rs`); inline bodies and `#[path]` overrides stay
/// `GAP-TEST-002`.
fn suite_child_violation(items: &[Item]) -> Option<ItemFinding> {
    struct Finder(Option<ItemFinding>);

    impl<'ast> syn::visit::Visit<'ast> for Finder {
        fn visit_item_mod(&mut self, module: &'ast ItemMod) {
            if self.0.is_some() {
                return;
            }
            let name = module.ident.to_string();
            if module.content.is_some() {
                self.0 = Some(ItemFinding::suite_path(
                    module.mod_token.span().start().line.max(1),
                    format!(
                        "inline child module `{name}` in tests.rs instead of a canonical case split"
                    ),
                    format!("move the module body to tests/{name}.rs and declare `mod {name};`"),
                ));
            } else if has_path_attr(module) {
                self.0 = Some(ItemFinding::suite_path(
                    attr_line(&module.attrs),
                    format!(
                        "child module `{name}` in tests.rs uses #[path] instead of resolving to tests/{name}.rs"
                    ),
                    "drop #[path] so the module resolves to the canonical tests/<case>.rs file",
                ));
            }
        }
    }

    let mut finder = Finder(None);
    for item in items {
        finder.visit_item(item);
    }
    finder.0
}

/// Macro-item checks: `macro_rules!` bodies carrying test attributes, and
/// `include!` of test-named files.
fn inspect_macro(mac: &syn::ItemMacro) -> Vec<ItemFinding> {
    if mac.mac.path.is_ident("include") {
        return inspect_include(mac);
    }
    if !(mac.ident.is_some() && mac.mac.path.is_ident("macro_rules")) {
        return Vec::new();
    }
    macro_test_attr_line(mac.mac.tokens.clone())
        .map(|line| {
            vec![ItemFinding::test(
                line,
                "test attribute inside a macro_rules! body in a production file",
                "move the test-generating macro to the sibling tests.rs",
            )]
        })
        .unwrap_or_default()
}

/// `include!("...")` with a test-named file stem.
fn inspect_include(mac: &syn::ItemMacro) -> Vec<ItemFinding> {
    let Ok(path) = syn::parse2::<syn::LitStr>(mac.mac.tokens.clone()) else {
        return Vec::new();
    };
    let stem = Path::new(&path.value())
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !is_test_stem(&stem) {
        return Vec::new();
    }
    vec![ItemFinding::test(
        mac.mac.path.span().start().line.max(1),
        format!(
            "include! of test-named file `{}` in a production file",
            path.value()
        ),
        "move the include! to the sibling tests.rs",
    )]
}

/// True for test file stems: `test`, `tests`, or `test(s)` joined to other
/// words with underscores (`foo_tests`, `test_foo`). Plain substrings such
/// as `latest` do not match.
fn is_test_stem(stem: &str) -> bool {
    stem == "test"
        || stem == "tests"
        || stem.starts_with("test_")
        || stem.starts_with("tests_")
        || stem.ends_with("_test")
        || stem.ends_with("_tests")
        || stem.contains("_test_")
        || stem.contains("_tests_")
}

/// Scan macro-definition tokens for `#<test attr>]`, skipping literals so
/// strings stay valid. Returns the 1-based line of the `#`.
fn macro_test_attr_line(tokens: proc_macro2::TokenStream) -> Option<usize> {
    let mut pending_hash: Option<proc_macro2::Span> = None;
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Punct(p) if p.as_char() == '#' => {
                pending_hash = Some(p.span());
            }
            proc_macro2::TokenTree::Group(group)
                if pending_hash.is_some()
                    && group.delimiter() == proc_macro2::Delimiter::Bracket
                    && group_contains_test_path(&group.stream()) =>
            {
                let line = pending_hash
                    .expect("guarded by is_some")
                    .start()
                    .line
                    .max(1);
                return Some(line);
            }
            proc_macro2::TokenTree::Group(group) => {
                if let Some(line) = macro_test_attr_line(group.stream()) {
                    return Some(line);
                }
                pending_hash = None;
            }
            proc_macro2::TokenTree::Literal(_) => {
                pending_hash = None;
            }
            _ => {
                if !matches!(token, proc_macro2::TokenTree::Punct(_)) {
                    pending_hash = None;
                }
            }
        }
    }
    None
}

/// True when a `[...]` attribute token stream is (or contains, for
/// `cfg_attr` nesting) a test attribute path.
fn group_contains_test_path(tokens: &proc_macro2::TokenStream) -> bool {
    let text = tokens.to_string().replace(' ', "");
    if is_test_path_text(&text) {
        return true;
    }
    // Recurse into nested groups for `cfg_attr(.., test)`-style payloads.
    tokens.clone().into_iter().any(|token| match token {
        proc_macro2::TokenTree::Group(group) => group_contains_test_path(&group.stream()),
        _ => false,
    })
}

/// True when whitespace-stripped attribute text is a test path with optional
/// `(...)` arguments: `test`, `tokio::test`, `test_case(..)`.
fn is_test_path_text(text: &str) -> bool {
    let head = text.split('(').next().unwrap_or("");
    head.split("::")
        .last()
        .is_some_and(|last| matches!(last, "test" | "rstest" | "test_case"))
        && !head.contains([',', '=', '#'])
}

/// `GAP-TEST-003`: files under `src/` unreachable from crate roots.
fn check_orphans(
    root: &Path,
    package: &crate::meta::MetaPackage,
    src: &Path,
    files: &[PathBuf],
    parsed: &BTreeMap<PathBuf, Option<syn::File>>,
    findings: &mut Vec<Finding>,
) {
    let mut roots: Vec<PathBuf> = package
        .targets
        .iter()
        .filter(|t| {
            t.kind
                .iter()
                .any(|k| matches!(k.as_str(), "lib" | "bin" | "proc-macro"))
        })
        .map(|t| t.src_path.clone())
        .collect();
    roots.sort();
    roots.dedup();

    let mut reachable: BTreeSet<PathBuf> = BTreeSet::new();
    let mut stack = roots;
    while let Some(file) = stack.pop() {
        if !reachable.insert(canonical_key(&file)) {
            continue;
        }
        let Some(Some(ast)) = parsed.get(&file) else {
            continue;
        };
        for child in child_module_files(&file, &ast.items) {
            if child.exists() && !reachable.contains(&canonical_key(&child)) {
                stack.push(child);
            }
        }
    }

    for file in files {
        if is_case_dir_file(file, src) && !has_sibling_suite(file, src, parsed) {
            continue; // Already flagged as GAP-TEST-002.
        }
        if parsed.get(file).is_some_and(Option::is_none) {
            continue; // Parse failure already reported.
        }
        if !reachable.contains(&canonical_key(file)) {
            let name = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            findings.push(Finding::new(
                "GAP-TEST-003",
                rel_path(file, root),
                1,
                "orphan Rust file: unreachable from any crate root through mod declarations",
                format!("declare `mod {name};` from a reachable module or delete the file"),
            ));
        }
    }
}

/// Lexical path key (no IO): resolves `.`/`..` textually for comparison.
fn canonical_key(path: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir => {}
            other => parts.push(other.as_os_str().to_owned()),
        }
    }
    parts.iter().collect()
}

/// Resolve external child modules (plus `include!`d files, which the
/// compiler also pulls in) of `file` to candidate file paths.
fn child_module_files(file: &Path, items: &[Item]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Macro(mac) if mac.mac.path.is_ident("include") => {
                if let Ok(path) = syn::parse2::<syn::LitStr>(mac.mac.tokens.clone())
                    && let Some(parent) = file.parent()
                {
                    out.push(parent.join(path.value()));
                }
                continue;
            }
            Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    // Inline modules share their parent's file; recurse for
                    // declarations nested inside.
                    out.extend(child_module_files(file, nested));
                    continue;
                }
                out.extend(resolve_external_module(file, module));
            }
            _ => {}
        }
    }
    out
}

/// Resolve one external `mod` declaration to candidate file paths.
fn resolve_external_module(file: &Path, module: &ItemMod) -> Vec<PathBuf> {
    if let Some(path_attr) = module.attrs.iter().find(|a| a.path().is_ident("path")) {
        if let Meta::NameValue(named) = &path_attr.meta
            && let syn::Expr::Lit(lit) = &named.value
            && let syn::Lit::Str(path_str) = &lit.lit
        {
            return vec![module_dir(file).join(path_str.value())];
        }
        return Vec::new();
    }
    let name = module.ident.to_string();
    let dir = module_dir(file);
    vec![
        dir.join(format!("{name}.rs")),
        dir.join(name).join("mod.rs"),
    ]
}

/// The directory child modules of `file` resolve against: crate roots
/// (`lib.rs`/`main.rs`) and `mod.rs` use their own directory, every other
/// `foo.rs` uses `foo/`.
fn module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or_else(|| Path::new("."));
    if file
        .file_name()
        .is_some_and(|n| n == "mod.rs" || n == "lib.rs" || n == "main.rs")
    {
        parent.to_path_buf()
    } else if let Some(stem) = file.file_stem() {
        parent.join(stem)
    } else {
        parent.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspect(text: &str) -> Vec<ItemFinding> {
        let ast = syn::parse_file(text).expect("test source parses");
        inspect_items(&ast.items)
    }

    fn rules(text: &str) -> Vec<&'static str> {
        inspect(text).iter().map(|f| f.rule).collect()
    }

    #[test]
    fn canonical_suite_any_whitespace_is_clean() {
        assert!(rules("#[cfg(test)] mod tests;\n").is_empty());
        assert!(rules("#[cfg(test)]\nmod tests;\n").is_empty());
        assert!(rules("#[ cfg ( test ) ] mod tests ;\n").is_empty());
    }

    #[test]
    fn direct_test_fn_flagged() {
        let findings = inspect("#[test]\nfn it() {}\n");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "GAP-TEST-001");
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn async_tokio_test_flagged() {
        let findings = inspect("#[tokio::test]\nasync fn it() {}\n");
        assert_eq!(
            rules("#[tokio::test]\nasync fn it() {}\n"),
            vec!["GAP-TEST-001"]
        );
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn cfg_attr_test_flagged() {
        assert_eq!(
            rules("#[cfg_attr(feature = \"x\", test)]\nfn it() {}\n"),
            vec!["GAP-TEST-001"]
        );
    }

    #[test]
    fn rstest_and_test_case_flagged() {
        assert_eq!(rules("#[rstest]\nfn it() {}\n"), vec!["GAP-TEST-001"]);
        assert_eq!(
            rules("#[test_case(1)]\nfn it(_: i32) {}\n"),
            vec!["GAP-TEST-001"]
        );
    }

    #[test]
    fn test_methods_in_impl_and_trait_flagged() {
        assert_eq!(
            rules("struct A;\nimpl A {\n#[test]\nfn t(&self) {}\n}\n"),
            vec!["GAP-TEST-001"]
        );
        assert_eq!(
            rules("trait T {\n#[test]\nfn t(&self) {}\n}\n"),
            vec!["GAP-TEST-001"]
        );
    }

    #[test]
    fn inline_test_module_flagged() {
        let findings = inspect("#[cfg(test)]\nmod tests {\n#[test]\nfn t() {}\n}\n");
        // The inline suite plus the nested test fn.
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.rule == "GAP-TEST-001"));
    }

    #[test]
    fn non_canonical_suite_forms_flagged() {
        assert_eq!(
            rules("#[cfg(test)]\npub mod tests;\n"),
            vec!["GAP-TEST-001"]
        );
        assert_eq!(rules("mod tests;\n"), vec!["GAP-TEST-001"]);
        assert_eq!(
            rules("#[cfg(all(test, feature = \"x\"))]\nmod tests;\n"),
            vec!["GAP-TEST-001"]
        );
        assert_eq!(
            rules("#[cfg(test)]\nmod unit_tests;\n"),
            vec!["GAP-TEST-001"]
        );
    }

    #[test]
    fn path_suite_is_suite_path_not_test_impl() {
        let findings = inspect("#[cfg(test)]\n#[path = \"s.rs\"]\nmod tests;\n");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "GAP-TEST-002");
    }

    #[test]
    fn plain_path_mod_is_allowed() {
        assert!(rules("#[path = \"real.rs\"]\nmod renamed;\n").is_empty());
    }

    #[test]
    fn comment_and_string_with_test_text_stay_valid() {
        let text = "// #[test]\n/* #[cfg(test)] mod tests {} */\nconst T: &str = \"#[test] fn fake() {}\";\nfn real() {}\n";
        assert!(rules(text).is_empty());
    }

    #[test]
    fn macro_body_with_test_attr_flagged() {
        let text = "macro_rules! gen {\n() => {\n#[test]\nfn t() {}\n};\n}\n";
        let findings = inspect(text);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "GAP-TEST-001");
        assert_eq!(findings[0].line, 3);
    }

    #[test]
    fn macro_body_with_test_string_stays_valid() {
        let text = "macro_rules! gen {\n() => {\nlet s = \"#[test]\";\n};\n}\n";
        assert!(rules(text).is_empty());
    }

    #[test]
    fn include_of_test_file_flagged() {
        assert_eq!(
            rules("include!(\"parser_tests.rs\");\n"),
            vec!["GAP-TEST-001"]
        );
        assert!(rules("include!(\"generated.rs\");\n").is_empty());
        assert!(rules("include!(\"latest.rs\");\n").is_empty());
    }

    #[test]
    fn suite_children_detected() {
        let ast = syn::parse_file("mod inline {}\n").expect("parses");
        let finding = suite_child_violation(&ast.items).expect("inline flagged");
        assert_eq!(finding.rule, "GAP-TEST-002");
        assert_eq!(finding.line, 1);
        let clean = syn::parse_file("#[test]\nfn t() {}\n").expect("parses");
        assert!(suite_child_violation(&clean.items).is_none());
    }

    #[test]
    fn suite_canonical_case_split_is_clean() {
        for text in ["mod extra;\n", "#[cfg(test)]\nmod extra;\n"] {
            let ast = syn::parse_file(text).expect("parses");
            assert!(suite_child_violation(&ast.items).is_none());
        }
    }

    #[test]
    fn suite_path_case_split_flagged() {
        let ast = syn::parse_file("#[path = \"other.rs\"]\nmod extra;\n").expect("parses");
        let finding = suite_child_violation(&ast.items).expect("path flagged");
        assert_eq!(finding.rule, "GAP-TEST-002");
        assert_eq!(finding.line, 1);
    }

    #[test]
    fn module_dir_rules() {
        assert_eq!(module_dir(Path::new("src/lib.rs")), PathBuf::from("src"));
        assert_eq!(module_dir(Path::new("src/main.rs")), PathBuf::from("src"));
        assert_eq!(
            module_dir(Path::new("src/foo/mod.rs")),
            PathBuf::from("src/foo")
        );
        assert_eq!(
            module_dir(Path::new("src/foo.rs")),
            PathBuf::from("src/foo")
        );
    }
}
