//! Faithful port of the Swift EELisp language-core tests
//! (eeditor/eelisp/Tests/EELispTests/EELispTests.swift — Lexer/Parser/Evaluator/Prelude/Printer).
//!
//! Database + agenda tests live in `tests/pending_db_agenda.rs` (ignored until those layers
//! are ported — roadmap step 2/3).
//!
//! Where this rewrite intentionally diverges from the Swift original (clean break, ANALYSIS §6),
//! the affected case is moved into `clean_break_*` tests that assert the NEW behavior.

use eelisp::lexer::{lex, Token};
use eelisp::parser::parse;
use eelisp::printer::print_value;
use eelisp::{Interpreter, Value};

fn s(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).expect("eval ok"), false)
}
fn one(src: &str) -> Value {
    parse(src).expect("parse ok").into_iter().next().expect("one form")
}

// ───────────────────────────── Lexer ─────────────────────────────

#[test]
fn lexer_basic() {
    assert_eq!(
        lex("(+ 1 2)").unwrap(),
        vec![
            Token::LParen,
            Token::Sym("+".into()),
            Token::Num(1.0),
            Token::Num(2.0),
            Token::RParen
        ]
    );
}

#[test]
fn lexer_strings_keywords_comments() {
    assert_eq!(lex("\"hello world\"").unwrap(), vec![Token::Str("hello world".into())]);
    assert_eq!(lex(":name :age").unwrap(), vec![Token::Kw("name".into()), Token::Kw("age".into())]);
    assert_eq!(lex("; comment\n42").unwrap(), vec![Token::Num(42.0)]);
}

#[test]
fn lexer_booleans_are_symbols_until_parsed() {
    // DIVERGENCE: the Swift lexer emits a `.bool` token; here true/false/nil are plain symbols
    // and resolve to Bool/Null in the PARSER (see `parser_resolves_literals`).
    assert_eq!(
        lex("true false nil").unwrap(),
        vec![Token::Sym("true".into()), Token::Sym("false".into()), Token::Sym("nil".into())]
    );
}

#[test]
fn lexer_negative_numbers() {
    assert_eq!(lex("-42 -3.14").unwrap(), vec![Token::Num(-42.0), Token::Num(-3.14)]);
    // matches Swift: -1 stays a negative literal in these positions
    assert_eq!(
        lex("(def x -1)").unwrap(),
        vec![Token::LParen, Token::Sym("def".into()), Token::Sym("x".into()), Token::Num(-1.0), Token::RParen]
    );
    assert_eq!(
        lex("(+ -1 2)").unwrap(),
        vec![Token::LParen, Token::Sym("+".into()), Token::Num(-1.0), Token::Num(2.0), Token::RParen]
    );
}

#[test]
fn clean_break_no_operator_number_splitting() {
    // DIVERGENCE (ANALYSIS §6): the Swift lexer split `(+1 3)` -> `(+ 1 3)` and treated `-`
    // after `(` as the minus operator. This rewrite does neither: an atom is read whole and
    // parsed, so `+1` and `-1` are numbers. Write `(- 1 3)` / `(+ 1 3)` explicitly.
    assert_eq!(
        lex("(+1 3)").unwrap(),
        vec![Token::LParen, Token::Num(1.0), Token::Num(3.0), Token::RParen]
    );
    assert_eq!(
        lex("(-1 3)").unwrap(),
        vec![Token::LParen, Token::Num(-1.0), Token::Num(3.0), Token::RParen]
    );
}

// ───────────────────────────── Parser ─────────────────────────────

#[test]
fn parser_nested_lists() {
    assert_eq!(
        one("(+ (* 2 3) 4)"),
        Value::List(std::rc::Rc::new(vec![
            Value::Symbol("+".into()),
            Value::List(std::rc::Rc::new(vec![
                Value::Symbol("*".into()),
                Value::Number(2.0),
                Value::Number(3.0),
            ])),
            Value::Number(4.0),
        ]))
    );
}

#[test]
fn parser_quote_shorthand() {
    assert_eq!(
        one("'(1 2 3)"),
        Value::List(std::rc::Rc::new(vec![
            Value::Symbol("quote".into()),
            Value::List(std::rc::Rc::new(vec![
                Value::Number(1.0),
                Value::Number(2.0),
                Value::Number(3.0),
            ])),
        ]))
    );
}

#[test]
fn parser_dict_literal() {
    match one("{:name \"Alice\" :age 30}") {
        Value::Dict(d) => {
            assert_eq!(d.keys.len(), 2);
            assert_eq!(d.get("name"), Some(&Value::Str("Alice".into())));
            assert_eq!(d.get("age"), Some(&Value::Number(30.0)));
        }
        other => panic!("expected dict, got {}", other),
    }
}

