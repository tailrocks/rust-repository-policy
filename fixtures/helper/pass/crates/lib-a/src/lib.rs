//! Demo library.
// #[test] in a line comment stays valid.
/* #[cfg(test)] mod tests {} in a block comment stays valid. */

/// Greeting. `"#[test]"` inside docs stays valid.
pub fn greet() -> &'static str {
    let decoy = "#[test] fn fake() {}";
    assert!(!decoy.is_empty());
    "hi"
}

#[cfg(test)]
mod tests;
