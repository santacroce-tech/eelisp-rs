//! Register instructions (docs/plans/register-vm.md, step 1), written by hand through
//! `vm-assemble`, as `tests/vm.rs` does for the stack instructions. Each program is one the
//! compiler's register back end will emit.

use eelisp::printer::print_value;
use eelisp::Interpreter;

fn s(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).unwrap_or_else(|e| panic!("{src}\n→ {e}")), false)
}

fn err(it: &Interpreter, src: &str) -> String {
    match it.eval_str(src) {
        Ok(v) => panic!("expected an error, got {}", print_value(&v, false)),
        Err(e) => e.to_string(),
    }
}

fn def(it: &Interpreter, name: &str, proto: &str) {
    s(it, &format!("(def {name} (vm-assemble '{proto}))"));
}

/// `(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))` — each call's argument is
/// computed into the register its frame then starts at.
const FIB: &str = r#"(proto :name "fib" :params (n) :slots 4 :code (
    (call2 1 < (local 0 n) (const 2))
    (jump-if-not 1 more)
    (return-value (local 0 n))
    (label more)
    (call2 2 - (local 0 n) (const 1))
    (call-global 1 fib 2 1)
    (call2 3 - (local 0 n) (const 2))
    (call-global 2 fib 3 1)
    (call2 1 + 1 2)
    (return-value 1)))"#;

#[test]
fn recursion_with_arguments_in_place() {
    let it = Interpreter::new();
    def(&it, "fib", FIB);
    assert_eq!(s(&it, "(fib 20)"), "6765");
    assert_eq!(s(&it, "(map fib (range 0 10))"), "(0 1 1 2 3 5 8 13 21 34)");
}

#[test]
fn a_loop_of_moves_and_jumps() {
    let it = Interpreter::new();
    def(&it, "sum-to", r#"(proto :params (k) :slots 4 :code (
        (move 1 (const 0)) (move 2 (const 0))
        (label top)
        (call2 3 = 1 (local 0 k))
        (jump-if-not 3 go)
        (return-value 2)
        (label go)
        (call2 2 + 2 1)
        (call2 1 + 1 (const 1))
        (jump top)))"#);
    assert_eq!(s(&it, "(sum-to 100)"), "4950");
    assert_eq!(s(&it, "(sum-to 0)"), "0");
}

#[test]
fn tail_calls_through_registers_run_in_constant_space() {
    let it = Interpreter::new();
    def(&it, "down", r#"(proto :name "down" :params (n) :slots 2 :code (
        (call2 1 = (local 0 n) (const 0))
        (jump-if-not 1 go)
        (return-value (const "done"))
        (label go)
        (call2 1 - (local 0 n) (const 1))
        (tail-call-global down 1 1)))"#);
    assert_eq!(s(&it, "(down 1000000)"), "done");
}

#[test]
fn a_tail_call_to_a_builtin_returns_its_value() {
    let it = Interpreter::new();
    def(&it, "three", r#"(proto :params (a) :slots 4 :code (
        (move 1 (local 0 a)) (move 2 (const 2)) (move 3 (const 3))
        (tail-call-global list 1 3)))"#);
    assert_eq!(s(&it, "(three 1)"), "(1 2 3)");
}

