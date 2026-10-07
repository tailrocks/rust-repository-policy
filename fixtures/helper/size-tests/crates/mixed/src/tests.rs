mod case;

use super::run;

#[test]
fn runs() {
    assert_eq!(run(), 42);
}