#[test]
fn parser_empty_list_and_literals() {
    assert_eq!(one("()"), Value::List(std::rc::Rc::new(vec![])));
    assert_eq!(one("true"), Value::Bool(true));
    assert_eq!(one("false"), Value::Bool(false));
    assert_eq!(one("nil"), Value::Null);
}

// ───────────────────────────── Evaluator ─────────────────────────────

#[test]
fn eval_arithmetic() {
    let i = Interpreter::new();
    assert_eq!(s(&i, "(+ 1 2 3)"), "6");
    assert_eq!(s(&i, "(- 10 3)"), "7");
    assert_eq!(s(&i, "(* 2 3 4)"), "24");
    assert_eq!(s(&i, "(/ 10 2)"), "5");
    assert_eq!(s(&i, "(mod 7 3)"), "1");
    assert_eq!(s(&i, "(+ -1 2)"), "1"); // negative literal in arg position
}

#[test]
fn clean_break_operator_number_splitting_now_errors() {
    // Swift returned (+1 3)=4, (-1 3)=-2, (*2 3)=6. Here these are NOT split, so they error:
    // (+1 3) -> call 1 as a fn; (*2 3) -> undefined symbol `*2`.
    let i = Interpreter::new();
    assert!(i.eval_str("(+1 3)").is_err());
    assert!(i.eval_str("(-1 3)").is_err());
    assert!(i.eval_str("(*2 3)").is_err());
    // the explicit forms work
    assert_eq!(s(&i, "(- 1 3)"), "-2");
}

#[test]
fn eval_comparisons() {
    let i = Interpreter::new();
    assert_eq!(s(&i, "(= 1 1)"), "true");
    assert_eq!(s(&i, "(= 1 2)"), "false");
    assert_eq!(s(&i, "(< 1 2)"), "true");
    assert_eq!(s(&i, "(> 2 1)"), "true");
    assert_eq!(s(&i, "(!= 1 2)"), "true");
}

#[test]
fn eval_strings() {
    let i = Interpreter::new();
    assert_eq!(s(&i, "(str \"hello\" \" \" \"world\")"), "hello world");
    assert_eq!(s(&i, "(str-len \"hello\")"), "5");
    assert_eq!(s(&i, "(str-upper \"hello\")"), "HELLO");
    assert_eq!(s(&i, "(str-contains \"hello world\" \"world\")"), "true");
}

#[test]
fn eval_def_defn_fn() {
    let i = Interpreter::new();
    i.eval_str("(def x 42)").unwrap();
    assert_eq!(s(&i, "x"), "42");
    i.eval_str("(defn square (x) (* x x))").unwrap();
    assert_eq!(s(&i, "(square 5)"), "25");
    assert_eq!(s(&i, "((fn (x) (* x x)) 4)"), "16");
}

#[test]
fn eval_if_cond_let_do() {
    let i = Interpreter::new();
    assert_eq!(s(&i, "(if true 1 2)"), "1");
    assert_eq!(s(&i, "(if false 1 2)"), "2");
    assert_eq!(s(&i, "(if nil 1 2)"), "2");
    assert_eq!(
        s(&i, "(cond ((= 1 2) \"nope\") ((= 1 1) \"yes\") (else \"default\"))"),
        "yes"
    );
    assert_eq!(s(&i, "(let ((x 1) (y 2)) (+ x y))"), "3");
    assert_eq!(s(&i, "(do 1 2 3)"), "3");
}

#[test]
fn eval_list_ops() {
    let i = Interpreter::new();
    assert_eq!(s(&i, "(head '(1 2 3))"), "1");
    assert_eq!(s(&i, "(tail '(1 2 3))"), "(2 3)");
    assert_eq!(s(&i, "(length '(1 2 3))"), "3");
    assert_eq!(s(&i, "(cons 0 '(1 2))"), "(0 1 2)");
    assert_eq!(s(&i, "(reverse '(1 2 3))"), "(3 2 1)");
}

#[test]
fn eval_map_filter_reduce_range() {
    let i = Interpreter::new();
    i.eval_str("(defn double (x) (* x 2))").unwrap();
    assert_eq!(s(&i, "(map double '(1 2 3))"), "(2 4 6)");
    assert_eq!(s(&i, "(filter even? (range 6))"), "(0 2 4)");
    assert_eq!(s(&i, "(reduce + 0 '(1 2 3 4))"), "10");
    assert_eq!(s(&i, "(range 5)"), "(0 1 2 3 4)");
}