#[test]
fn a_builtin_with_arguments_in_registers() {
    let it = Interpreter::new();
    def(&it, "three", r#"(proto :params (a) :slots 4 :code (
        (move 1 (local 0 a)) (move 2 (const 2)) (move 3 (const 3))
        (call-global 1 list 1 3)
        (return-value 1)))"#);
    assert_eq!(s(&it, "(three 1)"), "(1 2 3)");
    // a builtin that isn't arithmetic, through call2
    def(&it, "second", r#"(proto :params (xs) :slots 2 :code (
        (call2 1 nth (local 0 xs) (const 1)) (return-value 1)))"#);
    assert_eq!(s(&it, "(second (list 7 8 9))"), "8");
    // arithmetic on something that isn't a number goes through the builtin, and fails as the
    // interpreter's does
    def(&it, "add", r#"(proto :params (a b) :slots 3 :code (
        (call2 2 + (local 0 a) (local 1 b)) (return-value 2)))"#);
    let i = Interpreter::new();
    assert_eq!(err(&it, r#"(add "ab" 1)"#), err(&i, r#"(+ "ab" 1)"#));
}

#[test]
fn register_and_stack_code_mix() {
    let it = Interpreter::new();
    // a register instruction delivering to the stack, and a stack instruction's result into a
    // register
    def(&it, "inc", r#"(proto :params (a) :slots 1 :code (
        (call2 push + (local 0 a) (const 1)) (return)))"#);
    assert_eq!(s(&it, "(inc 41)"), "42");
    def(&it, "via", r#"(proto :params () :slots 2 :code (
        (load +) (const 2) (const 3) (call 2) (pop-to 1)
        (push 1) (push (const 10)) (pop-to 0)
        (call2 1 * 1 0)
        (return-value 1)))"#);
    assert_eq!(s(&it, "(via)"), "50");
    def(&it, "glob", r#"(proto :params () :slots 1 :code ((global 0 inc) (push 0) (const 1) (call 1) (return)))"#);
    assert_eq!(s(&it, "(glob)"), "2");
}

#[test]
fn every_kind_of_callee() {
    let it = Interpreter::new();
    def(&it, "fib", FIB);
    it.eval_str("(defn pair (a b) (list a b)) (defn all (. xs) xs)").unwrap();
    // an interpreted function, a compiled one, one with a rest parameter, one given too few
    // arguments, a stack-mode (named) proto
    def(&it, "named", r#"(proto :params (x) :code ((load list) (load x) (call 1) (return)))"#);
    def(&it, "go", r#"(proto :params () :slots 5 :code (
        (call2 0 pair (const 1) (const 2))
        (move 3 (const 10))
        (call-global 1 fib 3 1)
        (move 3 (const 4)) (move 4 (const 5))
        (call-global 2 all 3 2)
        (move 4 (const 7))
        (call-global 3 pair 4 1)
        (move 4 (const 8))
        (call-global 4 named 4 1)
        (push 0) (push 1) (push 2) (push 3) (push 4) (make-list item item item item item)
        (return)))"#);
    assert_eq!(s(&it, "(go)"), "((1 2) 55 (4 5) (7 nil) (8))");
}

