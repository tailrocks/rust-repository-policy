use crate::greet;

#[test]
fn greets_from_case_file() {
    assert_eq!(greet(), "hi");
}
