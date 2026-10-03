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
    it.editor.borrow_mut().current_dir = Some(Box::new(|| "/notes".to_string()));

    assert_eq!(s(&it, "(buffer-text)"), "the quick brown fox");
    assert_eq!(s(&it, "(current-file)"), "/notes/todo.md");
    assert_eq!(s(&it, "(current-dir)"), "/notes");
    // a script can process the buffer
    assert_eq!(s(&it, "(length (str-split (buffer-text) \" \"))"), "4");
}

/// Headless — no host, no filesystem. The reads answer with an empty string rather than blowing
/// up, so a snippet written for the editor still runs in the CLI.
#[test]
fn editor_reads_are_inert_without_a_host() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "(current-dir)"), "");
    assert_eq!(s(&it, "(current-file)"), "");
}

/// `notes` / `read-note` read the workspace on disk: every note, by workspace-relative path,
/// without hidden folders, and never a file outside the workspace.
#[test]
fn notes_are_read_from_the_workspace() {
    let dir = std::env::temp_dir().join(format!("eelisp-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["reviews", ".eeditor", "assets"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join("inbox.md"), "# Inbox\n- [ ] call Ana").unwrap();
    std::fs::write(dir.join("reviews/week.md"), "done").unwrap();
    std::fs::write(dir.join("reviews/Notes.MARKDOWN"), "x").unwrap();
    std::fs::write(dir.join(".eeditor/hidden.md"), "no").unwrap();
    std::fs::write(dir.join("assets/a.png"), [0u8, 1, 2]).unwrap();
    std::fs::write(dir.join("Budget.eesheet"), "not a note").unwrap();

    let it = Interpreter::new();
    let root = dir.display().to_string();
    it.editor.borrow_mut().current_dir = Some(Box::new(move || root.clone()));

    assert_eq!(s(&it, "(notes)"), "(inbox.md reviews/Notes.MARKDOWN reviews/week.md)");
    assert_eq!(s(&it, "(notes \"reviews\")"), "(reviews/Notes.MARKDOWN reviews/week.md)");
    assert_eq!(s(&it, "(notes \"nowhere\")"), "()");
    assert_eq!(s(&it, "(read-note \"inbox.md\")"), "# Inbox\n- [ ] call Ana");
    assert_eq!(s(&it, "(read-note \"reviews/../inbox.md\")"), "# Inbox\n- [ ] call Ana");
    assert_eq!(s(&it, "(read-note \"missing.md\")"), "nil");
    // every note, swept in one expression
    assert_eq!(s(&it, "(length (filter (fn (p) (str-contains (read-note p) \"[ ]\")) (notes)))"), "1");

    for bad in ["\"../secret.md\"", "\"reviews/../../x.md\"", "\"/etc/hosts\""] {
        let e = it.eval_str(&format!("(read-note {bad})")).unwrap_err().to_string();
        assert!(e.contains("workspace"), "{bad}: {e}");
    }
    assert!(it.eval_str("(read-note \"assets/a.png\")").is_ok(), "bytes that are UTF-8 read as text");
    let _ = std::fs::remove_dir_all(&dir);
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
