//! Policy helper stub.
//!
//! Owns only the checks native alint cannot express (see
//! `docs/content/policy/rule-owners.mdx`). Every subcommand below is a
//! not-implemented stub until its gap slice is specified.

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() -> &'static str {
    "usage: rust-repository-policy <check-gaps|--help|--version>"
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("check-gaps") => {
            eprintln!("not implemented: check-gaps");
            ExitCode::from(2)
        }
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
