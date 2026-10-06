//! Policy helper: checks native alint cannot express.
//!
//! See `docs/content/policy/gap-checks.mdx` for the gap catalogue. The
//! helper never compiles the checked workspace: Cargo facts come from
//! read-only `cargo metadata` runs, everything else from parsing files.

mod checks;
mod finding;
mod inventory;
mod meta;
mod util;

use std::path::PathBuf;
use std::process::ExitCode;

use checks::size::{DEFAULT_MAX_CODE_LINES, DEFAULT_MAX_FILES};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() -> &'static str {
    "usage: rust-repository-policy check-gaps [--root <dir>] \
     [--max-code-lines <n>] [--max-files <n>]"
}

struct Options {
    root: PathBuf,
    max_code_lines: usize,
    max_files: usize,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut root = std::env::current_dir().map_err(|e| format!("resolving cwd: {e}"))?;
    let mut max_code_lines = DEFAULT_MAX_CODE_LINES;
    let mut max_files = DEFAULT_MAX_FILES;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                root = args
                    .next()
                    .ok_or_else(|| "--root needs a directory".to_owned())
                    .map(PathBuf::from)?;
            }
            "--max-code-lines" => {
                max_code_lines = parse_budget(args.next(), "--max-code-lines")?;
            }
            "--max-files" => {
                max_files = parse_budget(args.next(), "--max-files")?;
            }
            "--help" | "-h" => return Ok(None),
            _ => {
                return Err(format!(
                    "unknown argument `{arg}`\n{usage}",
                    usage = usage()
                ))
            }
        }
    }
    Ok(Some(Options {
        root,
        max_code_lines,
        max_files,
    }))
}

fn parse_budget(value: Option<String>, flag: &str) -> Result<usize, String> {
    value
        .ok_or_else(|| format!("{flag} needs a number"))?
        .parse()
        .map_err(|_| format!("{flag} needs a positive integer"))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("check-gaps") => match parse_args(args) {
            Ok(Some(options)) => run(options),
            Ok(None) => {
                println!("{}", usage());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(2)
            }
        },
        Some("--help" | "-h") => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        Some("--version" | "-V") => {
            println!("rust-repository-policy {VERSION}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{}", usage());
            ExitCode::from(2)
        }
    }
}

fn run(options: Options) -> ExitCode {
    match check_all(&options) {
        Ok(findings) => {
            for finding in &findings {
                println!("{finding}");
            }
            if findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                eprintln!("{} gap(s) found", findings.len());
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("check-gaps failed: {e}");
            ExitCode::from(2)
        }
    }
}

fn check_all(options: &Options) -> Result<Vec<finding::Finding>, String> {
    // Canonicalize so metadata's absolute paths strip to relative ones even
    // when the root was passed through a symlink (e.g. /tmp on macOS).
    let root_buf = options
        .root
        .canonicalize()
        .unwrap_or_else(|_| options.root.clone());
    let root = &root_buf;
    if !root.join("Cargo.toml").is_file() {
        return Err(format!("no Cargo.toml under {}", root.display()));
    }
    let inventory = inventory::Inventory::collect(root);
    // Read-only metadata runs; the workspace is never compiled.
    let meta_no_deps = meta::run_metadata(root, true)?;
    let meta_full = meta::run_metadata(root, false)?;

    let mut findings = Vec::new();
    findings.extend(checks::inherit::check(root, &meta_no_deps)?);
    findings.extend(checks::ci::check(root, &inventory)?);
    findings.extend(checks::size::check(
        root,
        &meta_no_deps,
        &inventory,
        options.max_code_lines,
        options.max_files,
    )?);
    findings.extend(checks::syntax::check(root, &meta_no_deps, &inventory)?);
    findings.extend(checks::deps::check(root, &meta_full)?);
    findings.extend(checks::inventory::check(root, &meta_no_deps)?);
    finding::sort(&mut findings);
    Ok(findings)
}
