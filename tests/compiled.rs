//! Compiled mode (roadmap step 4): `Interpreter::set_compiled` makes the engine compile each
//! top-level form and run it on the VM. The rest of the test suite runs in that mode too, with
//! `EELISP_COMPILED=1 cargo test` (CI does both); these tests check the mode itself.

use eelisp::printer::print_value;
use eelisp::{Interpreter, Value};

fn global(it: &Interpreter, name: &str) -> Value {
    eelisp::env::get(&it.global, name).unwrap()
}

fn is_bytecode(v: &Value) -> bool {
    matches!(v, Value::Function(f) if f.code.is_some())
}

fn ev(it: &Interpreter, src: &str) -> String {
    print_value(
        &it.eval_str(src).unwrap_or_else(|e| panic!("{src}\n→ {e}")),
        false,
    )
}

fn compiled() -> Interpreter {
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    it
}

#[test]
fn definitions_become_bytecode() {
    let it = compiled();
    assert!(it.is_compiled());
    ev(&it, "(defn fact (n) (if (<= n 1) 1 (* n (fact (- n 1)))))");
    assert!(is_bytecode(&global(&it, "fact")));
    assert_eq!(ev(&it, "(fact 20)"), "2432902008176640000");
}

#[test]
fn the_prelude_is_recompiled() {
    let it = compiled();
    assert!(
        is_bytecode(&global(&it, "take")),
        "prelude functions run on the VM"
    );
    assert!(
        matches!(global(&it, "when"), Value::Macro(m) if m.code.is_some()),
        "and its macros"
    );
    assert_eq!(ev(&it, "(take 2 '(1 2 3))"), "(1 2)");
}

#[test]
fn the_interpreter_is_the_default_and_the_switch_goes_both_ways() {
    // (unless the whole suite runs with EELISP_COMPILED=1)
    let it = Interpreter::new();
    it.set_compiled(false).unwrap();
    ev(&it, "(defn a () 1)");
    assert!(!is_bytecode(&global(&it, "a")));
    it.set_compiled(true).unwrap();
    ev(&it, "(defn b () 2)");
    assert!(is_bytecode(&global(&it, "b")));
    it.set_compiled(false).unwrap();
    ev(&it, "(defn c () 3)");
    assert!(!is_bytecode(&global(&it, "c")));
    assert_eq!(ev(&it, "(list (a) (b) (c))"), "(1 2 3)");
}

#[test]
fn a_program_defining_compile_does_not_break_the_engine() {
    let it = compiled();
    ev(&it, "(defn compile (x) :mine)");
    assert_eq!(ev(&it, "(compile 1)"), ":mine");
    ev(&it, "(defn after () 42)");
    assert!(is_bytecode(&global(&it, "after")));
    assert_eq!(ev(&it, "(after)"), "42");
}

#[test]
fn a_program_defining_read_top_does_not_break_the_engine() {
    // the engine holds the reader's function, not its name, as it does the compiler's
    let it = compiled();
    ev(&it, "(defn read-top (src) :mine)");
    assert_eq!(ev(&it, "(read-top \"x\")"), ":mine");
    assert_eq!(ev(&it, "(+ 1 2) (* 2 3)"), "6");
}

#[test]
fn compiled_mode_reads_with_the_eelisp_reader() {
    let it = compiled();
    // what the engine reads with is what `read-top` makes of the text
    assert_eq!(
        ev(&it, "(read-top \";; doc\\n(def a 1) ; mine\")"),
        "(((def a 1) (def a 1) ; mine ;; doc))"
    );
    // a reading error is a parse error, with the Rust reader's message
    let e = it.eval_str("(defn broken (x)").unwrap_err();
    assert_eq!(
        (e.kind(), e.to_string().as_str()),
        ("parse", "Parse error: unclosed list")
    );
    let e = it.eval_str("(a ] \"oops").unwrap_err();
    assert_eq!(e.to_string(), "Parse error: unterminated string");
    // and the stray closer at the top level is skipped, as before
    assert_eq!(ev(&it, ") (+ 1 2)"), "3");
}

#[test]
fn the_compilers_internals_stay_out_of_listings() {
    let it = compiled();
    assert_eq!(ev(&it, "(length (function-list \"compiler-\"))"), "0");
    assert_eq!(ev(&it, "(length (function-list \"reader-\"))"), "0");
    assert_ne!(ev(&it, "(length (function-list \"take\"))"), "0");
}

#[test]
fn source_and_errors_work_as_interpreted() {
    let it = compiled();
    ev(&it, ";; Twice x.\n(defn twice (x) (* 2 x))  ; doubles\n");
    let src = ev(&it, "(source-text 'twice)");
    assert!(
        src.contains("Twice x.") && src.contains("(defn twice (x) (* 2 x))  ; doubles"),
        "{src}"
    );
    let e = it.eval_str("(twice \"a\")").unwrap_err().to_string();
    assert!(e.contains("Type mismatch"), "{e}");
    assert_eq!(
        ev(&it, "(try (twice \"a\") (catch e (dict-get e :kind)))"),
        "type"
    );
}

