//! A host giving scripts new functions (`define_fn`) and calling the ones they define (`call`):
//! the shape of a game's mod runner. Runs with and without the `sqlite` feature.

use std::cell::RefCell;
use std::rc::Rc;

use eelisp::{Interpreter, LispError, Value};

#[test]
fn a_host_function_is_callable_from_scripts() {
    let it = Interpreter::new();
    let placed = Rc::new(RefCell::new(Vec::new()));
    let p = placed.clone();
    it.define_fn("set-block", move |args| {
        let n = |i: usize| match args.get(i) {
            Some(Value::Number(n)) => Ok(*n as i32),
            _ => Err(LispError::Runtime("set-block wants x y z".into())),
        };
        p.borrow_mut().push([n(0)?, n(1)?, n(2)?]);
        Ok(Value::Bool(true))
    });
    it.eval_str("(for-each y (range 0 3) (set-block 1 y 2))").unwrap();
    assert_eq!(*placed.borrow(), vec![[1, 0, 2], [1, 1, 2], [1, 2, 2]]);
    // Its errors are the script's errors.
    let err = it.eval_str("(set-block \"up\")").unwrap_err();
    assert!(err.to_string().contains("set-block wants x y z"), "{err}");
}

#[test]
fn a_host_calls_the_handlers_a_script_defines() {
    let it = Interpreter::new();
    it.set_echo(false);
    it.eval_str(
        r#"
        (def joined [])
        (defn on-join (player name)
          (set! joined (append joined [name]))
          (str "Hello " name))
        (def not-a-function 3)
        "#,
    )
    .unwrap();
    let hello = it.call("on-join", &[Value::Number(7.0), Value::Str("Ada".into())]);
    assert!(matches!(hello, Some(Ok(Value::Str(s))) if s == "Hello Ada"));
    // Handlers keep state between calls.
    it.call("on-join", &[Value::Number(8.0), Value::Str("Bo".into())]);
    assert_eq!(it.eval_str("(length joined)").unwrap().to_string(), "2");
    // Handlers a script doesn't define (or that aren't functions) are skipped.
    assert!(it.call("on-leave", &[]).is_none());
    assert!(it.call("not-a-function", &[]).is_none());
    // A handler's error comes back to the host.
    it.eval_str("(defn on-tick (t) (error \"oops\"))").unwrap();
    assert!(matches!(it.call("on-tick", &[Value::Number(1.0)]), Some(Err(_))));
}
