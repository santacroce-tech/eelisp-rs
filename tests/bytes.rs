//! Byte buffers, bitwise integers and hex literals — the primitives code that works on raw memory
//! needs (a Z80 written in EELisp is the first user).

use eelisp::host::to_json;
use eelisp::lexer::{lex, Token};
use eelisp::printer::print_value;
use eelisp::Interpreter;

fn s(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).expect("eval ok"), false)
}
fn err(it: &Interpreter, src: &str) -> String {
    it.eval_str(src).expect_err("an error").to_string()
}

// ── hex literals ──

#[test]
fn hex_literals_read_as_numbers() {
    assert_eq!(lex("0xff").unwrap(), vec![Token::Num(255.0)]);
    assert_eq!(lex("0X4000").unwrap(), vec![Token::Num(16384.0)]);
    assert_eq!(lex("-0x10").unwrap(), vec![Token::Num(-16.0)]);
    let it = Interpreter::new();
    assert_eq!(s(&it, "(+ 0x3e 1)"), "63");
}

#[test]
fn what_isnt_hex_stays_a_symbol() {
    assert_eq!(lex("0x").unwrap(), vec![Token::Sym("0x".into())]);
    assert_eq!(lex("0xg1").unwrap(), vec![Token::Sym("0xg1".into())]);
    assert_eq!(lex("0x12345678901234").unwrap(), vec![Token::Sym("0x12345678901234".into())]);
}

// ── bitwise ──

#[test]
fn bitwise_on_integers() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "(band 0xf0 0x3c)"), "48");
    assert_eq!(s(&it, "(bor 0xf0 0x0f)"), "255");
    assert_eq!(s(&it, "(bxor 0xff 0x0f)"), "240");
    assert_eq!(s(&it, "(bnot 0)"), "-1");
    assert_eq!(s(&it, "(band (bnot 0x0f) 0xff)"), "240");
    assert_eq!(s(&it, "(shl 1 15)"), "32768");
    assert_eq!(s(&it, "(shr 0x4000 8)"), "64");
    assert_eq!(s(&it, "(shr -16 2)"), "-4");
}

#[test]
fn bitwise_refuses_what_isnt_an_exact_integer() {
    let it = Interpreter::new();
    assert!(err(&it, "(band 1.5 1)").contains("exact integer"));
    assert!(err(&it, "(bor \"a\" 1)").contains("number"));
    assert!(err(&it, "(shl 1 60)").contains("shift"));
    assert!(err(&it, "(shl 0x1000000000000 8)").contains("2^53"));
}

// ── byte buffers ──

#[test]
fn a_buffer_is_zeroed_and_sized() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "(bytes-len (make-bytes 65536))"), "65536");
    assert_eq!(s(&it, "(bget (make-bytes 4) 3)"), "0");
    assert_eq!(s(&it, "(make-bytes 3 7)"), "#<bytes 3: 07 07 07>");
    assert_eq!(s(&it, "(make-bytes 17)"), "#<bytes 17>");
}

#[test]
fn bset_mutates_in_place_for_every_holder() {
    let it = Interpreter::new();
    s(&it, "(def ram (make-bytes 8))");
    s(&it, "(def alias ram)");
    assert_eq!(s(&it, "(bset ram 2 0x1ff)"), "255"); // stored & 0xff
    assert_eq!(s(&it, "(bget alias 2)"), "255");
    assert_eq!(s(&it, "(bset ram 3 -1)"), "255");
}

#[test]
fn indexes_are_checked() {
    let it = Interpreter::new();
    s(&it, "(def b (make-bytes 4))");
    assert!(err(&it, "(bget b 4)").contains("4"));
    assert!(err(&it, "(bset b -1 0)").contains("-1"));
    assert!(err(&it, "(bfill b 2 3 0)").contains("past the end"));
}

#[test]
fn fill_and_copy_runs() {
    let it = Interpreter::new();
    s(&it, "(def b (list->bytes '(1 2 3 4 5 6)))");
    s(&it, "(bfill b 0 2 9)");
    assert_eq!(s(&it, "(bytes->list b)"), "(9 9 3 4 5 6)");
    s(&it, "(def c (make-bytes 3))");
    s(&it, "(bcopy c 0 b 3 3)");
    assert_eq!(s(&it, "(bytes->list c)"), "(4 5 6)");
    // overlapping, within one buffer — LDIR-style
    s(&it, "(bcopy b 1 b 0 5)");
    assert_eq!(s(&it, "(bytes->list b)"), "(9 9 9 3 4 5)");
}

#[test]
fn base64_round_trips_and_takes_a_run() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "(bytes->base64 (list->bytes '(1 2 3)))"), "AQID");
    assert_eq!(s(&it, "(bytes->base64 (list->bytes '(0 1 2 3 0)) 1 3)"), "AQID");
    assert_eq!(s(&it, "(bytes->list (base64->bytes \"AQID\"))"), "(1 2 3)");
    assert!(err(&it, "(base64->bytes \"abc\")").contains("base64"));
}

#[test]
fn bytes_compare_by_content_and_cross_the_host_as_base64() {
    let it = Interpreter::new();
    assert_eq!(s(&it, "(= (list->bytes '(1 2)) (list->bytes '(1 2)))"), "true");
    assert_eq!(s(&it, "(type (make-bytes 1))"), "bytes");
    let v = it.eval_str("(list->bytes '(1 2 3))").unwrap();
    assert_eq!(to_json(&v), serde_json::json!({ "$bytes": "AQID" }));
}