#[test]
fn host_calls_run_compiled_functions() {
    let it = compiled();
    ev(&it, "(defn add (a b) (+ a b))");
    let r = it
        .call("add", &[Value::Number(2.0), Value::Number(3.0)])
        .unwrap()
        .unwrap();
    assert_eq!(print_value(&r, false), "5");
    assert!(it.eval_host("(add 1 1)").contains("\"result\":2"));
}

#[test]
fn the_environment_variable_turns_it_on() {
    // Read when an engine is made; this run's value decides what we expect.
    let on = std::env::var("EELISP_COMPILED").is_ok_and(|v| v == "1" || v == "true");
    assert_eq!(Interpreter::new().is_compiled(), on);
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
#[test]
fn an_engine_handle_can_start_compiled() {
    // How a host such as eeditor opts in: in spawn_with's setup, on the engine's own thread.
    use eelisp::server::EngineHandle;
    let engine =
        EngineHandle::spawn_with(":memory:".to_string(), |it| it.set_compiled(true).unwrap());
    assert!(engine.eval("(defn sq (x) (* x x))").contains("\"ok\":true"));
    assert!(engine.eval("(sq 12)").contains("\"result\":144"));
}

/// `Load` caches where it found a global (src/vm.rs, `load_global`). Every way a global can change
/// must still be seen: redefined, `set!`, defined again, and the global scope growing until its
/// map moves its entries.
#[test]
fn a_cached_global_sees_every_change() {
    let it = compiled();
    ev(&it, "(defn helper (x) (* x 2)) (defn use-it (x) (helper x)) (def k 10) (defn get-k () k)");
    assert_eq!(ev(&it, "(use-it 5)"), "10");
    assert_eq!(ev(&it, "(get-k)"), "10");
    ev(&it, "(defn helper (x) (* x 3))");
    assert_eq!(ev(&it, "(use-it 5)"), "15");
    ev(&it, "(set! k 11)");
    assert_eq!(ev(&it, "(get-k)"), "11");
    ev(&it, "(def k 12)");
    assert_eq!(ev(&it, "(get-k)"), "12");
    // thousands of new globals: the map grows, and every entry moves
    for i in 0..2000 {
        ev(&it, &format!("(def filler-{i} {i})"));
    }
    assert_eq!(ev(&it, "(use-it 5)"), "15");
    assert_eq!(ev(&it, "(get-k)"), "12");
    ev(&it, "(defn helper (x) (+ x 1))");
    assert_eq!(ev(&it, "(use-it 5)"), "6");
    assert_eq!(ev(&it, "filler-1999"), "1999");
}

#[test]
fn two_engines_keep_their_own_globals() {
    let (a, b) = (compiled(), compiled());
    ev(&a, "(def who :a) (defn ask () who)");
    ev(&b, "(def who :b) (defn ask () who)");
    for _ in 0..3 {
        assert_eq!(ev(&a, "(ask)"), ":a");
        assert_eq!(ev(&b, "(ask)"), ":b");
        ev(&b, "(def extra-name 1)"); // b's global scope grows between a's calls
    }
}

/// `(+ a b)` and friends are computed inline on two numbers (`FastOp`), but only while the name
/// still means the language's own builtin.
#[test]
fn arithmetic_fast_path_follows_redefinition() {
    let it = compiled();
    ev(&it, "(defn mul2 (a b) (* a b)) (defn lt (a b) (< a b))");
    assert_eq!(ev(&it, "(mul2 3 4)"), "12");
    assert_eq!(ev(&it, "(lt 1 2)"), "true");
    assert_eq!(ev(&it, "(lt 1 nan)"), "false");
    // not two numbers: the builtin itself runs, with its own error
    let e = it.eval_str("(mul2 3 \"x\")").unwrap_err();
    assert_eq!(e.kind(), "type");
    // redefined: the program's own `*`, no fast path
    ev(&it, "(def * (fn (a b) 42))");
    assert_eq!(ev(&it, "(mul2 3 4)"), "42");
}

/// Texts read before are taken from the read cache. Reading is all it remembers: the forms are
/// compiled and run again each time, so definitions and macros made since are seen.
#[test]
fn a_text_read_again_is_still_compiled_and_run_again() {
    let it = compiled();
    let src = "(def counter (+ counter 1)) (m)";
    ev(&it, "(def counter 0) (defmacro m () 1)");
    assert_eq!(ev(&it, src), "1");
    assert_eq!(ev(&it, "counter"), "1");
    ev(&it, "(defmacro m () 2)");
    assert_eq!(ev(&it, src), "2", "the macro redefined since is used");
    assert_eq!(ev(&it, "counter"), "2", "the cached forms ran again");
    // `(source f)` still finds the text and the comments of a definition read from the cache
    let def = ";; doubles\n(defn twice (x) (* 2 x))";
    ev(&it, def);
    ev(&it, def);
    assert_eq!(ev(&it, "(source twice)"), ev(&Interpreter::new(), &format!("{def} (source twice)")));
}

#[test]
fn a_text_that_fails_to_read_fails_every_time() {
    let it = compiled();
    for _ in 0..2 {
        let err = it.eval_str("(+ 1").unwrap_err();
        assert!(matches!(err, eelisp::LispError::Parse(_)), "{err:?}");
    }
}
