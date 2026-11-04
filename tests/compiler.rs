//! The EELisp compiler (src/compiler.eel, roadmap step 2), checked against the interpreter.
//!
//! The corpus is the language test suite itself: every snippet `tests/lang.rs` evaluates (the
//! string literals passed to `s(&i, …)` and `i.eval_str(…)`) is read out of that file, and each
//! test function's snippets run in order twice, on two fresh engines. One interprets them; the
//! other compiles each top-level form and runs the bytecode. They must agree on every result
//! and every error.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use eelisp::parser::parse;
use eelisp::printer::print_value;
use eelisp::{Interpreter, Value};

const COMPILER: &str = include_str!("../src/compiler.eel");

fn compiling_engine() -> Interpreter {
    let it = Interpreter::new();
    it.eval_all(COMPILER).expect("the compiler loads");
    it
}

/// Recompile `src`'s definitions with the compiler already loaded in `it`.
fn recompile(it: &Interpreter, src: &str) {
    for form in parse(src).unwrap() {
        it.call("compile-run", &[form]).unwrap().unwrap();
    }
}

/// How the compiling engine is set up before a run.
#[derive(Clone, Copy)]
enum Setup {
    /// The interpreter runs the compiler; the prelude is interpreted.
    Plain,
    /// The prelude is compiled too.
    CompiledPrelude,
    /// The compiler has compiled itself: the code doing the compiling is bytecode on the VM.
    SelfCompiled,
}

impl Setup {
    fn name(self) -> &'static str {
        match self {
            Setup::Plain => "interpreted compiler",
            Setup::CompiledPrelude => "compiled prelude",
            Setup::SelfCompiled => "self-compiled compiler",
        }
    }
}

fn engine(setup: Setup) -> Interpreter {
    let it = compiling_engine();
    match setup {
        Setup::Plain => {}
        Setup::CompiledPrelude => recompile(&it, eelisp::prelude::PRELUDE),
        Setup::SelfCompiled => recompile(&it, COMPILER),
    }
    // count only what the test's own snippets leave to the interpreter
    it.eval_str("(set! compiler-fallbacks 0)").unwrap();
    it
}

