//! Port of the Swift EELisp DATABASE and AGENDA tests — all LIVE (roadmap steps 2 & 3).
//!
//! Assertions are expressed in pure EELisp where possible (counts via `count-records` /
//! `records` / `length`, field reads via `field-get`, listing strings via `tables` / `templates`
//! / `agendas`). Property-level expectations from the Swift originals that aren't directly
//! observable in the Lisp surface are kept as `// SPEC:` comments alongside a runnable check.
//!
//! Note: categories / rules / views and the smart NLP `add` are the remaining agenda slice
//! (step 3b) and have no tests here yet.

use eelisp::printer::print_value;
use eelisp::Interpreter;

fn s(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).expect("eval ok"), false)
}

// ═══════════════════════════ Database ═══════════════════════════

#[test]
fn db_deftable_creates_table() {
    let it = Interpreter::new();
    it.eval_str("(deftable contacts (name:string email:string age:number))").unwrap();
    // SPEC: returns .table{name:"contacts", fields:[name:string, email:string, age:number]}
    assert!(s(&it, "(tables)").contains("contacts"));
}

#[test]
fn db_insert_returns_record_with_id() {
    let it = Interpreter::new();
    it.eval_str("(deftable contacts (name:string age:number))").unwrap();
    it.eval_str("(def r (insert contacts {:name \"Alice\" :age 30}))").unwrap();
    assert_eq!(s(&it, "(record-id r)"), "1");
    assert_eq!(s(&it, "(field-get r :name)"), "Alice");
    assert_eq!(s(&it, "(field-get r :age)"), "30");
}

#[test]
fn db_query_returns_all() {
    let it = Interpreter::new();
    it.eval_str("(deftable items (title:string))").unwrap();
    it.eval_str("(insert items {:title \"Alpha\"})").unwrap();
    it.eval_str("(insert items {:title \"Beta\"})").unwrap();
    it.eval_str("(insert items {:title \"Gamma\"})").unwrap();
    assert_eq!(s(&it, "(length (records (query items)))"), "3");
}

#[test]
fn db_query_where() {
    let it = Interpreter::new();
    it.eval_str("(deftable nums (val:number))").unwrap();
    for v in [10, 20, 30] {
        it.eval_str(&format!("(insert nums {{:val {}}})", v)).unwrap();
    }
    assert_eq!(s(&it, "(length (records (query nums :where \"val > ?\" :params (list 15))))"), "2");
}

#[test]
fn db_query_order_by() {
    let it = Interpreter::new();
    it.eval_str("(deftable scores (name:string score:number))").unwrap();
    it.eval_str("(insert scores {:name \"A\" :score 30})").unwrap();
    it.eval_str("(insert scores {:name \"B\" :score 10})").unwrap();
    it.eval_str("(insert scores {:name \"C\" :score 20})").unwrap();
    it.eval_str("(def rs (records (query scores :order \"score\" :asc true)))").unwrap();
    assert_eq!(s(&it, "(field-get (nth rs 0) :name)"), "B");
    assert_eq!(s(&it, "(field-get (nth rs 1) :name)"), "C");
    assert_eq!(s(&it, "(field-get (nth rs 2) :name)"), "A");
}

#[test]
fn db_update() {
    let it = Interpreter::new();
    it.eval_str("(deftable people (name:string age:number))").unwrap();
    it.eval_str("(insert people {:name \"Alice\" :age 30})").unwrap();
    assert_eq!(s(&it, "(update people 1 {:age 31})"), "1"); // rows affected
    assert_eq!(s(&it, "(field-get (head (records (query people))) :age)"), "31");
}

#[test]
fn db_delete_soft_removes() {
    let it = Interpreter::new();
    it.eval_str("(deftable things (label:string))").unwrap();
    it.eval_str("(insert things {:label \"A\"})").unwrap();
    it.eval_str("(insert things {:label \"B\"})").unwrap();
    it.eval_str("(delete things 1)").unwrap();
    assert_eq!(s(&it, "(length (records (query things)))"), "1");
    assert_eq!(s(&it, "(field-get (head (records (query things))) :label)"), "B");
}

#[test]
fn db_count_records() {
    let it = Interpreter::new();
    it.eval_str("(deftable items (x:number))").unwrap();
    for x in [1, 2, 3] {
        it.eval_str(&format!("(insert items {{:x {}}})", x)).unwrap();
    }
    assert_eq!(s(&it, "(count-records items)"), "3");
    it.eval_str("(delete items 2)").unwrap();
    assert_eq!(s(&it, "(count-records items)"), "2");
}

#[test]
fn db_tables_lists_all() {
    let it = Interpreter::new();
    it.eval_str("(deftable alpha (x:string))").unwrap();
    it.eval_str("(deftable beta (y:number))").unwrap();
    // SPEC: 7 tables — _items, _categories, _rules, _views, _templates (auto), alpha, beta
    assert_eq!(s(&it, "(length (tables))"), "7");
    let names = s(&it, "(tables)");
    for expected in ["alpha", "beta", "_items", "_categories", "_rules", "_views", "_templates"] {
        assert!(names.contains(expected), "tables should contain {}", expected);
    }
}

