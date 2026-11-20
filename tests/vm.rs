//! The bytecode VM (roadmap step 1), driven by hand-written bytecode through `vm-assemble`.
//! Until the compiler exists, these programs are what it will emit, written out by hand.

use eelisp::printer::print_value;
use eelisp::{Interpreter, Value};

fn s(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).unwrap_or_else(|e| panic!("{src}\n→ {e}")), false)
}

fn err(it: &Interpreter, src: &str) -> String {
    match it.eval_str(src) {
        Ok(v) => panic!("expected an error, got {}", print_value(&v, false)),
        Err(e) => e.to_string(),
    }
}

/// `(def name (vm-assemble '<proto>))`
fn def(it: &Interpreter, name: &str, proto: &str) {
    s(it, &format!("(def {name} (vm-assemble '{proto}))"));
}

const FACT: &str = r#"(proto :name "fact" :params (n) :code (
    (load <=) (load n) (const 1) (call 2) (jump-if-false more)
    (const 1) (return)
    (label more)
    (load *) (load n) (load fact) (load -) (load n) (const 1) (call 2) (call 1) (tail-call 2)))"#;

const DOWN: &str = r#"(proto :name "down" :params (n) :code (
    (load =) (load n) (const 0) (call 2) (jump-if-false go)
    (const "done") (return)
    (label go)
    (load down) (load -) (load n) (const 1) (call 2) (tail-call 1)))"#;

#[test]
fn a_builtin_call() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "((vm-assemble '(proto :params (a b) :code ((load +) (load a) (load b) (tail-call 2)))) 2 3)"), "5");
}

#[test]
fn branches() {
    let it = Interpreter::new();
    def(&it, "pick", r#"(proto :params (c) :code (
        (load c) (jump-if-false no) (const "yes") (return) (label no) (const "no") (return)))"#);
    assert_eq!(s(&it, "(pick true)"), "yes");
    assert_eq!(s(&it, "(pick nil)"), "no");
    // only false and nil are false
    assert_eq!(s(&it, "(pick 0)"), "yes");
}

#[test]
fn recursion() {
    let it = Interpreter::new();
    def(&it, "fact", FACT);
    assert_eq!(s(&it, "(fact 10)"), "3628800");
}

#[test]
fn tail_calls_run_in_constant_space() {
    let it = Interpreter::new();
    def(&it, "down", DOWN);
    assert_eq!(s(&it, "(down 1000000)"), "done");
}

#[test]
fn deep_non_tail_recursion_is_an_error_not_a_crash() {
    let it = Interpreter::new();
    def(&it, "deep", r#"(proto :name "deep" :params (n) :code (
        (load =) (load n) (const 0) (call 2) (jump-if-false go) (const 0) (return)
        (label go) (load +) (const 1) (load deep) (load -) (load n) (const 1) (call 2) (call 1) (tail-call 2)))"#);
    assert_eq!(s(&it, "(deep 1000)"), "1000");
    assert!(err(&it, "(deep 200000)").contains("stack overflow"));
}

#[test]
fn closures_capture_their_environment() {
    let it = Interpreter::new();
    def(&it, "adder", r#"(proto :params (x) :code (
        (closure (proto :params (y) :code ((load +) (load x) (load y) (tail-call 2))))
        (return)))"#);
    assert_eq!(s(&it, "((adder 10) 5)"), "15");
    assert_eq!(s(&it, "(def add3 (adder 3)) (add3 4)"), "7");
}

#[test]
fn parameters_bind_like_the_interpreter() {
    let it = Interpreter::new();
    def(&it, "f", "(proto :params (a b . more) :code ((load list) (load a) (load b) (load more) (tail-call 3)))");
    assert_eq!(s(&it, "(f 1)"), "(1 nil ())");
    assert_eq!(s(&it, "(f 1 2 3 4)"), "(1 2 (3 4))");
}

#[test]
fn scopes_def_and_set() {
    let it = Interpreter::new();
    s(&it, "(def counter 0)");
    def(&it, "bump", "(proto :code (
        (push-scope) (const 5) (def step) (pop)
        (load +) (load counter) (load step) (call 2) (set counter)
        (pop-scope) (return)))");
    assert_eq!(s(&it, "(bump)"), "5");
    assert_eq!(s(&it, "(bump)"), "10");
    assert_eq!(s(&it, "counter"), "10");
    // `step` lived in the pushed scope only
    assert!(err(&it, "step").contains("Undefined symbol"));
}

#[test]
fn and_or_short_circuit() {
    let it = Interpreter::new();
    // (and a b): a's value if false, else b
    def(&it, "both", "(proto :params (a b) :code ((load a) (jump-if-false-or-pop end) (load b) (label end) (return)))");
    def(&it, "either", "(proto :params (a b) :code ((load a) (jump-if-true-or-pop end) (load b) (label end) (return)))");
    assert_eq!(s(&it, "(both 1 2)"), "2");
    assert_eq!(s(&it, "(both false 2)"), "false");
    assert_eq!(s(&it, "(both nil 2)"), "nil");
    assert_eq!(s(&it, "(either nil 2)"), "2");
    assert_eq!(s(&it, "(either 1 2)"), "1");
}

const SUM_LOOP: &str = r#"(proto :params (limit) :code (
    (push-scope)
    (const 0) (def i) (pop) (const 0) (def acc) (pop)
    (loop i acc)
    (load >) (load i) (load limit) (call 2) (jump-if-false body)
    (load acc) (jump end)
    (label body)
    (load +) (load i) (const 1) (call 2)
    (load +) (load acc) (load i) (call 2)
    (recur 2)
    (label end)
    (end-loop) (pop-scope) (return)))"#;

