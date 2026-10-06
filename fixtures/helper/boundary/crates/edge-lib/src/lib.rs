//! Edge crate for budget boundaries.
// #[test] stays valid in comments.
/* block comment
   #[test] nested /* deeper */ still comment */
pub fn answer() -> i32 {
    let text = "#[test] fn fake() {}"; // trailing comment
    let raw = r#"// not a comment"#;
    let _ = text.len() + raw.len();
    42
}

fn pick<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}

#[cfg(test)]
mod tests;