#[test]
fn calling_what_is_not_a_function() {
    let it = Interpreter::new();
    it.eval_str("(def five 5)").unwrap();
    def(&it, "bad", r#"(proto :params () :slots 2 :code ((move 1 (const 1)) (call-global 0 five 1 1) (return-value 0)))"#);
    let i = Interpreter::new();
    i.eval_str("(def five 5)").unwrap();
    let interpreted = match i.eval_str("(five 1)") {
        Err(e) => e.to_string(),
        Ok(_) => panic!("calling a number worked"),
    };
    assert_eq!(err(&it, "(bad)"), interpreted);
    assert!(err(&it, "(do (vm-run '(proto :slots 1 :code ((global 0 no-such-thing) (return-value 0)))))").contains("no-such-thing"));
}

#[test]
fn an_unbound_local_means_the_global() {
    let it = Interpreter::new();
    it.eval_str("(def g 99)").unwrap();
    def(&it, "f", r#"(proto :params () :slots 2 :code ((return-value (local 1 g))))"#);
    assert_eq!(s(&it, "(f)"), "99");
    def(&it, "h", r#"(proto :params () :slots 2 :code ((const 5) (init-local 1) (return-value (local 1 g))))"#);
    assert_eq!(s(&it, "(h)"), "5");
}

#[test]
fn captured_variables_as_operands() {
    let it = Interpreter::new();
    def(&it, "adder", r#"(proto :params (x) :slots 1 :code (
        (closure (proto :params (y) :slots 2 :captures ((local 0)) :code (
            (call2 1 + (cap 0 x) (local 0 y)) (return-value 1))))
        (return)))"#);
    assert_eq!(s(&it, "((adder 10) 5)"), "15");
    // a captured slot is read through its cell, as a local too
    def(&it, "both", r#"(proto :params (x) :slots 2 :code (
        (closure (proto :params () :slots 0 :captures ((local 0)) :code ((return-value (cap 0 x)))))
        (pop)
        (call2 1 + (local 0 x) (const 1))
        (return-value 1)))"#);
    assert_eq!(s(&it, "(both 1)"), "2");
}

#[test]
fn an_error_in_a_callee_lands_in_the_callers_try_with_its_registers_back() {
    let it = Interpreter::new();
    def(&it, "boom", r#"(proto :params (x) :slots 1 :code ((global 0 no-such-thing) (return-value 0)))"#);
    // boom's frame starts at register 1, so the caller's registers above it are gone while it
    // runs; the handler writes register 3 after the error
    def(&it, "safe", r#"(proto :params () :slots 4 :code (
        (try caught)
        (move 1 (const 1))
        (call-global 2 boom 1 1)
        (end-try)
        (return-value 2)
        (label caught)
        (pop)
        (move 3 (const "caught"))
        (return-value 3)))"#);
    assert_eq!(s(&it, "(safe)"), "caught");
    // and after a normal return too
    def(&it, "id", r#"(proto :params (x) :slots 1 :code ((return-value (local 0 x))))"#);
    def(&it, "after", r#"(proto :params () :slots 4 :code (
        (move 1 (const 1))
        (call-global 0 id 1 1)
        (move 3 (const 2))
        (call2 0 + 0 3)
        (return-value 0)))"#);
    assert_eq!(s(&it, "(after)"), "3");
}

#[test]
fn deep_register_recursion_is_an_error_not_a_crash() {
    let it = Interpreter::new();
    def(&it, "deep", r#"(proto :name "deep" :params (n) :slots 2 :code (
        (call2 1 + (local 0 n) (const 1))
        (call-global 1 deep 1 1)
        (return-value 1)))"#);
    assert!(err(&it, "(deep 0)").contains("stack overflow"));
    def(&it, "fib", FIB);
    assert_eq!(s(&it, "(fib 10)"), "55");
}

#[test]
fn malformed_register_code_is_refused() {
    let it = Interpreter::new();
    for (code, what) in [
        ("(proto :slots 2 :code ((move 2 (const 1))))", "out of range"),
        ("(proto :slots 2 :code ((call-global 0 f 1 2)))", "out of range"),
        ("(proto :slots 2 :code ((move 0 (frob 1))))", "an operand is"),
        ("(proto :code ((move 0 (const 1))))", "needs a proto with :slots"),
        ("(proto :slots 1 :code ((push (cap 0 x))))", "capture 0 is out of range"),
        ("(proto :slots 1 :code ((call2 0 + 0)))", "takes 4"),
    ] {
        let e = err(&it, &format!("(vm-assemble '{code})"));
        assert!(e.contains(what), "{code}: {e}");
    }
}

#[test]
fn labels_count_the_wider_instructions() {
    // call2 and call-global take two instructions, tail-call-global three: a label after them
    // must land after all of it
    let it = Interpreter::new();
    def(&it, "f", r#"(proto :params (a) :slots 3 :code (
        (jump start)
        (label back)
        (call2 1 + (local 0 a) (const 1))
        (move 2 1)
        (call-global 1 list 2 1)
        (return-value 1)
        (label start)
        (jump back)))"#);
    assert_eq!(s(&it, "(f 1)"), "(2)");
}

