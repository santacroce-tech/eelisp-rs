//! `(functions …)` and `(source …)` — the language documenting itself.
//!
//! Both builtins print and return nil, so every assertion here reads the captured output.

use eelisp::docs::{self, Entry};
use eelisp::printer::print_value;
use eelisp::value::Value;
use eelisp::Interpreter;

/// Evaluate and return what was printed.
fn out(it: &Interpreter, src: &str) -> String {
    it.eval_str(src).expect("eval ok");
    it.take_output()
}

fn fresh() -> Interpreter {
    let it = Interpreter::new();
    it.set_echo(false);
    it.take_output();
    it
}

// ─────────────────────────── (functions …) ───────────────────────────

#[test]
fn functions_lists_everything_by_default() {
    let it = fresh();
    let listing = out(&it, "(functions)");
    // one row per kind, under its own heading
    for heading in ["special forms\n", "builtins\n", "macros\n", "functions\n"] {
        assert!(listing.contains(heading), "no {heading:?} group:\n{listing}");
    }
    assert!(listing.contains("\n  map "), "a builtin:\n{listing}");
    assert!(listing.contains("\n  cond "), "a special form:\n{listing}");
    assert!(listing.contains("\n  take "), "a prelude function:\n{listing}");
    assert!(listing.contains("\n  when "), "a prelude macro:\n{listing}");
    assert!(listing.contains("— 186 functions\n") || listing.contains(" functions\n"), "a total:\n{listing}");
}

#[test]
fn functions_filters_by_name_case_insensitively() {
    let it = fresh();
    let listing = out(&it, "(functions \"STR-JOIN\")");
    assert!(listing.contains("str-join"), "{listing}");
    assert!(!listing.contains("str-split"), "{listing}");
    assert!(listing.contains("1 of "), "{listing}");
}

#[test]
fn functions_filter_matches_anywhere_in_the_name() {
    let it = fresh();
    let listing = out(&it, "(functions \"item\")");
    for name in ["add-item", "item-get", "items-between"] {
        assert!(listing.contains(name), "missing {name} in:\n{listing}");
    }
}

#[test]
fn functions_groups_rows_under_the_kind_they_are() {
    let it = fresh();
    let listing = out(&it, "(functions \"str-len\")");
    assert!(listing.starts_with("builtins\n"), "{listing}");
    assert!(listing.contains("  str-len  (str-len s) → number"), "{listing}");
}

#[test]
fn functions_groups_the_language_first_and_user_code_last() {
    let it = fresh();
    let listing = out(&it, "(functions \"co\")");
    let at = |h: &str| listing.find(h).unwrap_or_else(|| panic!("no {h:?} in:\n{listing}"));
    assert!(at("special forms") < at("builtins"), "{listing}");
    assert!(at("builtins") < at("functions"), "{listing}");
}

#[test]
fn functions_shows_a_user_functions_real_parameter_list() {
    let it = fresh();
    let listing = out(&it, "(defn shout (text . rest) text) (functions \"shout\")");
    assert!(listing.contains("(shout text . rest)"), "{listing}");
    assert!(listing.contains("functions\n"), "{listing}");
}

#[test]
fn functions_says_so_when_nothing_matches() {
    let it = fresh();
    let listing = out(&it, "(functions \"nosuchthing\")");
    assert!(listing.contains("no function matching"), "{listing}");
    assert!(listing.contains("(functions) lists all"), "{listing}");
}

#[test]
fn functions_accepts_a_symbol_or_keyword_filter() {
    let it = fresh();
    for src in ["(functions 'date)", "(functions :date)"] {
        let listing = out(&it, src);
        assert!(listing.contains("date-add"), "{src}:\n{listing}");
    }
}

#[test]
fn functions_returns_nil_so_the_listing_is_the_output() {
    let it = fresh();
    assert_eq!(it.eval_str("(functions \"map\")").unwrap(), Value::Null);
}

// ───────────────────────────── (source …) ────────────────────────────

#[test]
fn source_of_a_builtin_is_its_manual_entry() {
    let it = fresh();
    let text = out(&it, "(source str-join)");
    assert!(text.contains("str-join — builtin"), "{text}");
    assert!(text.contains("(str-join sep parts) → string"), "{text}");
    assert!(text.contains("Either argument order is accepted"), "{text}");
    assert!(text.contains("(str-join \"-\" '(\"a\" \"b\"))"), "an example:\n{text}");
}

#[test]
fn source_of_a_special_form_works_even_though_it_is_not_a_binding() {
    let it = fresh();
    let text = out(&it, "(source cond)");
    assert!(text.contains("cond — special form"), "{text}");
    assert!(text.contains("(cond test result …)"), "{text}");
}