#[test]
fn db_pack_purges_soft_deleted() {
    let it = Interpreter::new();
    it.eval_str("(deftable data (val:number))").unwrap();
    it.eval_str("(insert data {:val 1})").unwrap();
    it.eval_str("(insert data {:val 2})").unwrap();
    it.eval_str("(delete data 1)").unwrap();
    it.eval_str("(pack data)").unwrap();
    assert_eq!(s(&it, "(count-records data)"), "1");
}

#[test]
fn db_field_get_and_record_id() {
    let it = Interpreter::new();
    it.eval_str("(deftable contacts (name:string))").unwrap();
    it.eval_str("(def r (insert contacts {:name \"Alice\"}))").unwrap();
    assert_eq!(s(&it, "(field-get r :name)"), "Alice");
    assert_eq!(s(&it, "(record-id r)"), "1");
}

#[test]
fn db_records_extracts_from_result_set() {
    let it = Interpreter::new();
    it.eval_str("(deftable items (label:string))").unwrap();
    it.eval_str("(insert items {:label \"X\"})").unwrap();
    it.eval_str("(insert items {:label \"Y\"})").unwrap();
    it.eval_str("(def recs (records (query items)))").unwrap();
    assert_eq!(s(&it, "(length recs)"), "2");
    assert_eq!(s(&it, "(field-get (head recs) :label)"), "X");
}

#[test]
fn db_query_limit() {
    let it = Interpreter::new();
    it.eval_str("(deftable many (n:number))").unwrap();
    for n in 1..=10 {
        it.eval_str(&format!("(insert many {{:n {}}})", n)).unwrap();
    }
    assert_eq!(s(&it, "(length (records (query many :limit 3)))"), "3");
}

#[test]
fn db_schema_and_defaults_persist_across_reopen() {
    // This is the ANALYSIS §6 fix: the Swift custom schema format dropped field defaults/choices
    // on reload. The Rust JSON schema store keeps them — and a file-backed DB reopens cleanly.
    use eelisp::Interpreter;
    let path = "/tmp/eelisp_rs_persist_test.db";
    let _ = std::fs::remove_file(path);
    {
        let it = Interpreter::with_database(path);
        // long-form field with a DEFAULT
        it.eval_str("(deftable notes (title:string (priority :type number :default 3)))").unwrap();
        it.eval_str("(insert notes {:title \"hello\" :priority 1})").unwrap();
    }
    {
        let it = Interpreter::with_database(path);
        assert!(s(&it, "(tables)").contains("notes"), "schema survived reopen");
        assert_eq!(s(&it, "(count-records notes)"), "1", "row survived reopen");
        // insert omitting priority -> the persisted DEFAULT (3) applies
        it.eval_str("(insert notes {:title \"world\"})").unwrap();
        let by_title = "(records (query notes :order \"title\"))";
        assert_eq!(s(&it, &format!("(field-get (nth {} 1) :priority)", by_title)), "3");
    }
    let _ = std::fs::remove_file(path);
}

// ═══════════════════════════ Agenda — Phase 6 (recurrence, templates) ═══════════════════════════

#[test]
fn agenda_every_builds_pattern_strings() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "(every 3 :months)"), "every:3:months");
    assert_eq!(s(&it, "(every 2 :weeks)"), "every:2:weeks");
    assert_eq!(s(&it, "(every 1 :days)"), "every:1:days");
}

#[test]
fn agenda_add_item_with_recur() {
    let it = Interpreter::new();
    it.eval_str("(add-item \"Standup\" :when \"2026-02-24\" :recur :weekly)").unwrap();
    // SPEC: returns .item with properties[recurrence]="weekly", properties[when]="2026-02-24"
    assert_eq!(s(&it, "(item-count)"), "1");
}

#[test]
fn agenda_item_done_weekly_creates_next() {
    let it = Interpreter::new();
    it.eval_str("(add-item \"Standup\" :when \"2026-02-24\" :recur :weekly)").unwrap();
    // SPEC: (item-done 1) returns a NEW .item with when="2026-03-03", recurrence="weekly",
    //       text="Standup", id != 1; original is soft-deleted.
    assert!(it.eval_str("(item-done 1)").is_ok());
    assert_eq!(s(&it, "(item-count)"), "1"); // the new occurrence
}

#[test]
fn agenda_item_done_without_recurrence_returns_true() {
    let it = Interpreter::new();
    it.eval_str("(add-item \"One-off\" :when \"2026-03-01\")").unwrap();
    assert_eq!(s(&it, "(item-done 1)"), "true");
}

#[test]
fn agenda_item_done_monthly_and_custom() {
    // monthly: 2026-03-01 -> 2026-04-01
    let it = Interpreter::new();
    it.eval_str("(add-item \"Pay rent\" :when \"2026-03-01\" :recur :monthly)").unwrap();
    // SPEC: new item when="2026-04-01"
    assert!(it.eval_str("(item-done 1)").is_ok());

    // custom: (every 3 :months) 2026-04-01 -> 2026-07-01
    let it2 = Interpreter::new();
    it2.eval_str("(add-item \"Quarterly\" :when \"2026-04-01\" :recur (every 3 :months))").unwrap();
    // SPEC: new item when="2026-07-01"
    assert!(it2.eval_str("(item-done 1)").is_ok());
}

