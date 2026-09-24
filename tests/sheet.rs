//! Sheets: a grid whose formulas are EELisp, each sheet a SQLite file. Addressing, what a cell
//! holds, recalculation order, cycles, errors, persistence — and that opening a sheet runs nothing.

use std::path::PathBuf;

use eelisp::printer::print_value;
use eelisp::sheet_ref::*;
use eelisp::Interpreter;

/// A fresh folder, unique to one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eelisp-sheet-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// An interpreter whose workspace is `dir`, so sheet names resolve there.
fn engine(dir: &PathBuf) -> Interpreter {
    let it = Interpreter::new();
    let root = dir.display().to_string();
    it.editor.borrow_mut().current_dir = Some(Box::new(move || root.clone()));
    it
}

fn ev(it: &Interpreter, src: &str) -> String {
    match it.eval_str(src) {
        Ok(v) => print_value(&v, true),
        Err(e) => panic!("{src}\n  → {e}"),
    }
}

fn err(it: &Interpreter, src: &str) -> String {
    match it.eval_str(src) {
        Ok(v) => panic!("{src} should fail, gave {}", print_value(&v, true)),
        Err(e) => e.to_string(),
    }
}

/// A new sheet called Budget in a fresh workspace.
fn budget(name: &str) -> (PathBuf, Interpreter) {
    let dir = scratch(name);
    let it = engine(&dir);
    ev(&it, r#"(sheet-new "Budget")"#);
    (dir, it)
}

/// `sheet-set` on a given interpreter's Budget — for tests that make more than one.
fn sheet_set(it: &Interpreter, cell: &str, input: &str) -> String {
    set(it, cell, input)
}

fn set(it: &Interpreter, cell: &str, input: &str) -> String {
    let escaped = input.replace('\\', "\\\\").replace('"', "\\\"");
    ev(it, &format!(r#"(sheet-set "Budget" "{cell}" "{escaped}")"#))
}

fn get(it: &Interpreter, cell: &str) -> String {
    ev(it, &format!(r#"(sheet-get "Budget" "{cell}")"#))
}

fn get_err(it: &Interpreter, cell: &str) -> String {
    err(it, &format!(r#"(sheet-get "Budget" "{cell}")"#))
}

// ── addressing ───────────────────────────────────────────────────────

#[test]
fn column_letters_round_trip() {
    for (i, name) in [(0, "A"), (25, "Z"), (26, "AA"), (701, "ZZ"), (702, "AAA"), (MAX_COLS - 1, "ZZZ")] {
        assert_eq!(col_name(i), name);
        assert_eq!(col_index(name), Some(i));
    }
    assert_eq!(col_index("a"), None, "formulas are uppercase");
    assert_eq!(col_index("AAAA"), None);
}

#[test]
fn a_symbol_is_a_reference_only_when_it_is_shaped_like_one() {
    assert_eq!(classify("C3"), Some(RefSym::Cell(CellRef::new(2, 2))));
    assert_eq!(classify("$C$3"), Some(RefSym::Cell(CellRef::new(2, 2))));
    assert_eq!(
        classify("B9:A1"),
        Some(RefSym::Range(Range::new(CellRef::new(0, 0), CellRef::new(8, 1)))),
        "corners in either order"
    );
    assert_eq!(
        classify("Other!A1"),
        Some(RefSym::Other { sheet: "Other".into(), at: Box::new(RefSym::Cell(CellRef::new(0, 0))) })
    );
    assert!(matches!(classify("money/Q3!A1:B2"), Some(RefSym::Other { .. })));
    assert_eq!(classify("!A1"), None);
    assert_eq!(classify("#REF!"), Some(RefSym::Deleted));
    for not_a_ref in ["c3", "A0", "A01", "x1", "total", "A1:", "3A"] {
        assert_eq!(classify(not_a_ref), None, "{not_a_ref}");
    }
    assert_eq!(parse_area(" c3 ").map(|r| r.a1()), Some("C3".into()), "hosts may be casual");
}

// ── making and opening sheets ────────────────────────────────────────

#[test]
fn a_sheet_is_a_file_in_the_workspace() {
    let (dir, it) = budget("file");
    assert!(dir.join("Budget.eesheet").is_file());
    assert!(err(&it, r#"(sheet-new "Budget")"#).contains("already exists"));
    // a sheet in a folder, named with or without its extension
    std::fs::create_dir(dir.join("money")).unwrap();
    ev(&it, r#"(sheet-new "money/Q3.eesheet")"#);
    assert!(dir.join("money/Q3.eesheet").is_file());
}

#[test]
fn opening_a_missing_sheet_is_an_error_and_makes_no_file() {
    let dir = scratch("missing");
    let it = engine(&dir);
    assert!(err(&it, r#"(sheet-open "Nope")"#).contains("no sheet at"));
    assert!(!dir.join("Nope.eesheet").exists());
}

#[test]
fn a_file_that_is_not_a_sheet_is_refused() {
    let dir = scratch("not-a-sheet");
    std::fs::write(dir.join("Notes.eesheet"), "just some text\n").unwrap();
    let it = engine(&dir);
    assert!(err(&it, r#"(sheet-open "Notes")"#).contains("is not a sheet"));
    // an SQLite database that isn't one of ours
    rusqlite::Connection::open(dir.join("Other.eesheet")).unwrap().execute_batch("CREATE TABLE t (x)").unwrap();
    assert!(err(&it, r#"(sheet-open "Other")"#).contains("is not a sheet"));
}

// ── what a cell holds ────────────────────────────────────────────────

#[test]
fn input_decides_the_value() {
    let (_, it) = budget("typing");
    for (input, value) in [
        ("1200", "1200"),
        ("-3.5", "-3.5"),
        (" 42 ", "42"),
        ("1e3", "1000"),
        ("'42", "\"42\""),
        ("rent", "\"rent\""),
        ("1,200", "1200"), // a thousand two hundred — see a_number_can_be_typed_the_way_it_is_written
        ("inf", "\"inf\""),
        ("true", "\"true\""),
        ("", "nil"),
    ] {
        set(&it, "A1", input);
        assert_eq!(get(&it, "A1"), value, "input {input:?}");
    }
}

#[test]
fn a_formula_reads_cells_and_ranges() {
    let (_, it) = budget("formula");
    set(&it, "B1", "1200");
    set(&it, "B2", "450");
    set(&it, "C3", "=(sum B1:B2)");
    set(&it, "C4", "=(/ C3 2)");
    set(&it, "C5", r#"=(if (> C3 1500) "over" "ok")"#);
    assert_eq!(get(&it, "C3"), "1650");
    assert_eq!(get(&it, "C4"), "825");
    assert_eq!(get(&it, "C5"), "\"over\"");

    // a range is a flat list, blanks included
    set(&it, "D1", "=B1:B3");
    assert_eq!(get(&it, "D1"), "(1200 450 nil)");
    // anchored and plain spellings of one cell both work
    set(&it, "D2", "=(+ $B$1 B1)");
    assert_eq!(get(&it, "D2"), "2400");
    // any function in scope
    ev(&it, "(defn tax (x) (* x 0.25))");
    set(&it, "D3", "=(tax B1)");
    assert_eq!(get(&it, "D3"), "300");
}

#[test]
fn changing_a_cell_recalculates_what_depends_on_it_and_returns_it() {
    let (_, it) = budget("dependents");
    set(&it, "B1", "1200");
    set(&it, "B2", "450");
    set(&it, "C3", "=(sum B1:B2)");
    set(&it, "C4", "=(* C3 2)");
    let changed = set(&it, "B1", "1000");
    // rows are (row col input value error fmt), row-major
    assert_eq!(changed, r#"((0 1 "1000" 1000 nil nil) (2 2 "=(sum B1:B2)" 1450 nil nil) (3 2 "=(* C3 2)" 2900 nil nil))"#);
}

#[test]
fn a_diamond_recalculates_each_formula_once_in_order() {
    let (_, it) = budget("diamond");
    ev(&it, "(def runs 0)");
    set(&it, "A1", "1");
    set(&it, "B1", "=(+ A1 1)");
    set(&it, "C1", "=(* A1 10)");
    set(&it, "D1", "=(do (set! runs (+ runs 1)) (+ B1 C1))");
    ev(&it, "(set! runs 0)");
    set(&it, "A1", "2");
    assert_eq!(get(&it, "D1"), "23", "sees both B1 and C1 already updated");
    assert_eq!(ev(&it, "runs"), "1");
}

#[test]
fn a_cycle_is_an_error_on_every_cell_in_it_and_breaking_it_recovers() {
    let (_, it) = budget("cycle");
    set(&it, "A1", "=(+ B1 1)");
    set(&it, "B1", "=(+ A1 1)");
    set(&it, "C1", "=(* A1 2)");
    assert!(get_err(&it, "A1").contains("circular reference between A1, B1"));
    assert!(get_err(&it, "B1").contains("circular reference"));
    assert!(get_err(&it, "C1").contains("A1 has an error — circular reference"), "{}", get_err(&it, "C1"));

    set(&it, "B1", "5");
    assert_eq!(get(&it, "A1"), "6");
    assert_eq!(get(&it, "C1"), "12");

    // a formula that reads itself, directly or through a range
    set(&it, "D1", "=(+ D1 1)");
    assert!(get_err(&it, "D1").contains("circular reference between D1"));
    set(&it, "E5", "=(sum E1:E9)");
    assert!(get_err(&it, "E5").contains("circular reference"));
}

#[test]
fn a_long_chain_recalculates_without_exhausting_the_stack() {
    let (_, it) = budget("chain");
    // typed in one block, the way a paste arrives: one recalculation, one transaction
    let block: Vec<String> = (0..5000).map(|i| if i == 0 { "(\"1\")".into() } else { format!("(\"=(+ A{} 1)\")", i) }).collect();
    ev(&it, &format!(r#"(sheet-set "Budget" "A1" '({}))"#, block.join(" ")));
    assert_eq!(get(&it, "A5000"), "5000");
    set(&it, "A1", "10");
    assert_eq!(get(&it, "A5000"), "5009");
}

#[test]
fn an_error_propagates_naming_where_it_started() {
    let (_, it) = budget("errors");
    set(&it, "A1", "=(no-such-function)");
    set(&it, "B1", "=(+ A1 1)");
    set(&it, "C1", "=(+ B1 1)");
    set(&it, "D1", "=(sum A1:C1)");
    assert!(get_err(&it, "A1").contains("Undefined symbol: no-such-function"));
    for cell in ["B1", "C1", "D1"] {
        assert!(get_err(&it, cell).contains("A1 has an error — Undefined symbol"), "{cell}: {}", get_err(&it, cell));
    }
    set(&it, "A1", "1");
    assert_eq!(get(&it, "C1"), "3");
}

#[test]
fn formulas_that_do_not_read_as_one_expression_say_so() {
    let (_, it) = budget("bad-formulas");
    set(&it, "A1", "=");
    assert!(get_err(&it, "A1").contains("empty formula"));
    set(&it, "A2", "=(+ 1 2) (+ 3 4)");
    assert!(get_err(&it, "A2").contains("wrap several in (do"));
    set(&it, "A3", "=(+ 1");
    assert!(get_err(&it, "A3").contains("Parse error"));
    set(&it, "A4", "=(+ Nowhere!A1 1)");
    assert!(get_err(&it, "A4").contains("no sheet at"), "{}", get_err(&it, "A4"));
    set(&it, "A5", "=(pow 10 400)");
    assert!(get_err(&it, "A5").contains("not a finite number"));
}

#[test]
fn sheet_rows_reads_an_area_as_rows() {
    let (_, it) = budget("rows");
    ev(&it, r#"(sheet-put "Budget" "A1" '(("rent" 1200) ("food" 450)))"#);
    assert_eq!(ev(&it, r#"(sheet-rows "Budget" "a1:b2")"#), r#"(("rent" 1200) ("food" 450))"#);
    assert!(err(&it, r#"(sheet-get "Budget" "A1:B2")"#).contains("use (sheet-rows"));
    set(&it, "C1", "=(oops)");
    assert!(err(&it, r#"(sheet-rows "Budget" "A1:C2")"#).contains("C1: Undefined symbol"));
}

#[test]
fn sheet_put_keeps_values_as_values() {
    let (_, it) = budget("put");
    ev(&it, r#"(sheet-put "Budget" "B2" '("=not a formula" "42" 42 nil true))"#);
    assert_eq!(get(&it, "B2"), "\"=not a formula\"");
    assert_eq!(get(&it, "C2"), "\"42\"", "text that looks like a number stays text");
    assert_eq!(get(&it, "D2"), "42");
    assert_eq!(get(&it, "E2"), "nil");
    assert_eq!(get(&it, "F2"), "true");

    ev(&it, "(deftable people (name:string age:number))");
    ev(&it, r#"(insert people {:name "Ada" :age 36})"#);
    ev(&it, r#"(insert people {:name "Alan" :age 41})"#);
    ev(&it, r#"(sheet-put "Budget" "A5" (query people :order "name"))"#);
    assert_eq!(ev(&it, r#"(sheet-rows "Budget" "A5:B7")"#), r#"(("name" "age") ("Ada" 36) ("Alan" 41))"#);
    // and a formula can read the database directly
    set(&it, "D5", "=(count-records people)");
    assert_eq!(get(&it, "D5"), "2");
}

#[test]
fn a_formula_can_read_sheets_but_not_change_them() {
    let (_, it) = budget("reentry");
    ev(&it, r#"(sheet-new "Rates")"#);
    ev(&it, r#"(sheet-set "Rates" "A1" "0.2")"#);
    set(&it, "B1", "100");
    set(&it, "B2", r#"=(* B1 (sheet-get "Rates" "A1"))"#);
    assert_eq!(get(&it, "B2"), "20");
    // reading its own sheet works too
    set(&it, "B3", r#"=(sheet-get "Budget" "B1")"#);
    assert_eq!(get(&it, "B3"), "100");

    set(&it, "B4", r#"=(sheet-set "Rates" "A1" "0.9")"#);
    assert!(get_err(&it, "B4").contains("a formula can read sheets but not change them"));
    assert_eq!(ev(&it, r#"(sheet-get "Rates" "A1")"#), "0.2");
    // and the refusal didn't leave the engine thinking a recalculation is still running
    ev(&it, r#"(sheet-set "Rates" "A1" "0.3")"#);
    ev(&it, r#"(sheet-recalc "Budget")"#);
    assert_eq!(get(&it, "B2"), "30");
}

// ── persistence ──────────────────────────────────────────────────────

#[test]
fn everything_survives_a_reopen_and_opening_runs_no_formula() {
    let (dir, it) = budget("reopen");
    ev(&it, "(def runs 0)");
    set(&it, "A1", "rent");
    set(&it, "B1", "1200");
    set(&it, "B2", "=(do (set! runs (+ runs 1)) (* B1 2))");
    set(&it, "B3", r#"=(list 1 "two" {:k 3})"#);
    set(&it, "B4", "=(query people)");
    set(&it, "B5", "=(boom)");
    ev(&it, r#"(sheet-format "Budget" "B1:B2" {:num "currency" :dp 2})"#);
    ev(&it, r#"(sheet-col-width "Budget" "A" 160)"#);
    assert_eq!(ev(&it, "runs"), "1");
    ev(&it, r#"(sheet-close "Budget")"#);

    // a different engine entirely — nothing in memory survives, only the file
    let other = engine(&dir);
    ev(&other, "(def runs 0)");
    let payload = ev(&other, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#"(1 1 "=(do (set! runs (+ runs 1)) (* B1 2))" 2400 nil {:dp 2 :num "currency"})"#), "{payload}");
    assert!(payload.contains(":widths ((0 160))"), "{payload}");
    assert_eq!(ev(&other, r#"(sheet-get "Budget" "B3")"#), r#"(1 "two" {:k 3})"#, "data stays data");
    assert!(err(&other, r#"(sheet-get "Budget" "B5")"#).contains("Undefined symbol: boom"), "errors are stored too");
    assert_eq!(ev(&other, "runs"), "0", "opening and reading ran nothing");

    ev(&other, r#"(sheet-recalc "Budget")"#);
    assert_eq!(ev(&other, "runs"), "1", "recalculating does");
}

#[test]
fn another_connections_write_is_picked_up() {
    let (dir, first) = budget("two-engines");
    set(&first, "A1", "1");
    let second = engine(&dir);
    ev(&second, r#"(sheet-set "Budget" "A1" "2")"#);
    assert_eq!(get(&first, "A1"), "2");
}

#[test]
fn the_version_moves_on_writes_and_only_on_writes() {
    let (_, it) = budget("version");
    let v = |it: &Interpreter| ev(it, r#"(sheet-version "Budget")"#).parse::<i64>().unwrap();
    let start = v(&it);
    set(&it, "A1", "1");
    assert!(v(&it) > start);
    let after_write = v(&it);
    get(&it, "A1");
    ev(&it, r#"(sheet-open "Budget")"#);
    ev(&it, r#"(sheet-recalc "Budget")"#); // no formulas: nothing changed
    assert_eq!(v(&it), after_write);
}

#[test]
fn formats_merge_and_clear() {
    let (_, it) = budget("formats");
    ev(&it, r#"(sheet-format "Budget" "A1" {:bold true :num "number"})"#);
    ev(&it, r#"(sheet-format "Budget" "A1" {:num nil :align "right"})"#);
    let payload = ev(&it, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#"(0 0 "" nil nil {:align "right" :bold true})"#), "{payload}");
    ev(&it, r#"(sheet-format "Budget" "A1" nil)"#);
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(":cells ()"), "a blank, unformatted cell is gone");
    assert!(err(&it, r#"(sheet-format "Budget" "A1" {:bold '(1)})"#).contains("must be a string, number, bool or nil"));

    // a block sets each cell's format exactly — how undo puts back formats that differed
    ev(&it, r#"(sheet-format "Budget" "A1:B1" {:italic true})"#);
    ev(&it, r#"(sheet-format "Budget" "A1" '(({:bold true} nil)))"#);
    let payload = ev(&it, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#"(0 0 "" nil nil {:bold true})"#), "replaced, not merged: {payload}");
    assert!(!payload.contains("(0 1 "), "nil cleared B1: {payload}");
    assert!(err(&it, r#"(sheet-format "Budget" "A1" '((1)))"#).contains("a dict or nil"));
}

// ── inserting and deleting rows and columns ──────────────────────────

#[test]
fn inserting_rows_moves_cells_and_rewrites_formulas_keeping_their_text() {
    let (_, it) = budget("insert-rows");
    set(&it, "A1", "1");
    set(&it, "A2", "2");
    set(&it, "A3", "=(sum A1:A2) ; the total");
    set(&it, "B1", "=(*  $A$2   10)");
    set(&it, "C1", r#"=(str "A2 stays in a string" A2)"#);
    ev(&it, r#"(sheet-insert-rows "Budget" 2)"#);

    assert_eq!(get(&it, "A3"), "2");
    let payload = ev(&it, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#""=(sum A1:A3) ; the total""#), "{payload}");
    assert!(payload.contains(r#""=(*  $A$3   10)""#), "spacing and anchors kept: {payload}");
    assert!(payload.contains(r#""=(str \"A2 stays in a string\" A3)""#), "{payload}");
    assert_eq!(get(&it, "A4"), "3");

    // the new row is inside the range, so a formula that counts cells sees it
    set(&it, "D1", "=(length A1:A3)");
    assert_eq!(get(&it, "D1"), "3");
    ev(&it, r#"(sheet-insert-rows "Budget" 2 2)"#);
    assert_eq!(get(&it, "D1"), "5", "recalculated because its range grew");
}

#[test]
fn deleting_rows_leaves_ref_errors_and_shrinks_ranges() {
    let (_, it) = budget("delete-rows");
    ev(&it, r#"(sheet-put "Budget" "A1" '((1) (2) (3) (4)))"#);
    set(&it, "B6", "=(* A2 10)");
    set(&it, "B7", "=(sum A1:A4)");
    set(&it, "B8", "=(+ A4 0)");
    ev(&it, r#"(sheet-delete-rows "Budget" 2)"#);
    // row 2 is gone, so everything below moves up one
    assert!(get_err(&it, "B5").contains("#REF!"), "it read the deleted A2");
    assert_eq!(get(&it, "A3"), "4");
    assert_eq!(get(&it, "B6"), "8", "(sum A1:A3) after the shrink: 1 + 3 + 4");
    assert_eq!(get(&it, "B7"), "4", "A4 moved to A3 and the formula followed");
    let payload = ev(&it, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#""=(* #REF! 10)""#) && payload.contains(r#""=(sum A1:A3)""#), "{payload}");
}

#[test]
fn deleting_the_row_a_formula_reads_leaves_ref() {
    let (_, it) = budget("ref-error");
    set(&it, "A1", "5");
    set(&it, "A3", "=(* A1 2)");
    ev(&it, r#"(sheet-delete-rows "Budget" 1)"#);
    let e = get_err(&it, "A2");
    assert!(e.contains("#REF!"), "{e}");
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(r#""=(* #REF! 2)""#));
}

#[test]
fn columns_move_with_their_widths() {
    let (_, it) = budget("cols");
    set(&it, "A1", "1");
    set(&it, "B1", "2");
    set(&it, "C1", "=(+ A1 B1)");
    ev(&it, r#"(sheet-col-width "Budget" "B" 200)"#);
    ev(&it, r#"(sheet-insert-cols "Budget" "B")"#);
    assert_eq!(get(&it, "D1"), "3");
    let payload = ev(&it, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#""=(+ A1 C1)""#), "{payload}");
    assert!(payload.contains(":widths ((2 200))"), "{payload}");
    ev(&it, r#"(sheet-delete-cols "Budget" "A")"#);
    assert!(get_err(&it, "C1").contains("#REF!"));
}

#[test]
fn an_insert_that_would_push_cells_off_the_sheet_is_refused() {
    let (_, it) = budget("no-room");
    set(&it, "A1048576", "last");
    assert!(err(&it, r#"(sheet-insert-rows "Budget" 1)"#).contains("no room"));
    assert_eq!(get(&it, "A1048576"), "\"last\"");
}

#[test]
fn rewrite_formula_moves_only_references() {
    let edit = Edit::Delete { axis: Axis::Rows, at: 1, n: 2 }; // rows 2 and 3
    let rw = rewrite_formula("(sum A1:A9 B2:B3 C4) ; A2 in a comment", &edit);
    assert_eq!(rw.text, "(sum A1:A7 #REF! C2) ; A2 in a comment");
    assert!(rw.resized && rw.changed);
    let moved = rewrite_formula("(+ A5 1)", &Edit::Insert { axis: Axis::Rows, at: 0, n: 1 });
    assert_eq!(moved.text, "(+ A6 1)");
    assert!(moved.changed && !moved.resized, "a reference that moved with its cell reads the same value");
    let other = rewrite_formula("(+ Other!A5 1)", &Edit::Insert { axis: Axis::Rows, at: 0, n: 1 });
    assert!(!other.changed, "another sheet's cells don't move");
}

// ── numbers as people write them ─────────────────────────────────────

#[test]
fn a_number_can_be_typed_the_way_it_is_written() {
    let (_, it) = budget("typing-numbers");
    // value, and the format the spelling asked for
    for (input, value, fmt) in [
        ("1200", "1200", ""),
        ("-3.5", "-3.5", ""),
        ("1e3", "1000", ""),
        ("50%", "0.5", r#"{:num "percent"}"#),
        ("-50%", "-0.5", r#"{:num "percent"}"#),
        ("$1,200", "1200", r#"{:cur "USD" :num "currency"}"#),
        ("R$ 1.200,50", "1200.5", r#"{:cur "BRL" :num "currency"}"#),
        ("5 €", "5", r#"{:cur "EUR" :num "currency"}"#),
        ("1,200", "1200", r#"{:num "number"}"#),
        ("1,200.50", "1200.5", r#"{:num "number"}"#),
        ("1.200,50", "1200.5", r#"{:num "number"}"#),
        ("1 200,50", "1200.5", r#"{:num "number"}"#),
        ("1,5", "1.5", ""),
        ("1.200", "1.2", ""),
    ] {
        let (_, sheet) = budget("n");
        sheet_set(&sheet, "A1", input);
        assert_eq!(get(&sheet, "A1"), value, "value of {input:?}");
        let payload = ev(&sheet, r#"(sheet-open "Budget")"#);
        if fmt.is_empty() {
            assert!(payload.contains(&format!(r#""{input}" {value} nil nil)"#)), "{input:?} asked for no format: {payload}");
        } else {
            assert!(payload.contains(fmt), "{input:?} should ask for {fmt}: {payload}");
        }
    }
    // what isn't a number stays text
    for input in ["1,2,3", "$", "%", "12 monkeys", "1..2", "--5"] {
        set(&it, "B1", input);
        assert_eq!(get(&it, "B1"), format!("{input:?}"), "{input:?} is text");
    }
}

#[test]
fn a_format_already_on_the_cell_wins() {
    let (_, it) = budget("typing-format");
    ev(&it, r#"(sheet-format "Budget" "A1" {:num "currency" :cur "EUR"})"#);
    set(&it, "A1", "50%");
    assert_eq!(get(&it, "A1"), "0.5", "the value still reads as a percentage");
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(r#"{:cur "EUR" :num "currency"}"#), "the cell keeps its own format");
}

#[test]
fn an_iso_date_is_a_date_and_other_spellings_are_text() {
    let (_, it) = budget("dates");
    set(&it, "A1", "2026-09-16");
    assert_eq!(get(&it, "A1"), "\"2026-09-16\"");
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(r#"{:date "iso"}"#), "it asked to be shown as a date");
    // the value is the language's own date, so date arithmetic works on it
    set(&it, "A2", "=(date-add A1 1 :months)");
    assert_eq!(get(&it, "A2"), "\"2026-10-16\"");
    set(&it, "A3", "=(date-diff \"2026-09-30\" A1)");
    assert_eq!(get(&it, "A3"), "14"); // (date-diff a b) counts the days from b to a

    for text in ["16/09/2026", "2026-13-01", "2026-02-30", "2026-9-16", "16 Sep 2026"] {
        set(&it, "B1", text);
        assert_eq!(get(&it, "B1"), format!("{text:?}"), "{text:?} stays text");
    }
    // a time after the date is still a date
    set(&it, "C1", "2026-09-16 14:30");
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(r#""2026-09-16 14:30" "2026-09-16 14:30" nil {:date "iso"}"#));
}

#[test]
fn a_row_can_be_given_a_height() {
    let (dir, it) = budget("heights");
    ev(&it, r#"(sheet-row-height "Budget" 3 48)"#);
    ev(&it, r#"(sheet-col-width "Budget" "B" 200)"#);
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(":heights ((2 48))"));

    // heights follow their rows through an insert, and survive a reopen
    ev(&it, r#"(sheet-insert-rows "Budget" 1)"#);
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(":heights ((3 48))"));
    ev(&it, r#"(sheet-close "Budget")"#);
    let other = engine(&dir);
    let payload = ev(&other, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(":heights ((3 48))") && payload.contains(":widths ((1 200))"), "{payload}");
    ev(&other, r#"(sheet-row-height "Budget" 4 nil)"#);
    assert!(ev(&other, r#"(sheet-open "Budget")"#).contains(":heights ()"));
    assert!(err(&other, r#"(sheet-row-height "Budget" 4 -1)"#).contains("positive number"));
}

// ── one sheet reading another ────────────────────────────────────────

#[test]
fn a_formula_reads_another_sheet_by_name() {
    let (dir, it) = budget("cross");
    ev(&it, r#"(sheet-new "Rates")"#);
    ev(&it, r#"(sheet-set "Rates" "A1" '(("0.2") ("0.3") ("0.5")))"#);
    set(&it, "A1", "100");
    set(&it, "B1", "=(* A1 Rates!A1)");
    set(&it, "B2", "=(sum Rates!A1:A3)");
    assert_eq!(get(&it, "B1"), "20");
    assert_eq!(get(&it, "B2"), "1");

    // a sheet in a folder is named from the folder of the sheet reading it
    std::fs::create_dir(dir.join("money")).unwrap();
    ev(&it, r#"(sheet-new "money/Q3")"#);
    ev(&it, r#"(sheet-set "money/Q3" "A1" "7")"#);
    set(&it, "B3", "=(+ money/Q3!A1 1)");
    assert_eq!(get(&it, "B3"), "8");
}

#[test]
fn changing_a_sheet_redoes_the_open_ones_that_read_it() {
    let (_, it) = budget("cross-update");
    ev(&it, r#"(sheet-new "Rates")"#);
    ev(&it, r#"(sheet-set "Rates" "A1" "0.2")"#);
    set(&it, "A1", "100");
    set(&it, "B1", "=(* A1 Rates!A1)");
    assert_eq!(get(&it, "B1"), "20");

    // writing the other sheet is enough — nothing asks Budget to recalculate
    ev(&it, r#"(sheet-set "Rates" "A1" "0.5")"#);
    assert_eq!(get(&it, "B1"), "50");

    // and an error over there arrives here, named
    ev(&it, r#"(sheet-set "Rates" "A1" "=(nope)")"#);
    assert!(get_err(&it, "B1").contains("Rates!A1 has an error — Undefined symbol: nope"), "{}", get_err(&it, "B1"));
    ev(&it, r#"(sheet-set "Rates" "A1" "0.25")"#);
    assert_eq!(get(&it, "B1"), "25");
}

#[test]
fn two_sheets_reading_each_other_settle() {
    let (_, it) = budget("cross-circle");
    ev(&it, r#"(sheet-new "Other")"#);
    set(&it, "A1", "1");
    set(&it, "B1", "=(+ Other!A1 1)");
    ev(&it, r#"(sheet-set "Other" "A1" "=(+ Budget!A1 10)")"#);
    // each sheet is recomputed once per write: no bouncing, and both hold a value
    assert_eq!(ev(&it, r#"(sheet-get "Other" "A1")"#), "11");
    assert_eq!(get(&it, "B1"), "12");
}

// ── copying cells around ─────────────────────────────────────────────

#[test]
fn a_pasted_formula_reads_where_it_landed() {
    let (_, it) = budget("paste");
    ev(&it, r#"(sheet-set "Budget" "A1" '(("1" "10") ("2" "20") ("=(sum A1:A2)" "=(* A1 $B$1)")))"#);
    assert_eq!(get(&it, "A3"), "3");
    // copy the pair in row 3 one column right: relative references follow, $B$1 stays put
    ev(&it, r#"(sheet-paste "Budget" "B3" (sheet-copy "Budget" "A3:B3") "A3")"#);
    let payload = ev(&it, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#""=(sum B1:B2)""#), "{payload}");
    assert!(payload.contains(r#""=(* B1 $B$1)""#), "the anchored one didn't move: {payload}");
    assert_eq!(get(&it, "B3"), "30");
    assert_eq!(get(&it, "C3"), "100");

    // without a source, the text is typed as it is — a paste from another program
    ev(&it, r#"(sheet-paste "Budget" "E1" '(("=(sum A1:A2)")))"#);
    assert!(ev(&it, r#"(sheet-open "Budget")"#).contains(r#"(0 4 "=(sum A1:A2)" 3"#));

    // a reference pushed off the top of the sheet has nowhere to point
    ev(&it, r#"(sheet-paste "Budget" "A1" '(("=(+ A3 1)")) "A5")"#);
    assert!(get_err(&it, "A1").contains("#REF!"));
}

#[test]
fn fill_repeats_a_block_and_moves_its_references() {
    let (_, it) = budget("fill");
    ev(&it, r#"(sheet-set "Budget" "A1" '(("1") ("2") ("3") ("4")))"#);
    set(&it, "B1", "=(* A1 10)");
    ev(&it, r#"(sheet-fill "Budget" "B1" "B1:B4")"#);
    assert_eq!(ev(&it, r#"(sheet-rows "Budget" "B1:B4")"#), "((10) (20) (30) (40))");
    let payload = ev(&it, r#"(sheet-open "Budget")"#);
    assert!(payload.contains(r#""=(* A4 10)""#), "{payload}");

    // a two-cell source tiles across the target
    ev(&it, r#"(sheet-set "Budget" "D1" '(("x") ("y")))"#);
    ev(&it, r#"(sheet-fill "Budget" "D1:D2" "D1:D6")"#);
    assert_eq!(ev(&it, r#"(sheet-rows "Budget" "D1:D6")"#), r#"(("x") ("y") ("x") ("y") ("x") ("y"))"#);

    // filling from a formula that reads its own row keeps reading its own row
    set(&it, "E1", "=(str A1 \"!\")");
    ev(&it, r#"(sheet-fill "Budget" "E1" "E1:E3")"#);
    assert_eq!(ev(&it, r#"(sheet-rows "Budget" "E1:E3")"#), r#"(("1!") ("2!") ("3!"))"#);
}

#[test]
fn shift_formula_moves_only_what_is_free_to_move() {
    let rw = shift_formula("(+ A1 $A$1 B$2 $B2) ; A1 in a comment", 1, 2);
    assert_eq!(rw.text, "(+ C2 $A$1 D$2 $B3) ; A1 in a comment");
    assert!(rw.changed && !rw.resized);
    let off = shift_formula("(+ A1 1)", -1, 0);
    assert_eq!(off.text, "(+ #REF! 1)");
    assert!(off.resized);
    let ranges = shift_formula("(sum A1:B2)", 2, 0);
    assert_eq!(ranges.text, "(sum A3:B4)");
    assert_eq!(shift_formula("(+ 1 2)", 5, 5).changed, false);
}

// ── the aggregates a sheet leans on ──────────────────────────────────

#[test]
fn sum_avg_min_max_read_lists_and_skip_blanks_and_text() {
    let it = Interpreter::new();
    assert_eq!(ev(&it, r#"(sum 1 '(2 nil "x" (3)))"#), "6");
    assert_eq!(ev(&it, "(sum)"), "0");
    assert_eq!(ev(&it, "(avg '(2 4 nil))"), "3");
    assert_eq!(ev(&it, "(min 3 '(1 nil 2))"), "1");
    assert_eq!(ev(&it, "(max 3 1 2)"), "3", "plain numbers as before");
    assert!(err(&it, "(min)").contains("needs at least one number"), "used to panic the engine thread");
    assert!(err(&it, "(avg '(nil))").contains("needs at least one number"));
    assert!(err(&it, r#"(sum "3")"#).contains("Type mismatch"));
}

// ── a sheet as bytes — what a browser, which has no files, is handed ──

#[test]
fn a_sheet_travels_as_bytes_formulas_and_all() {
    let (_dir, a) = budget("bytes-a");
    set(&a, "A1", "40");
    set(&a, "A2", "2");
    set(&a, "A3", "=(+ A1 A2)");
    let bytes = a.export_sheet("Budget").unwrap();
    assert_eq!(&bytes[..16], b"SQLite format 3\0");

    // Another engine, in another workspace where no Budget file exists: it opens from the bytes.
    let dir = scratch("bytes-b");
    let b = engine(&dir);
    b.import_sheet("Budget", &bytes).unwrap();
    assert_eq!(get(&b, "A3"), "42");
    assert!(!dir.join("Budget.eesheet").exists(), "nothing was written to disk");

    // It is a live sheet there: a change recalculates, and moves its version.
    let before = b.sheet_versions();
    set(&b, "A1", "100");
    assert_eq!(get(&b, "A3"), "102");
    let after = b.sheet_versions();
    assert_eq!(after.len(), 1);
    assert!(after[0].1 > before[0].1, "{before:?} → {after:?}");

    // …and goes back out as bytes with the change in it.
    let c = engine(&scratch("bytes-c"));
    c.import_sheet("Budget", &b.export_sheet("Budget").unwrap()).unwrap();
    assert_eq!(get(&c, "A3"), "102");
}

#[test]
fn bytes_that_are_not_a_sheet_are_refused() {
    let it = engine(&scratch("bytes-bad"));
    let e = it.import_sheet("Budget", b"certainly not a sqlite file, just text").unwrap_err().to_string();
    assert!(e.contains("is not a sheet"), "{e}");
    // A SQLite database that isn't a sheet — the engine's own — is refused too.
    let db = Interpreter::new().export_database().unwrap();
    let e = it.import_sheet("Budget", &db).unwrap_err().to_string();
    assert!(e.contains("is not a sheet"), "{e}");
    assert!(it.export_sheet("Budget").is_err());
}