#[test]
fn loop_and_recur() {
    let it = Interpreter::new();
    def(&it, "sum-to", SUM_LOOP);
    assert_eq!(s(&it, "(sum-to 100)"), "5050");
    assert_eq!(s(&it, "(sum-to 100000)"), "5000050000");
}

#[test]
fn a_recur_from_an_interpreted_function_restarts_the_loop() {
    // The interpreter's `recur` is a signal that travels up to the innermost loop, whichever
    // evaluator runs it.
    let it = Interpreter::new();
    s(&it, "(defn again (i) (recur (+ i 1)))");
    def(&it, "upto3", r#"(proto :code (
        (push-scope) (const 0) (def i) (pop)
        (loop i)
        (load >=) (load i) (const 3) (call 2) (jump-if-false more)
        (load i) (jump end)
        (label more) (load again) (load i) (call 1)
        (label end) (end-loop) (pop-scope) (return)))"#);
    assert_eq!(s(&it, "(upto3)"), "3");
}

const GUARDED: &str = r#"(proto :params (thunk) :code (
    (try caught)
    (load thunk) (call 0)
    (end-try) (jump done)
    (label caught)
    (push-scope) (def e) (pop)
    (load list) (load dict-get) (load e) (const :kind) (call 2) (load dict-get) (load e) (const :message) (call 2) (call 2)
    (pop-scope)
    (label done) (return)))"#;

#[test]
fn try_catches_errors_from_any_depth() {
    let it = Interpreter::new();
    def(&it, "guarded", GUARDED);
    assert_eq!(s(&it, "(guarded (fn () 42))"), "42");
    assert_eq!(s(&it, r#"(guarded (fn () (error "boom")))"#), "(runtime boom)");
    assert_eq!(s(&it, "(guarded (fn () (/ 1 0)))"), "(division-by-zero Division by zero)");
    // the error comes from a compiled function two calls down
    def(&it, "fact", FACT);
    let e = s(&it, "(guarded (fn () (fact \"x\")))");
    assert!(e.starts_with("(type "), "{e}");
}

#[test]
fn the_error_dict_matches_the_interpreter() {
    let it = Interpreter::new();
    def(&it, "guarded", GUARDED);
    let interpreted = s(&it, r#"(try (nth '(1) 5) (catch e (list (dict-get e :kind) (dict-get e :message))))"#);
    let compiled = s(&it, "(guarded (fn () (nth '(1) 5)))");
    assert_eq!(compiled, interpreted);
}

#[test]
fn recur_passes_through_try() {
    let it = Interpreter::new();
    def(&it, "count", r#"(proto :code (
        (push-scope) (const 0) (def i) (pop)
        (loop i)
        (try caught)
        (load <) (load i) (const 5) (call 2) (jump-if-false out)
        (load +) (load i) (const 1) (call 2) (recur 1)
        (label out) (load i) (end-try) (jump end)
        (label caught) (pop) (const "caught the recur: wrong")
        (label end) (end-loop) (pop-scope) (return)))"#);
    assert_eq!(s(&it, "(count)"), "5");
}

#[test]
fn dict_literals() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "((vm-assemble '(proto :code ((const 1) (load +) (const 1) (const 1) (call 2) (dict :a :b) (return)))))"), "{:a 1 :b 2}");
}

#[test]
fn compiled_functions_work_wherever_functions_do() {
    let it = Interpreter::new();
    def(&it, "sq", "(proto :params (x) :code ((load *) (load x) (load x) (tail-call 2)))");
    assert_eq!(s(&it, "(map sq '(1 2 3))"), "(1 4 9)");
    assert_eq!(s(&it, "(reduce (vm-assemble '(proto :params (a b) :code ((load +) (load a) (load b) (tail-call 2)))) 0 '(1 2 3))"), "6");
    assert_eq!(s(&it, "(apply sq '(7))"), "49");
    assert_eq!(s(&it, "(fn? sq)"), "true");
    // and from the host
    let r = it.call("sq", &[Value::Number(9.0)]).unwrap().unwrap();
    assert_eq!(print_value(&r, false), "81");
}

#[test]
fn compiled_code_calls_interpreted_code_and_back() {
    let it = Interpreter::new();
    s(&it, "(defn twice (f x) (f (f x)))");
    def(&it, "inc2", "(proto :params (x) :code ((load twice) (load inc) (load x) (tail-call 2)))");
    assert_eq!(s(&it, "(inc2 5)"), "7");
}

#[test]
fn a_macro_must_be_expanded_by_the_compiler() {
    let it = Interpreter::new();
    let e = err(&it, "((vm-assemble '(proto :code ((load when) (const true) (const 1) (call 2) (return)))))");
    assert!(e.contains("macro when"), "{e}");
}

#[test]
fn bad_bytecode_is_refused_when_assembled() {
    let it = Interpreter::new();
    assert!(err(&it, "(vm-assemble '(proto :code ((frobnicate))))").contains("unknown instruction frobnicate"));
    assert!(err(&it, "(vm-assemble '(proto :code ((jump nowhere))))").contains("no label nowhere"));
    assert!(err(&it, "(vm-assemble '(proto :code ((const))))").contains("takes 1 argument"));
    assert!(err(&it, "(vm-assemble '(not-a-proto))").contains("expected a (proto"));
}