#[test]
fn eval_dicts() {
    let i = Interpreter::new();
    i.eval_str("(def d {:name \"Alice\" :age 30})").unwrap();
    assert_eq!(s(&i, "(dict-get d :name)"), "Alice");
    assert_eq!(s(&i, "(dict-get d :age)"), "30");
    assert_eq!(s(&i, "(dict-has d :name)"), "true");
    assert_eq!(s(&i, "(dict-has d :email)"), "false");
}

#[test]
fn eval_rest_params_closures_recursion() {
    let i = Interpreter::new();
    i.eval_str("(defn my-list (. items) items)").unwrap();
    assert_eq!(s(&i, "(my-list 1 2 3)"), "(1 2 3)");

    i.eval_str("(defn make-adder (n) (fn (x) (+ x n)))").unwrap();
    i.eval_str("(def add5 (make-adder 5))").unwrap();
    assert_eq!(s(&i, "(add5 10)"), "15");

    i.eval_str("(defn factorial (n) (if (<= n 1) 1 (* n (factorial (- n 1)))))").unwrap();
    assert_eq!(s(&i, "(factorial 5)"), "120");
}

#[test]
fn eval_for_each_and_set() {
    let i = Interpreter::new();
    i.eval_str("(def total 0)").unwrap();
    i.eval_str("(for-each x '(1 2 3 4 5) (set! total (+ total x)))").unwrap();
    assert_eq!(s(&i, "total"), "15");
}

#[test]
fn eval_type_predicates_and_logic() {
    let i = Interpreter::new();
    assert_eq!(s(&i, "(number? 42)"), "true");
    assert_eq!(s(&i, "(string? \"hi\")"), "true");
    assert_eq!(s(&i, "(list? '(1 2))"), "true");
    assert_eq!(s(&i, "(nil? nil)"), "true");
    assert_eq!(s(&i, "(and true true)"), "true");
    assert_eq!(s(&i, "(and true false)"), "false");
    assert_eq!(s(&i, "(or false true)"), "true");
    assert_eq!(s(&i, "(or false false)"), "false");
}

// ───────────────────────────── Prelude ─────────────────────────────

#[test]
fn prelude_functions() {
    let i = Interpreter::new();
    assert_eq!(s(&i, "(inc 41)"), "42");
    assert_eq!(s(&i, "(dec 43)"), "42");
    assert_eq!(s(&i, "(even? 4)"), "true");
    assert_eq!(s(&i, "(odd? 3)"), "true");
    assert_eq!(s(&i, "(first '(a b c))"), "a");
    assert_eq!(s(&i, "(last '(1 2 3))"), "3");
}

// ───────────────────────────── Printer ─────────────────────────────

#[test]
fn printer_formats_values() {
    assert_eq!(print_value(&Value::Number(42.0), true), "42");
    assert_eq!(print_value(&Value::Number(3.14), true), "3.14");
    assert_eq!(print_value(&Value::Str("hello".into()), true), "\"hello\"");
    assert_eq!(print_value(&Value::Bool(true), true), "true");
    assert_eq!(print_value(&Value::Null, true), "nil");
    assert_eq!(print_value(&Value::Keyword("name".into()), true), ":name");
    assert_eq!(
        print_value(&Value::List(std::rc::Rc::new(vec![Value::Number(1.0), Value::Number(2.0)])), true),
        "(1 2)"
    );
}

// ───────────────────────────── Rewrite guarantees ─────────────────────────────

#[test]
fn rewrite_tail_call_optimization() {
    // Would overflow the native stack without TCO (Swift had none — ANALYSIS §6).
    let i = Interpreter::new();
    i.eval_str("(defn countdown (n) (if (= n 0) \"done\" (countdown (- n 1))))").unwrap();
    assert_eq!(s(&i, "(countdown 500000)"), "done");
}

#[test]
fn rewrite_macro_rest_params_and_quasiquote() {
    // Swift's defmacro ignored `. rest`, and quasiquote was unimplemented — both fixed here.
    let i = Interpreter::new();
    i.eval_str("(defmacro my-when (test . body) `(if ,test (do ,@body) nil))").unwrap();
    assert_eq!(s(&i, "(my-when true 1 2 3)"), "3");
    assert_eq!(s(&i, "(my-when false 1)"), "nil");
    // prelude's when/unless (which use the same machinery)
    assert_eq!(s(&i, "(when true 41 42)"), "42");
    assert_eq!(s(&i, "(unless false 99)"), "99");
}
