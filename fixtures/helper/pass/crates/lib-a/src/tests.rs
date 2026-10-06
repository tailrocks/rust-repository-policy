use super::greet;

#[test]
fn greets() {
    assert_eq!(greet(), "hi");
}
