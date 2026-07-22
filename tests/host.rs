//! Step 4 — the host boundary (ANALYSIS §5): structured view values, the JSON serialization
//! bridge (`eval_host`/`eval_json`), output capture, the editor RPC, and json-parse/stringify.

use std::cell::RefCell;
use std::rc::Rc;

use eelisp::printer::print_value;
use eelisp::Interpreter;

fn s(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).expect("eval ok"), false)
}

// ── structured view values ──

#[test]
fn browse_returns_table_view() {
    let it = Interpreter::new();
    it.eval_str("(deftable t (x:number))").unwrap();
    it.eval_str("(insert t {:x 1})").unwrap();
    assert_eq!(eelisp::value::type_name(&it.eval_str("(browse t)").unwrap()), "table-view");
    assert_eq!(eelisp::value::type_name(&it.eval_str("(edit t)").unwrap()), "form-view");
    assert_eq!(
        eelisp::value::type_name(&it.eval_str("(defform f (a:number) :computed ((b (* a 2))))").unwrap()),
        "form-view"
    );
}

// ── JSON serialization bridge ──

#[test]
fn eval_host_scalar_envelope() {
    let it = Interpreter::new();
    it.set_echo(false);
    let out = it.eval_host("(+ 1 2)");
    assert!(out.contains("\"ok\":true"));
    assert!(out.contains("\"result\":3"));

    let err = it.eval_host("(this-is-undefined)");
    assert!(err.contains("\"ok\":false"));
    assert!(err.contains("Undefined symbol"));
}

#[test]
fn eval_json_tags_structured_values() {
    let it = Interpreter::new();
    it.eval_str("(deftable c (name:string))").unwrap();
    it.eval_str("(insert c {:name \"Alice\"})").unwrap();

    let tv = it.eval_json("(browse c)").unwrap();
    assert!(tv.contains("$tableView"));
    assert!(tv.contains("Alice"));
    assert!(tv.contains("tableDef"));

    let fv = it.eval_json("(defform k (p:number) :computed ((d (* p 2))))").unwrap();
    assert!(fv.contains("$formView"));
    assert!(fv.contains("\"isStandalone\":true"));
    assert!(fv.contains("computedFields"));

    // a dict keeps insertion order via ordered pairs
    let d = it.eval_json("(dict :b 2 :a 1)").unwrap();
    assert!(d.contains("$dict"));
    assert!(d.find("\"b\"").unwrap() < d.find("\"a\"").unwrap());
}

// ── output capture ──

#[test]
fn output_is_captured() {
    let it = Interpreter::new();
    it.set_echo(false);
    it.eval_str("(println \"line one\") (print \"no-newline\")").unwrap();
    assert_eq!(it.take_output(), "line one\nno-newline");

    // eval_host bundles captured output into the envelope
    let out = it.eval_host("(println \"hello host\")");
    assert!(out.contains("hello host"));
}

// ── editor RPC ──

#[test]
fn editor_read_callbacks() {
    let it = Interpreter::new();
    it.editor.borrow_mut().buffer_text = Some(Box::new(|| "the quick brown fox".to_string()));
    it.editor.borrow_mut().current_file = Some(Box::new(|| "/notes/todo.md".to_string()));

    assert_eq!(s(&it, "(buffer-text)"), "the quick brown fox");
    assert_eq!(s(&it, "(current-file)"), "/notes/todo.md");
    // a script can process the buffer
    assert_eq!(s(&it, "(length (str-split (buffer-text) \" \"))"), "4");
}

#[test]
fn editor_mutation_callback() {
    let it = Interpreter::new();
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let log2 = log.clone();
    it.editor.borrow_mut().insert_at = Some(Box::new(move |pos, text| {
        log2.borrow_mut().push(format!("insert@{}:{}", pos, text));
    }));
    it.eval_str("(insert-at 5 \"hi\")").unwrap();
    assert_eq!(log.borrow().as_slice(), &["insert@5:hi".to_string()]);
}

// ── json builtins ──

#[test]
fn json_parse_and_stringify() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "(dict-get (json-parse \"{\\\"a\\\": 1}\") :a)"), "1");
    assert_eq!(s(&it, "(nth (json-parse \"[10, 20, 30]\") 1)"), "20");
    // objects → dict with sorted keys, and round-trips
    assert_eq!(s(&it, "(json-stringify {:name \"Bob\" :age 25})"), "{\"age\":25,\"name\":\"Bob\"}");
}