/// A fresh, empty folder for one engine's files (sheets are created next to the "current" file).
fn scratch(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("eelisp-compiler-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An engine whose current folder is `dir`.
fn in_dir(it: Interpreter, dir: &Path) -> Interpreter {
    let root = dir.display().to_string();
    it.editor.borrow_mut().current_dir = Some(Box::new(move || root.clone()));
    it
}

/// A result as text, with the engine's folder written as `<dir>` so two engines compare.
fn outcome(r: Result<Value, eelisp::LispError>, dir: &Path) -> String {
    let text = match r {
        Ok(v) => print_value(&v, true),
        Err(e) => format!("error: {e}"),
    };
    text.replace(&dir.display().to_string(), "<dir>")
}

/// Every form of `src`, compiled and run; the last one's value.
fn compile_run(it: &Interpreter, src: &str) -> Result<Value, eelisp::LispError> {
    let mut last = Value::Null;
    for form in parse(src)? {
        last = it.call("compile-run", &[form]).expect("compile-run is defined")?;
    }
    Ok(last)
}

/// Read a Rust string literal (`"…"` with escapes, or `r#"…"#`) at the start of `s`.
fn rust_literal(s: &str) -> Option<String> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('r') {
        let hashes = rest.chars().take_while(|&c| c == '#').count();
        let body = rest[hashes..].strip_prefix('"')?;
        let close = format!("\"{}", "#".repeat(hashes));
        return body.find(&close).map(|end| body[..end].to_string());
    }
    let mut chars = s.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                '\n' => {
                    // a line continuation swallows the next line's indentation
                    while chars.clone().next().is_some_and(char::is_whitespace) {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// Each `#[test] fn`'s name and the snippets it evaluates, in order.
fn corpus(source: &str) -> Vec<(String, Vec<String>)> {
    let mut tests = Vec::new();
    for chunk in source.split("\nfn ").skip(1) {
        let name = chunk.split('(').next().unwrap_or("").trim().to_string();
        let mut snippets = Vec::new();
        let mut rest = chunk;
        loop {
            // `s(&it, "…")`, `ev(&it, "…")`, `err(&it, "…")` and `it.eval_str("…")`
            let helper = ["s(&", "ev(&", "err(&"]
                .iter()
                .filter_map(|h| rest.find(h))
                .filter_map(|at| rest[at..].find(',').map(|c| at + c + 1))
                .min();
            let method = rest.find(".eval_str(").map(|at| at + ".eval_str(".len());
            let Some(at) = [helper, method].into_iter().flatten().min() else { break };
            if let Some(lit) = rust_literal(&rest[at..]) {
                snippets.push(lit);
            }
            rest = &rest[at..];
        }
        if !snippets.is_empty() {
            tests.push((name, snippets));
        }
    }
    tests
}

#[test]
fn the_corpus_is_read() {
    let c = corpus(include_str!("lang.rs"));
    let snippets: usize = c.iter().map(|(_, s)| s.len()).sum();
    assert!(c.len() >= 20 && snippets >= 100, "{} tests, {snippets} snippets", c.len());
    assert!(c.iter().any(|(n, s)| n == "eval_def_defn_fn" && s.contains(&"(defn square (x) (* x x))".to_string())));
}

/// The suites whose snippets run on a plain `Interpreter::new()`. (Sheets need files on disk and
/// HTTP a server, so their snippets are built at run time and aren't literals anyway.)
const SUITES: &[(&str, &str)] = &[
    ("lang", include_str!("lang.rs")),
    ("book_gaps", include_str!("book_gaps.rs")),
    ("bytes", include_str!("bytes.rs")),
    ("csv", include_str!("csv.rs")),
    ("host", include_str!("host.rs")),
    ("agenda_advanced", include_str!("agenda_advanced.rs")),
    ("db_agenda", include_str!("db_agenda.rs")),
];

#[test]
fn compiled_code_agrees_with_the_interpreter_on_the_test_suites() {
    agree(Setup::Plain);
}

#[test]
fn with_the_prelude_compiled_too() {
    agree(Setup::CompiledPrelude);
}

#[test]
fn with_the_compiler_compiled_by_itself() {
    agree(Setup::SelfCompiled);
}

fn agree(setup: Setup) {
    let mut disagreements = Vec::new();
    let mut count = 0;
    let mut fallbacks = 0.0;
    for (test, snippets) in SUITES.iter().flat_map(|(suite, src)| {
        corpus(src).into_iter().map(move |(t, s)| (format!("{suite}::{t}"), s))
    }) {
        let (dir_i, dir_c) = (scratch("interp"), scratch("comp"));
        let interp = in_dir(Interpreter::new(), &dir_i);
        let comp = in_dir(engine(setup), &dir_c);
        for src in &snippets {
            count += 1;
            let want = outcome(interp.eval_str(src), &dir_i);
            let got = outcome(compile_run(&comp, src), &dir_c);
            if want != got {
                disagreements.push(format!("{test}: {src}\n    interpreter: {want}\n    compiled:    {got}"));
            }
        }
        if let Ok(Value::Number(n)) = comp.eval_str("compiler-fallbacks") {
            fallbacks += n;
        }
        let _ = std::fs::remove_dir_all(&dir_i);
        let _ = std::fs::remove_dir_all(&dir_c);
    }
    println!("{}: {count} snippets compiled; {fallbacks} forms left to the interpreter", setup.name());
    assert!(disagreements.is_empty(), "{} of {count} disagree:\n{}", disagreements.len(), disagreements.join("\n"));
}

#[test]
fn compile_returns_bytecode_a_person_can_read() {
    let it = compiling_engine();
    let p = print_value(&it.eval_str("(compile '(if ok 1 2))").unwrap(), true);
    assert_eq!(
        p,
        "(proto :name nil :params () :source ((if ok 1 2)) :code ((load ok) (jump-if-false 1) (const 1) (jump 2) (label 1) (const 2) (label 2)))"
    );
}

#[test]
fn compiled_definitions_are_compiled_functions() {
    let it = compiling_engine();
    compile_run(&it, "(defn down (n) (if (= n 0) \"done\" (down (- n 1))))").unwrap();
    // a tail call a million deep: the VM's frames, not the Rust stack
    assert_eq!(print_value(&compile_run(&it, "(down 1000000)").unwrap(), false), "done");
    // it keeps its source: `(source down)` rebuilds the definition from it
    let src = print_value(&it.eval_str("(source-text 'down)").unwrap(), false);
    assert!(src.contains("(defn down (n)") && src.contains("(down (- n 1))"), "{src}");
}

#[test]
fn macros_expand_when_compiled() {
    let it = compiling_engine();
    assert_eq!(print_value(&compile_run(&it, "(unless false 1 2)").unwrap(), false), "2");
    // a macro defined by an earlier top-level form is known to the next one
    compile_run(&it, "(defmacro twice (x) `(do ,x ,x))").unwrap();
    compile_run(&it, "(def n 0)").unwrap();
    assert_eq!(print_value(&compile_run(&it, "(twice (set! n (+ n 1)))").unwrap(), false), "2");
}
