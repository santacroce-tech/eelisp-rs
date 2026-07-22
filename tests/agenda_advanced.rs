//! Step-3b agenda features: categories (+ exclusivity), rules (auto + manual + regex `match`),
//! views, and the smart NLP `add`/`smart-parse`. No Swift counterpart existed, so these lock in
//! the intended behavior of the reflective engine.

use eelisp::printer::print_value;
use eelisp::Interpreter;

fn s(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).expect("eval ok"), false)
}

#[test]
fn categories_and_exclusivity() {
    let it = Interpreter::new();
    it.eval_str("(defcategory priority :exclusive true :children (high low))").unwrap();
    it.eval_str("(add-item \"task\")").unwrap();

    it.eval_str("(assign 1 \"priority/high\")").unwrap();
    assert_eq!(s(&it, "(length (records (items :category \"priority/high\")))"), "1");

    // exclusive parent: assigning low removes high
    it.eval_str("(assign 1 \"priority/low\")").unwrap();
    assert_eq!(s(&it, "(length (records (items :category \"priority/high\")))"), "0");
    assert_eq!(s(&it, "(length (records (items :category \"priority/low\")))"), "1");

    // unassign
    it.eval_str("(unassign 1 \"priority/low\")").unwrap();
    assert_eq!(s(&it, "(length (records (items :category \"priority/low\")))"), "0");
}

#[test]
fn categories_tree_listing() {
    let it = Interpreter::new();
    it.eval_str("(defcategory work)").unwrap();
    it.eval_str("(defcategory work/projects)").unwrap();
    let tree = s(&it, "(categories)");
    assert!(tree.contains("work"));
    assert!(tree.contains("work/projects"));
}

#[test]
fn rules_auto_categorize() {
    let it = Interpreter::new();
    it.eval_str("(defcategory priority :children (high))").unwrap();
    it.eval_str("(defrule urgent :when (str-contains text \"URGENT\") :assign \"priority/high\")").unwrap();
    it.eval_str("(auto-categorize true)").unwrap();

    it.eval_str("(add-item \"URGENT deploy\")").unwrap();
    it.eval_str("(add-item \"normal task\")").unwrap();
    // only the URGENT item picked up the category
    assert_eq!(s(&it, "(length (records (items :category \"priority/high\")))"), "1");
}

#[test]
fn rules_apply_manually() {
    let it = Interpreter::new();
    it.eval_str("(defrule urgent :when (str-contains text \"URGENT\") :assign \"priority/high\")").unwrap();
    it.eval_str("(add-item \"URGENT thing\")").unwrap(); // auto-categorize OFF, so not yet tagged
    assert_eq!(s(&it, "(length (records (items :category \"priority/high\")))"), "0");

    let changed = s(&it, "(apply-rules)");
    assert_eq!(changed, "1"); // one item changed
    assert_eq!(s(&it, "(length (records (items :category \"priority/high\")))"), "1");
}

#[test]
fn rules_regex_match_action() {
    // condition uses str-matches; action uses (match 1) to feed item-set — the reflective path
    let it = Interpreter::new();
    it.eval_str("(auto-categorize true)").unwrap();
    it.eval_str("(defrule datex :when (str-matches text \"(\\\\d{4}-\\\\d{2}-\\\\d{2})\") :action (item-set id :when (match 1)))").unwrap();
    it.eval_str("(add-item \"meeting 2026-09-09 with team\")").unwrap();
    assert_eq!(s(&it, "(length (records (items-on \"2026-09-09\")))"), "1");
}

#[test]
fn views_filter_result_set() {
    let it = Interpreter::new();
    it.eval_str("(add-item \"has work\")").unwrap();
    it.eval_str("(add-item \"no work\")").unwrap();
    it.eval_str("(assign 1 \"work\")").unwrap();
    it.eval_str("(defview workboard :filter (has-category \"work\") :sort-by when)").unwrap();
    assert!(s(&it, "(views)").contains("workboard"));
    assert_eq!(s(&it, "(length (records (show workboard)))"), "1");
}

#[test]
fn views_grouped_output() {
    let it = Interpreter::new();
    it.eval_str("(add-item \"a\")").unwrap();
    it.eval_str("(assign 1 \"work\")").unwrap();
    it.eval_str("(defview grp :group-by category)").unwrap();
    let out = s(&it, "(show grp)");
    assert!(out.contains("▸")); // grouped rendering
    assert!(out.contains("work"));
}

#[test]
fn smart_parse_extracts_fields() {
    let it = Interpreter::new();
    // ISO date + priority + person are all deterministic
    assert_eq!(s(&it, "(dict-get (smart-parse \"call Bob 2026-03-15 !!\") :when)"), "2026-03-15");
    assert_eq!(s(&it, "(dict-get (smart-parse \"call Bob 2026-03-15 !!\") :priority)"), "2");
    assert_eq!(s(&it, "(nth (dict-get (smart-parse \"meet Alice about launch\") :who) 0)"), "Alice");
    assert_eq!(s(&it, "(dict-get (smart-parse \"URGENT fix server\") :priority)"), "1");
}

#[test]
fn smart_add_creates_item() {
    let it = Interpreter::new();
    it.eval_str("(add \"URGENT fix the server\")").unwrap();
    assert_eq!(s(&it, "(item-count)"), "1");
    assert_eq!(s(&it, "(length (records (items :priority 1)))"), "1");
}
