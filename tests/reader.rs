//! The reader written in EELisp (`src/reader.eel`) against the Rust one (`parser::parse`).
//!
//! Both read the same texts. They must agree on every form, or stop with the same message. The
//! texts are the repository's own files (EELisp, but also Rust and Markdown, which make good
//! hostile input), a list of edge cases, and random strings over the characters that matter to a
//! reader. Each runs twice: on the reader interpreted, and on the reader as bytecode from the
//! bootstrap image.

use std::path::Path;

use eelisp::boot;
use eelisp::parser::parse;
use eelisp::printer::print_value;
use eelisp::{Interpreter, LispError, Value};

#[derive(Clone, Copy, Debug)]
enum Setup {
    Interpreted,
    FromImage,
}

fn engine(setup: Setup) -> Interpreter {
    match setup {
        Setup::Interpreted => {
            let it = Interpreter::new();
            it.set_compiled(false).unwrap();
            it.eval_all(boot::READER).expect("the reader loads");
            it
        }
        Setup::FromImage => boot::with_image(boot::IMAGE).expect("the image loads"),
    }
}

fn show(forms: &[Value]) -> String {
    forms
        .iter()
        .map(|f| print_value(f, true))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the Rust reader makes of src: the forms, printed, or its message.
fn by_rust(src: &str) -> String {
    match parse(src) {
        Ok(forms) => show(&forms),
        Err(LispError::Parse(m)) => format!("error: {m}"),
        Err(e) => format!("unexpected error: {e}"),
    }
}

/// The same, by the EELisp reader. Its errors are `(error msg)`, so a runtime error with the
/// same message.
fn by_eelisp(it: &Interpreter, src: &str) -> String {
    match it
        .call("read-all", &[Value::Str(src.to_string())])
        .expect("read-all is defined")
    {
        Ok(Value::List(forms)) => show(&forms),
        Ok(other) => format!("not a list: {}", print_value(&other, true)),
        Err(LispError::Runtime(m)) => format!("error: {m}"),
        Err(e) => format!("unexpected error: {e}"),
    }
}

fn agree(it: &Interpreter, setup: Setup, what: &str, src: &str) {
    let (rust, eel) = (by_rust(src), by_eelisp(it, src));
    if rust != eel {
        let line = rust.lines().zip(eel.lines()).position(|(a, b)| a != b);
        panic!(
            "{setup:?}: the readers disagree on {what} (first differing line {line:?})\n--- source\n{}\n--- rust\n{}\n--- eelisp\n{}",
            &src[..src.len().min(400)],
            &rust[..rust.len().min(800)],
            &eel[..eel.len().min(800)]
        );
    }
}

const EDGES: &[&str] = &[
    "",
    "   \n\t ",
    "; only a comment",
    "1 ; trailing comment",
    "(a b . c)",
    "[1 [2 3] ()]",
    "{:a 1 \"b c\" 2 :a 3}",
    "{}",
    "'x `(a ,b ,@c) ''y",
    ", ,@",
    "a'b a;b\n\"a\"b a`b a,b",
    ":",
    ":a:b ::x",
    "true false nil truex Nil",
    // numbers: decimal, as Rust's f64 parser takes them
    "1 -1 +1 1.5 .5 5. -.5 1e3 1E-3 1e400 -1e400 1.2.3 1e 1e+",
    "inf -inf +inf Infinity INF nan NaN -nan infx nanx",
    "- + . .. ... -x +x ->string -> -0 +0 0",
    // hex, including what Rust's integer parser allows after the 0x
    "0x10 0XfF -0x10 0x 0xg1 0x1234567890abc 0x1234567890abcd -0x0",
    "0x+ff 0x-ff -0x-ff 0x+ 0x- 0x+-1",
    // strings and escapes
    r#""a\nb\tc\rd\\e\"f" "\d" "\q" "é" "#,
    r#""ends with backslash \"#,
    "\"unterminated",
    "\"\\",
    // Unicode whitespace separates atoms
    "a\u{00a0}b\u{2003}c\u{3000}d",
    "\u{feff}a",
    // errors
    "(",
    "(a (b)",
    "[1 2",
    "{:a",
    "{:a 1",
    "{1 2}",
    "{:a ]}",
    "(a ]",
    "'",
    "`(a ,",
    // stray closers at the top level are skipped, but not inside a form
    ") ] } a ) (b)",
    "(a ))) b",
    // a lexing error later in the text wins over a parse error earlier, as in Rust
    "(a ] \"unterminated",
    "{1 2} \"x",
];

/// The repository's own files: whatever they are, both readers must make the same of them.
fn corpus() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in [
        "src",
        "boot",
        "tests",
        "examples",
        "docs",
        "docs/self-hosting",
        "docs/plans",
    ] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "eel" | "image" | "rs" | "md") {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    out.push((p.strip_prefix(root).unwrap().display().to_string(), text));
                }
            }
        }
    }
    out
}

/// Random texts over the characters a reader cares about. A fixed seed, so a failure repeats.
fn fuzz(count: usize) -> Vec<String> {
    const PIECES: &[&str] = &[
        "(", ")", "[", "]", "{", "}", "'", "`", ",", ",@", "\"", "\\", ";", "\n", " ", "\t", ":",
        "a", "b", "x1", "-", "+", ".", "0", "1", "9", "e", "0x", "f", "inf", "nan", "true", "nil",
        "é", "\u{a0}",
    ];
    let mut seed: u64 = 0x5eed_eeee_1234_5678;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    (0..count)
        .map(|_| {
            let len = next() % 24;
            (0..len)
                .map(|_| PIECES[next() % PIECES.len()])
                .collect::<String>()
        })
        .collect()
}

fn run(setup: Setup) {
    let it = engine(setup);
    for (i, src) in EDGES.iter().enumerate() {
        agree(&it, setup, &format!("edge case {i}"), src);
    }
    for (i, src) in fuzz(1500).iter().enumerate() {
        agree(&it, setup, &format!("fuzz case {i}"), src);
    }
    let files = corpus();
    assert!(
        files.len() > 40,
        "the corpus found only {} files",
        files.len()
    );
    for (name, text) in &files {
        agree(&it, setup, name, text);
    }
}

#[test]
fn the_eelisp_reader_agrees_with_the_rust_one_interpreted() {
    run(Setup::Interpreted);
}

#[test]
fn the_eelisp_reader_agrees_with_the_rust_one_from_the_image() {
    run(Setup::FromImage);
}

#[test]
fn read_all_makes_ordinary_values() {
    let it = engine(Setup::FromImage);
    let src = r#"(defn f (x) [x :k {"a b" 1}]) "s\n" 0x1F 'q"#;
    let read = |s: &str| match it
        .call("read-all", &[Value::Str(s.into())])
        .unwrap()
        .unwrap()
    {
        Value::List(l) => l.to_vec(),
        other => panic!("{other:?}"),
    };
    let forms = read(src);
    assert_eq!(forms, parse(src).unwrap());
    // symbols and keywords made by the reader are the same values the Rust reader makes
    assert!(
        matches!(&forms[0], Value::List(l) if matches!(&l[0], Value::Symbol(s) if &**s == "defn"))
    );
    // and what it reads, the compiler compiles: read by EELisp, compiled by EELisp, run on the VM
    let mut last = Value::Null;
    for form in read("(defn sq (x) (* x x)) (sq 12)") {
        last = it.call("compile-run", &[form]).unwrap().unwrap();
    }
    assert_eq!(print_value(&last, true), "144");
}
