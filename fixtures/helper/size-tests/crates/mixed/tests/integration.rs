use mixed::run;

#[test]
fn integration_runs() {
    assert_eq!(run(), 42);
}

#[test]
fn integration_runs_twice() {
    assert_eq!(run() + run(), 84);
}

#[test]
fn integration_table() {
    let cases = [run(), run(), run()];
    for got in cases {
        assert_eq!(got, 42);
    }
}

#[test]
fn integration_repeated() {
    for _ in 0..5 {
        assert_eq!(run(), 42);
    }
}
