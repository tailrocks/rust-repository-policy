use big_lib::extra::c;
use big_lib::{a, b};

#[test]
fn sums() {
    assert_eq!(a() + b() + c(), 6);
}

#[test]
fn individual() {
    assert_eq!(a(), 1);
    assert_eq!(b(), 2);
    assert_eq!(c(), 3);
}

#[test]
fn repeated() {
    for _ in 0..10 {
        assert_eq!(a() + b() + c(), 6);
    }
}

#[test]
fn table() {
    let cases = [(a(), 1), (b(), 2), (c(), 3)];
    for (got, want) in cases {
        assert_eq!(got, want);
    }
}
