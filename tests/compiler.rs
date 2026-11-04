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
    /// The compiler comes from the committed image (boot/compiler.image) alone; its source is
    /// never interpreted.
    FromImage,
}

impl Setup {
    fn name(self) -> &'static str {
        match self {
            Setup::Plain => "interpreted compiler",
            Setup::CompiledPrelude => "compiled prelude",
            Setup::SelfCompiled => "self-compiled compiler",
            Setup::FromImage => "compiler from the image",
        }
    }
}

fn engine(setup: Setup) -> Interpreter {
    if let Setup::FromImage = setup {
        let image = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/boot/compiler.image")).unwrap();
        let it = eelisp::boot::with_image(&image).unwrap();
        it.eval_str("(set! compiler-fallbacks 0)").unwrap();
        return it;
    }
    let it = compiling_engine();
    match setup {
        Setup::Plain => {}
        Setup::CompiledPrelude => recompile(&it, eelisp::prelude::PRELUDE),
        Setup::SelfCompiled => recompile(&it, COMPILER),
        Setup::FromImage => unreachable!(),
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

#[test]
fn with_the_compiler_loaded_from_the_image() {
    agree(Setup::FromImage);
}

/// Cases the suites barely touch: quasiquote, for-each and defmacro, edges included. Run in
/// order on one engine pair, like a test function's snippets.
const EDGES: &[&str] = &[
    // quasiquote
    "(def x 1) (def xs '(2 3))",
    "`(a ,x ,@xs b)",
    "`(a (b ,x) ,@xs)",
    "`()",
    "`x",
    "`,x",
    "`(1 ,@2)",
    "`(1 ,@nil)",
    "`{:a ,x}",
    "`(outer `(inner ,x))",
    "(quasiquote)",
    "(let (y 5) `(y is ,y))",
    // for-each
    "(def acc 0)",
    "(for-each n '(1 2 3) (set! acc (+ acc n)))",
    "acc",
    "(for-each n 5 (error \"not a list: never runs\"))",
    "(for-each 1 (error \"not reached\") 2)",
    "(for-each n '(1 2))",
    "n",
    "(for-each n '(1 2 3) (def seen n) (set! acc (* acc 10)))",
    "(list acc seen)",
    "(try (for-each n '(1 2 3) (if (= n 2) (error \"two\") n)) (catch e (dict-get e :message)))",
    "(loop (i 0 total 0) (if (< i 3) (do (for-each k '(1 2) (set! total (+ total k))) (recur (+ i 1) total)) total))",
    "(defn sum-list (l) (let (t 0) (for-each v l (set! t (+ t v))) t))",
    "(sum-list (range 0 100))",
    // defmacro
    "(defmacro my-if (c a b) `(cond ,c ,a true ,b))",
    "(my-if false 1 2)",
    "(my-if (= 1 1) \"yes\" (error \"not evaluated\"))",
    "(defmacro my-list (. items) `(list ,@items))",
    "(my-list 1 (+ 1 1) 3)",
    "(defn uses-macro (v) (my-if v :t :f))",
    "(list (uses-macro true) (uses-macro nil))",
    "(defmacro swap! (a b) `(let (tmp ,a) (set! ,a ,b) (set! ,b tmp)))",
    "(def p 1) (def q 2) (swap! p q) (list p q)",
    "(defmacro)",
    "(defmacro bad)",
    "my-if",
    // locals in slots, closures sharing them
    "(defn make-counter () (let (n 0) (fn () (set! n (+ n 1)) n)))",
    "(def c1 (make-counter)) (def c2 (make-counter)) (c1) (c1) (list (c1) (c2))",
    "(defn adders () (map (fn (i) (fn (x) (+ x i))) (range 0 3)))",
    "(map (fn (f) (f 10)) (adders))",
    "(defn lets-in-loop () (loop (i 0 fs (list)) (if (= i 3) (map (fn (f) (f)) fs) (let (j (* i 10)) (recur (+ i 1) (cons (fn () j) fs))))))",
    "(lets-in-loop)",
    "(defn loop-var-shared () (loop (i 0 f nil) (if (= i 3) (f) (recur (+ i 1) (if (nil? f) (fn () i) f)))))",
    "(loop-var-shared)",
    "(defn items-captured () (let (fs (list)) (for-each k '(1 2 3) (set! fs (cons (fn () k) fs))) (map (fn (f) (f)) fs)))",
    "(items-captured)",
    "(defn three-levels (a) (fn (b) (fn (c) (list a b c))))",
    "(((three-levels 1) 2) 3)",
    "(defn set-outer () (let (x 1) (let (g (fn () (set! x (+ x 10)))) (g) (g) x)))",
    "(set-outer)",
    "(defn local-def () (def y 5) (def y (+ y 1)) y)",
    "(local-def)",
    "y",
    "(def z \"global z\")",
    "(defn maybe-def (flag) (if flag (def z \"local z\")) z)",
    "(list (maybe-def true) (maybe-def false))",
    "(defn local-rec (n) (defn helper (k acc) (if (= k 0) acc (helper (- k 1) (* acc k)))) (helper n 1))",
    "(local-rec 10)",
    "(defn local-fn-def () (def again (fn (k) (if (= k 0) :done (again (- k 1))))) (again 5))",
    "(local-fn-def)",
    "(defn shadow (x) (let (x (* x 2)) (let (x (+ x 1)) x)))",
    "(shadow 5)",
    "(defn rest-slots (a . more) (list a more (length more)))",
    "(list (rest-slots 1) (rest-slots 1 2 3))",
    "(defn missing-args (a b c) (list a b c))",
    "(missing-args 1)",
    "(defn set-global () (set! z \"set from a function\") z)",
    "(list (set-global) z)",
    "(defn catch-binds () (try (error \"inner\") (catch e (fn () (dict-get e :message)))))",
    "((catch-binds))",
    "(defn defs-each-iteration () (loop (i 0 out (list)) (if (= i 2) out (let () (if (= i 0) (def w :first)) (recur (+ i 1) (cons (try w (catch e :unbound)) out))))))",
    "(defs-each-iteration)",
    "(defn eval-sees-globals (z) (eval 'z))",
    "(eval-sees-globals \"a local\")",
];

fn agree(setup: Setup) {
    let mut disagreements = Vec::new();
    let mut count = 0;
    let mut fallbacks = 0.0;
    let edges = ("edges".to_string(), EDGES.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    for (test, snippets) in SUITES
        .iter()
        .flat_map(|(suite, src)| corpus(src).into_iter().map(move |(t, s)| (format!("{suite}::{t}"), s)))
        .chain(std::iter::once(edges))
    {
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
        "(proto :name nil :params () :source ((if ok 1 2)) :slots 0 :captures () :code ((load ok) (jump-if-false 1) (const 1) (jump 2) (label 1) (const 2) (label 2)))"
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
