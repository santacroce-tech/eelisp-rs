//! Small gaps found by writing the Zero to Hero book (eeditor-next docs/ROADMAP.md item 16), each
//! pinned here so it stays closed.

use std::path::PathBuf;

use eelisp::printer::print_value;
use eelisp::Interpreter;

fn ev(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).unwrap_or_else(|e| panic!("{src}\n  → {e}")), true)
}
fn err(it: &Interpreter, src: &str) -> String {
    it.eval_str(src).map(|v| panic!("{src} should fail, gave {}", print_value(&v, true))).unwrap_err().to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eelisp-gaps-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

#[test]
fn a_person_is_a_capitalised_name_after_the_verb() {
    let it = Interpreter::new();
    assert_eq!(ev(&it, r#"(dict-get (smart-parse "call the bank tomorrow") :who)"#), "()");
    assert_eq!(ev(&it, r#"(dict-get (smart-parse "call Marta at the bank") :who)"#), r#"("Marta")"#);
    assert_eq!(ev(&it, r#"(dict-get (smart-parse "Call Marta") :who)"#), r#"("Marta")"#, "the verb in any case");
    assert_eq!(ev(&it, r#"(dict-get (smart-parse "lunch with Ana") :who)"#), r#"("Ana")"#);
}

#[test]
fn choices_are_enforced_by_insert_and_update() {
    let it = Interpreter::new();
    ev(&it, r#"(deftable h (symbol:string (kind :type choice :choices ("crypto" "stock"))))"#);
    ev(&it, r#"(insert h {:symbol "btc" :kind "crypto"})"#);
    ev(&it, r#"(insert h {:symbol "x"})"#); // nil is fine — :required is for that
    let e = err(&it, r#"(insert h {:symbol "gld" :kind "gold"})"#);
    assert!(e.contains("h.kind must be one of crypto, stock") && e.contains("gold"), "{e}");
    assert!(err(&it, r#"(update h 1 {:kind "bond"})"#).contains("must be one of"));
    assert_eq!(ev(&it, "(count-records h)"), "2");
}

#[test]
fn select_takes_a_list_or_text() {
    let it = Interpreter::new();
    ev(&it, r#"(deftable t (a:string b:number c:string))"#);
    ev(&it, r#"(insert t {:a "x" :b 1 :c "y"})"#);
    let as_list = ev(&it, "(record->dict (first (records (query t :select '(a b)))))");
    let as_text = ev(&it, r#"(record->dict (first (records (query t :select "a, b"))))"#);
    assert_eq!(as_list, as_text);
    assert!(!as_text.contains(":c"), "{as_text}");
}

#[test]
fn a_record_becomes_a_dict_and_json() {
    let it = Interpreter::new();
    ev(&it, r#"(deftable t (a:string n:number))"#);
    ev(&it, r#"(insert t {:a "x" :n 2})"#);
    assert_eq!(ev(&it, "(record->dict (first (records (query t))))"), r#"{:id 1 :a "x" :n 2}"#);
    assert_eq!(ev(&it, "(json-stringify (map record->dict (records (query t))))"), r#""[{\"a\":\"x\",\"id\":1,\"n\":2}]""#);
    assert!(err(&it, "(record->dict 5)").contains("a record"));
    // a table named by a value: a call in the table's place is evaluated
    assert_eq!(ev(&it, r#"(let (name "t") (count-records (str name)))"#), "1");
}

#[test]
fn export_agenda_makes_its_folder() {
    let dir = scratch("export");
    let it = Interpreter::new();
    ev(&it, r#"(add-item "x" :when "2026-11-02")"#);
    let path = dir.join("backup").join("2026-10-26").join("agenda.json");
    ev(&it, &format!(r#"(export-agenda "memory" :format :json :path "{}")"#, path.display()));
    assert!(path.exists());
}

#[test]
fn sheet_clear_empties_an_area_and_recalculates() {
    let dir = scratch("clear");
    let it = Interpreter::new();
    let root = dir.display().to_string();
    it.editor.borrow_mut().current_dir = Some(Box::new(move || root.clone()));
    ev(&it, r#"(sheet-new "S")"#);
    ev(&it, r#"(sheet-put "S" "A1" '((1 2) (3 4) (5 6)))"#);
    ev(&it, r#"(sheet-set "S" "D1" "=(sum A:B)")"#);
    assert_eq!(ev(&it, r#"(sheet-get "S" "D1")"#), "21");
    ev(&it, r#"(sheet-clear "S" "A2:B3")"#);
    assert_eq!(ev(&it, r#"(sheet-rows "S" "A1:B3")"#), "((1 2) (nil nil) (nil nil))");
    assert_eq!(ev(&it, r#"(sheet-get "S" "D1")"#), "3", "what read the cleared cells is recalculated");
    ev(&it, r#"(sheet-clear "S" "A:B")"#);
    assert_eq!(ev(&it, r#"(length (sheet-rows "S" "A:A"))"#), "0");
}