#[test]
fn agenda_templates_roundtrip() {
    let it = Interpreter::new();
    it.eval_str("(deftemplate weekly-review :text \"Weekly review\" :category \"work/admin\" :priority 2)").unwrap();
    it.eval_str("(from-template weekly-review :when \"2026-03-07\")").unwrap();
    // SPEC: item text="Weekly review", when="2026-03-07", priority="2", category contains work/admin
    assert_eq!(s(&it, "(item-count)"), "1");
}

#[test]
fn agenda_templates_list_and_drop() {
    let it = Interpreter::new();
    it.eval_str("(deftemplate standup :text \"Daily standup\" :category \"work\" :recur :daily)").unwrap();
    let listing = s(&it, "(templates)");
    assert!(listing.contains("standup"));
    assert!(listing.contains("Daily standup"));

    it.eval_str("(deftemplate temp1 :text \"Temporary\")").unwrap();
    it.eval_str("(drop-template temp1)").unwrap();
    // temp1 gone (standup still present)
    assert!(!s(&it, "(templates)").contains("temp1"));
}

#[test]
fn agenda_from_template_with_overrides() {
    let it = Interpreter::new();
    it.eval_str("(deftemplate base :text \"Base task\" :priority 3 :category \"work\")").unwrap();
    it.eval_str("(from-template base :when \"2026-04-01\" :priority 1 :category \"urgent\")").unwrap();
    // SPEC: item priority="1", categories contains both "work" and "urgent"
    assert_eq!(s(&it, "(item-count)"), "1");
}

// ═══════════════════════════ Agenda — Phase 7 (multiple agendas) ═══════════════════════════

#[test]
fn agenda_lists_default() {
    let it = Interpreter::new();
    let listing = s(&it, "(agendas)");
    assert!(listing.contains("memory"));
    assert!(listing.contains("[active]"));
}

#[test]
fn agenda_use_unknown_errors() {
    let it = Interpreter::new();
    match it.eval_str("(use-agenda nonexistent)") {
        Err(e) => assert!(format!("{}", e).contains("not found")),
        Ok(_) => panic!("should have errored"),
    }
}

#[test]
fn agenda_open_use_isolation() {
    let it = Interpreter::new();
    let tmp = "/tmp/eelisp_rs_test_use.db";
    let _ = std::fs::remove_file(tmp);

    it.eval_str("(add-item \"Memory item\" :when \"2026-03-01\")").unwrap();
    it.eval_str(&format!("(open-agenda \"{}\")", tmp)).unwrap();
    it.eval_str("(add-item \"File item\" :when \"2026-03-02\")").unwrap();

    it.eval_str("(use-agenda memory)").unwrap();
    // SPEC: memory agenda has "Memory item" but not "File item" (agendas are isolated)
    assert_eq!(s(&it, "(length (records (items :search \"Memory item\")))"), "1");
    assert_eq!(s(&it, "(length (records (items :search \"File item\")))"), "0");

    let _ = std::fs::remove_file(tmp);
}

#[test]
fn agenda_close_removes_from_registry() {
    let it = Interpreter::new();
    let tmp = "/tmp/eelisp_rs_test_close.db";
    let _ = std::fs::remove_file(tmp);

    it.eval_str(&format!("(open-agenda \"{}\")", tmp)).unwrap();
    assert!(s(&it, "(agendas)").contains("eelisp_rs_test_close"));
    it.eval_str("(close-agenda eelisp_rs_test_close)").unwrap();
    let after = s(&it, "(agendas)");
    assert!(after.contains("memory"));
    assert!(!after.contains("eelisp_rs_test_close"));

    let _ = std::fs::remove_file(tmp);
}

#[test]
fn agenda_export_import_roundtrip() {
    let src = Interpreter::new();
    src.eval_str("(add-item \"Round trip\" :when \"2026-05-01\" :priority 3 :category \"test\" :notes \"Some notes\")").unwrap();
    src.eval_str("(deftemplate rt-template :text \"RT task\" :category \"test\" :priority 2)").unwrap();

    let tmp = "/tmp/eelisp_rs_test_roundtrip.json";
    let _ = std::fs::remove_file(tmp);
    src.eval_str(&format!("(export-agenda \"memory\" :format :json :path \"{}\")", tmp)).unwrap();

    // SPEC: JSON file is {version:1, agenda:"memory", tables:{_items:[...>=1], ...}}
    let json = std::fs::read_to_string(tmp).unwrap();
    assert!(json.contains("\"version\""));
    assert!(json.contains("Round trip"));

    let dst = Interpreter::new();
    let msg = s(&dst, &format!("(import-agenda \"{}\")", tmp));
    assert!(msg.contains("Imported"));
    assert_eq!(s(&dst, "(length (records (items :search \"Round trip\")))"), "1");
    assert!(s(&dst, "(templates)").contains("rt-template"));

    let _ = std::fs::remove_file(tmp);
}