#[test]
fn source_echoes_a_definition_with_the_comments_written_above_it() {
    let it = fresh();
    let text = out(
        &it,
        ";; Builds a greeting.\n;; (greet \"Ada\") → \"Hello, Ada!\"\n\
         (defn greet (name)\n  (str \"Hello, \" name \"!\"))\n(source greet)",
    );
    assert!(text.contains("greet — function"), "{text}");
    assert!(text.contains(";; Builds a greeting."), "{text}");
    assert!(text.contains(";; (greet \"Ada\") → \"Hello, Ada!\""), "{text}");
    // The body comes back as written, indentation included — not re-printed from the AST.
    assert!(text.contains("(defn greet (name)\n    (str \"Hello, \" name \"!\"))"), "{text}");
}

#[test]
fn a_comment_block_stops_at_a_blank_line() {
    let it = fresh();
    let text = out(&it, ";; belongs to nothing\n\n(defn bare (x) x)\n(source bare)");
    assert!(!text.contains("belongs to nothing"), "{text}");
    assert!(text.contains("(defn bare (x) x)"), "{text}");
}

#[test]
fn a_comment_block_stops_at_the_definition_above_it() {
    let it = fresh();
    let text = out(&it, ";; for one\n(defn one (x) x)\n(defn two (x) x)\n(source two)");
    assert!(!text.contains("for one"), "{text}");
}

#[test]
fn a_comment_after_a_definition_on_the_same_line_belongs_to_it() {
    let it = fresh();
    let text = out(&it, "(defn ed-goto (pos) pos)  ;; caret to an offset\n(source ed-goto)");
    assert!(text.contains(";; caret to an offset"), "{text}");
}

#[test]
fn a_trailing_comment_on_the_line_above_is_not_picked_up() {
    let it = fresh();
    let text = out(&it, "(defn one (x) x) ;; a note about one\n(defn two (x) x)\n(source two)");
    assert!(!text.contains("a note about one"), "{text}");
}

#[test]
fn redefining_a_function_replaces_its_recorded_source() {
    let it = fresh();
    out(&it, ";; first\n(defn f (x) x)");
    let text = out(&it, ";; second\n(defn f (x) (* x 2))\n(source f)");
    assert!(text.contains(";; second"), "{text}");
    assert!(!text.contains(";; first"), "{text}");
}

#[test]
fn source_accepts_a_bare_symbol_a_quoted_symbol_or_a_string() {
    let it = fresh();
    for src in ["(source map)", "(source 'map)", "(source \"map\")"] {
        let text = out(&it, src);
        assert!(text.contains("map — builtin"), "{src}:\n{text}");
    }
}

#[test]
fn source_rebuilds_a_definition_it_never_saw_in_text() {
    let it = fresh();
    // Defined inside a `do`, so no top-level form carried its text.
    let text = out(&it, "(do (defn inner (a b) (+ a b)))\n(source inner)");
    assert!(text.contains("inner — function"), "{text}");
    assert!(text.contains("no source text on record"), "{text}");
    assert!(text.contains("(defn inner (a b)"), "{text}");
    assert!(text.contains("(+ a b)"), "{text}");
}

#[test]
fn source_of_a_macro_says_macro() {
    let it = fresh();
    let text = out(&it, "(source when)");
    assert!(text.contains("when — macro"), "{text}");
    assert!(text.contains("(defmacro when (test . body)"), "{text}");
}

#[test]
fn source_of_a_prelude_function_carries_the_prelude_comment() {
    let it = fresh();
    let text = out(&it, "(source take)");
    assert!(text.contains(";; The first n elements."), "{text}");
    assert!(text.contains("(defn take (n lst)"), "{text}");
}

#[test]
fn source_of_a_plain_binding_names_its_type_and_shows_how_it_was_written() {
    let it = fresh();
    let text = out(&it, ";; the cap\n(def limit 10)\n(source limit)");
    assert!(text.contains("limit — number"), "{text}");
    assert!(text.contains(";; the cap"), "{text}");
    assert!(text.contains("(def limit 10)"), "{text}");
}

#[test]
fn source_of_a_binding_it_never_saw_in_text_says_what_it_is() {
    let it = fresh();
    let text = out(&it, "(do (def hidden 10))\n(source hidden)");
    assert!(text.contains("hidden — number, not a function"), "{text}");
    assert!(text.contains("10"), "{text}");
}

#[test]
fn source_of_an_unknown_name_is_an_error_that_points_at_functions() {
    let it = fresh();
    let err = it.eval_str("(source nosuchthing)").unwrap_err().to_string();
    assert!(err.contains("nothing named"), "{err}");
    assert!(err.contains("(functions"), "{err}");
}

