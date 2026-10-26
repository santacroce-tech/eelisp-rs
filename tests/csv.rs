//! csv-parse, csv-records, csv-stringify — the same rules as the app's CSV reader
//! (eeditor-next src/core/sheet.ts), so a file reads the same in a sheet import and in a script.

use eelisp::printer::print_value;
use eelisp::Interpreter;

fn ev(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).unwrap_or_else(|e| panic!("{src}\n  → {e}")), true)
}

#[test]
fn quotes_separators_and_line_breaks() {
    let it = Interpreter::new();
    assert_eq!(ev(&it, r#"(csv-parse "a,b\n1,2")"#), r#"(("a" "b") ("1" "2"))"#);
    assert_eq!(ev(&it, r#"(csv-parse "x,\"a, b\",\"say \"\"hi\"\"\"")"#), r#"(("x" "a, b" "say \"hi\""))"#);
    assert_eq!(ev(&it, r#"(= (csv-parse "\"two\nlines\",z") (list (list "two\nlines" "z")))"#), "true");
    assert_eq!(ev(&it, r#"(csv-parse "a\r\nb\r\n")"#), r#"(("a") ("b"))"#, "CRLF, and no row for the last newline");
    assert_eq!(ev(&it, r#"(csv-parse "a,,c")"#), r#"(("a" "" "c"))"#);
    assert_eq!(ev(&it, r#"(csv-parse "")"#), "()");
    assert_eq!(ev(&it, "(csv-parse \"a\\tb\" \"\\t\")"), r#"(("a" "b"))"#, "TSV");
}

#[test]
fn records_by_header_with_numbers() {
    let it = Interpreter::new();
    assert_eq!(
        ev(&it, r#"(csv-records "symbol,price,note\nAAPL,231.5,\nMSFT, 412 ,big")"#),
        r#"({:symbol "AAPL" :price 231.5 :note nil} {:symbol "MSFT" :price 412 :note "big"})"#
    );
    assert_eq!(ev(&it, r#"(csv-records "a,b")"#), "()", "a header and nothing else");
    assert_eq!(ev(&it, r#"(dict-get (first (csv-records "id,code\n7,007")) :code)"#), "7", "numeric text is a number");
}

#[test]
fn stringify_quotes_what_needs_it_and_reads_back() {
    let it = Interpreter::new();
    assert_eq!(
        ev(&it, r#"(= (csv-stringify '(("a" "b") (1 "x, y") (nil "say \"hi\""))) "a,b\n1,\"x, y\"\n,\"say \"\"hi\"\"\"")"#),
        "true"
    );
    assert_eq!(
        ev(&it, r#"(= (csv-parse (csv-stringify '(("x, y" "two\nlines" "q\"q")))) '(("x, y" "two\nlines" "q\"q")))"#),
        "true",
        "round trip"
    );
}

#[test]
fn a_bad_separator_says_so() {
    let it = Interpreter::new();
    assert!(it.eval_str(r#"(csv-parse "a" ";;")"#).unwrap_err().to_string().contains("one character"));
}