#[test]
fn nth_and_dict_get_inside_the_instruction_mean_what_the_builtins_mean() {
    let it = Interpreter::new();
    let i = Interpreter::new();
    def(&it, "rnth", r#"(proto :params (l k) :slots 3 :code ((call2 2 nth (local 0 l) (local 1 k)) (return-value 2)))"#);
    def(&it, "rget", r#"(proto :params (d k) :slots 3 :code ((call2 2 dict-get (local 0 d) (local 1 k)) (return-value 2)))"#);
    let cases = [
        ("rnth", "nth", "(list 1 2 3)", "1"),
        ("rnth", "nth", "(list 1 2 3)", "7"),
        ("rnth", "nth", "(list 1 2 3)", "-1"),
        ("rnth", "nth", "(list 1 2 3)", "1.7"),
        ("rnth", "nth", "(list)", "0"),
        ("rget", "dict-get", r#"{"a" 1 :b 2}"#, r#""a""#),
        ("rget", "dict-get", r#"{"a" 1 :b 2}"#, ":b"),
        ("rget", "dict-get", r#"{"a" 1 :b 2}"#, "'b"),
        ("rget", "dict-get", r#"{"a" 1 :b 2}"#, r#""zz""#),
    ];
    for (r, b, x, y) in cases {
        assert_eq!(s(&it, &format!("({r} {x} {y})")), s(&i, &format!("({b} {x} {y})")), "({b} {x} {y})");
    }
    // what they don't cover goes to the builtin, and fails as it does
    for (r, b, x, y) in [
        ("rnth", "nth", r#""abc""#, "1"),
        ("rnth", "nth", "(list 1 2)", r#""x""#),
        ("rget", "dict-get", "(list 1)", r#""a""#),
        ("rget", "dict-get", r#"{"a" 1}"#, "5"),
    ] {
        assert_eq!(err(&it, &format!("({r} {x} {y})")), err(&i, &format!("({b} {x} {y})")), "({b} {x} {y})");
    }
}

#[test]
fn a_redefined_nth_is_called_not_the_intrinsic() {
    let it = Interpreter::new();
    def(&it, "rnth", r#"(proto :params (l k) :slots 3 :code ((call2 2 nth (local 0 l) (local 1 k)) (return-value 2)))"#);
    assert_eq!(s(&it, "(rnth (list 1 2 3) 0)"), "1");
    it.eval_str("(defn nth (l k) :mine)").unwrap();
    assert_eq!(s(&it, "(rnth (list 1 2 3) 0)"), ":mine");
}

#[test]
fn operands_in_cells_or_unbound_take_the_general_path() {
    let it = Interpreter::new();
    it.eval_str("(def xs (list 10 20 30))").unwrap();
    // slot 1 unbound: the global xs; slot 0 captured by a closure: a cell
    def(&it, "f", r#"(proto :params (k) :slots 3 :code (
        (closure (proto :params () :slots 0 :captures ((local 0)) :code ((return-value (cap 0 k)))))
        (pop)
        (call2 2 nth (local 1 xs) (local 0 k))
        (return-value 2)))"#);
    assert_eq!(s(&it, "(f 2)"), "30");
}

#[test]
fn call_at_calls_whatever_its_register_holds() {
    let it = Interpreter::new();
    it.eval_str("(defn pair (a b) (list a b))").unwrap();
    def(&it, "fib", FIB);
    // a function passed in, a global read first (the stack's order: the function, then the
    // arguments), a compiled function entered in place
    def(&it, "apply2", r#"(proto :params (f x) :slots 2 :temps 4 :code (
        (move (t 0) (local 0 f)) (move (t 1) (local 1 x)) (move (t 2) (const 2))
        (call-at (t 0) (t 0) 2)
        (global (t 1) fib) (move (t 2) (const 10)) (call-at (t 1) (t 1) 1)
        (call2 (t 0) list (t 0) (t 1))
        (return-value (t 0))))"#);
    assert_eq!(s(&it, "(apply2 pair 1)"), "((1 2) 55)");
    assert_eq!(s(&it, "(apply2 + 1)"), "(3 55)");
    assert_eq!(s(&it, "(apply2 (fn (a b) (* a b)) 4)"), "(8 55)");
    let i = Interpreter::new();
    assert_eq!(err(&it, "(apply2 5 1)"), err(&i, "(5 1 2)"));
    // an undefined function fails when it's read, before any argument is computed
    def(&it, "early", r#"(proto :params () :slots 0 :temps 2 :code (
        (global (t 0) no-such-function) (call2 (t 1) / (const 1) (const 0)) (call-at (t 0) (t 0) 1)
        (return-value (t 0))))"#);
    assert!(err(&it, "(early)").contains("no-such-function"));
}

#[test]
fn tail_call_at_runs_in_constant_space() {
    let it = Interpreter::new();
    def(&it, "down", r#"(proto :name "down" :params (n) :slots 1 :temps 2 :code (
        (call2 (t 0) = (local 0 n) (const 0))
        (jump-if-not (t 0) go)
        (return-value (const "done"))
        (label go)
        (global (t 0) down)
        (call2 (t 1) - (local 0 n) (const 1))
        (tail-call-at (t 0) 1)))"#);
    assert_eq!(s(&it, "(down 1000000)"), "done");
    let e = err(&it, "(vm-assemble '(proto :slots 2 :code ((call-at 0 1 1))))");
    assert!(e.contains("out of range"), "{e}");
}

#[test]
fn equality_inside_the_instruction_means_what_the_builtins_mean() {
    let it = Interpreter::new();
    let i = Interpreter::new();
    def(&it, "req", r#"(proto :params (a b) :slots 3 :code ((call2 2 = (local 0 a) (local 1 b)) (return-value 2)))"#);
    def(&it, "rne", r#"(proto :params (a b) :slots 3 :code ((call2 2 != (local 0 a) (local 1 b)) (return-value 2)))"#);
    let pairs = [
        (r#""a""#, r#""a""#),
        (r#""a""#, r#""b""#),
        ("'x", "'x"),
        ("'x", ":x"),
        (r#""x""#, "'x"),
        ("(list 1 2)", "(list 1 2)"),
        ("(list 1 2)", "(list 1 3)"),
        ("{:a 1}", "{:a 1}"),
        ("nil", "nil"),
        ("nil", "false"),
        ("1", r#""1""#),
        ("true", "true"),
        ("1", "1.0"),
    ];
    for (a, b) in pairs {
        assert_eq!(s(&it, &format!("(req {a} {b})")), s(&i, &format!("(= {a} {b})")), "(= {a} {b})");
        assert_eq!(s(&it, &format!("(rne {a} {b})")), s(&i, &format!("(!= {a} {b})")), "(!= {a} {b})");
    }
}

#[test]
fn one_argument_intrinsics_mean_what_the_builtins_mean() {
    let it = Interpreter::new();
    let i = Interpreter::new();
    let values = [
        "(list 1 2)", "(list)", r#""héllo""#, r#""""#, "{:a 1}", "{}", "nil", "false", "true", "0", "'x", ":k",
    ];
    for f in ["head", "length", "empty?", "nil?", "list?", "symbol?", "not"] {
        def(&it, "r", &format!("(proto :params (a) :slots 2 :code ((call1 1 {f} (local 0 a)) (return-value 1)))"));
        for v in values {
            let (call, builtin) = (format!("(r {v})"), format!("({f} {v})"));
            match (it.eval_str(&call), i.eval_str(&builtin)) {
                (Ok(a), Ok(b)) => assert_eq!(print_value(&a, false), print_value(&b, false), "{builtin}"),
                (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{builtin}"),
                (a, b) => panic!("{builtin}: {:?} against {:?}", a.map(|v| print_value(&v, false)), b.map(|v| print_value(&v, false))),
            }
        }
    }
    // to the stack, and a function that isn't a builtin
    it.eval_str("(defn twice (x) (* 2 x))").unwrap();
    def(&it, "g", "(proto :params (a) :slots 1 :code ((call1 push twice (local 0 a)) (return)))");
    assert_eq!(s(&it, "(g 21)"), "42");
}