#[test]
fn source_returns_nil_so_the_listing_is_the_output() {
    let it = fresh();
    assert_eq!(it.eval_str("(source map)").unwrap(), Value::Null);
}

// ───────────────────── the manual matches the engine ─────────────────

/// Every builtin the interpreter registers has a manual entry, and every manual entry names a
/// builtin that exists. Without this, adding a builtin silently ships an undocumented one.
#[test]
fn the_manual_and_the_environment_agree() {
    let it = Interpreter::new();
    let registered: Vec<String> = it
        .global
        .borrow()
        .vars
        .iter()
        .filter(|(_, v)| matches!(v, Value::Builtin(_)))
        .map(|(k, _)| k.to_string())
        .collect();

    let mut undocumented: Vec<&String> =
        registered.iter().filter(|n| docs::builtin_entry(n).is_none()).collect();
    undocumented.sort();
    assert!(undocumented.is_empty(), "builtins with no manual entry: {undocumented:?}");

    let mut phantom: Vec<&str> = docs::builtins()
        .map(|e| e.name)
        .filter(|n| !registered.iter().any(|r| r == n))
        .collect();
    phantom.sort();
    assert!(phantom.is_empty(), "manual entries for builtins that don't exist: {phantom:?}");
}

/// Every manual entry is complete and unique — no blank prose, no name listed twice.
#[test]
fn the_manual_is_well_formed() {
    let all: Vec<&Entry> = docs::SPECIAL_FORMS.iter().chain(docs::builtins()).collect();
    for e in &all {
        assert!(!e.sig.trim().is_empty(), "{} has no signature", e.name);
        assert!(!e.summary.trim().is_empty(), "{} has no summary", e.name);
        assert!(e.sig.contains(e.name), "{}'s signature doesn't name it: {}", e.name, e.sig);
    }
    let mut names: Vec<&str> = all.iter().map(|e| e.name).collect();
    names.sort();
    let before = names.len();
    names.dedup();
    assert_eq!(before, names.len(), "the manual lists a name twice");
}

/// Every special form the manual documents is one `source` can actually find.
#[test]
fn every_special_form_is_reachable_from_source() {
    let it = fresh();
    for e in docs::SPECIAL_FORMS {
        let text = out(&it, &format!("(source {})", e.name));
        assert!(text.contains(e.name), "(source {}) said:\n{text}", e.name);
    }
}

// ───────────────────── function-list / source-text ───────────────────

#[test]
fn function_list_returns_the_rows_functions_prints() {
    let it = fresh();
    let v = it
        .eval_str("(map (fn (r) (list (dict-get r \"name\") (dict-get r \"kind\") (dict-get r \"sig\"))) (function-list \"str-join\"))")
        .unwrap();
    let text = print_value(&v, true);
    assert!(text.contains("\"str-join\" \"builtin\""), "{text}");
    assert!(it.eval_str("(function-list \"str-join\")").is_ok());
}

#[test]
fn function_list_carries_the_summary_a_definition_was_written_with() {
    let it = fresh();
    let v = it
        .eval_str(";; Says hello.\n(defn greet (name) name)\n(first (function-list \"greet\"))")
        .unwrap();
    let text = print_value(&v, true);
    assert!(text.contains("\"function\""), "{text}");
    assert!(text.contains("(greet name)"), "{text}");
    assert!(text.contains("Says hello."), "{text}");
}

#[test]
fn function_list_with_no_match_is_empty_not_an_error() {
    let it = fresh();
    assert_eq!(it.eval_str("(length (function-list \"nosuchthing\"))").unwrap(), Value::Number(0.0));
}

#[test]
fn source_text_returns_what_source_prints() {
    let it = fresh();
    let v = it.eval_str(";; squares\n(defn sq (n) (* n n))\n(def which \"sq\")\n(source-text which)").unwrap();
    match v {
        Value::Str(s) => {
            assert!(s.contains("sq — function"), "{s}");
            assert!(s.contains(";; squares"), "{s}");
        }
        other => panic!("expected a string, got {other:?}"),
    }
    assert!(it.eval_str("(source-text \"nosuchthing\")").is_err());
}

#[test]
fn function_list_summary_is_the_first_sentence_even_across_lines() {
    let it = fresh();
    let v = it
        .eval_str(";; Adds two numbers and\n;; returns the sum. Nothing else.\n(defn add2 (a b) (+ a b))\n(dict-get (first (function-list \"add2\")) \"summary\")")
        .unwrap();
    assert_eq!(v, Value::Str("Adds two numbers and returns the sum.".into()));
}
