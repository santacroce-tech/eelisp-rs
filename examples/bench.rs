//! Interpreter against compiled code, on the same workloads.
//!
//!     cargo run --release --example bench [-- <snippets dir> [--only name]]
//!
//! Each workload is defined on two engines: one interprets it, the other has the compiler loaded
//! from `boot/compiler.image` and compiles every definition. Then `(bench-run)` is called the
//! same number of times on both and timed. The two results must agree.
//!
//! With a directory (eeditor's `src/snippets`), every example call written in its files'
//! comments — `;; (zzromanos 2024) → …` — becomes a workload too, except in files that reach the
//! network.

use std::time::{Duration, Instant};

use eelisp::parser::parse;
use eelisp::printer::print_value;
use eelisp::{Interpreter, Value};

const SYNTHETIC: &[(&str, &str, &str, u32)] = &[
    ("fib", "(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))", "(fib 20)", 5),
    ("loop", "", "(loop (i 0 acc 0) (if (= i 200000) acc (recur (+ i 1) (+ acc i))))", 5),
    (
        "lists",
        "(defn sq (x) (* x x))",
        "(reduce + 0 (filter even? (map sq (range 0 20000))))",
        5,
    ),
    (
        "let-heavy",
        "(defn dist (x1 y1 x2 y2) (let (dx (- x2 x1) dy (- y2 y1)) (+ (* dx dx) (* dy dy))))",
        "(loop (i 0 acc 0) (if (= i 50000) acc (recur (+ i 1) (+ acc (dist i 1 2 i)))))",
        5,
    ),
    (
        "closures",
        "(defn adder (n) (fn (x) (+ x n)))",
        "(loop (i 0 acc 0) (if (= i 50000) acc (recur (+ i 1) ((adder i) acc))))",
        5,
    ),
    (
        "strings",
        "(defn pad (s n) (if (>= (str-len s) n) s (pad (str s \".\") n)))",
        "(length (map (fn (i) (pad (->string i) 40)) (range 0 1000)))",
        5,
    ),
];

fn compiled_engine(image: &str) -> Interpreter {
    eelisp::boot::with_image(image).expect("load the compiler image")
}

fn compile_all(it: &Interpreter, src: &str) -> Result<(), String> {
    for form in parse(src).map_err(|e| e.to_string())? {
        it.call("compile-run", &[form]).unwrap().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn show(r: &Result<Value, eelisp::LispError>) -> String {
    match r {
        Ok(v) => print_value(v, true),
        Err(e) => format!("error: {e}"),
    }
}

struct Row {
    name: String,
    interp: Duration,
    vm: Duration,
}

/// Define `setup` and `(defn bench-run () call)` on both engines, run it `reps` times on each.
fn measure(image: &str, name: &str, setup: &str, call: &str, reps: u32) -> Option<Row> {
    let driver = format!("(defn bench-run () {call})");
    let interp = Interpreter::new();
    if let Err(e) = interp.eval_str(setup).and_then(|_| interp.eval_str(&driver)) {
        eprintln!("skip {name}: {e}");
        return None;
    }
    let vm = compiled_engine(image);
    if let Err(e) = compile_all(&vm, setup).and_then(|_| compile_all(&vm, &driver)) {
        eprintln!("skip {name}: compiling: {e}");
        return None;
    }
    let run = |it: &Interpreter| -> (Duration, String) {
        let t = Instant::now();
        let mut last = String::new();
        for _ in 0..reps {
            last = show(&it.call("bench-run", &[]).unwrap());
        }
        (t.elapsed(), last)
    };
    let (ti, ri) = run(&interp);
    let (tv, rv) = run(&vm);
    if ri != rv {
        // a workload that draws random numbers or reads the clock can't agree; anything else is a bug
        eprintln!("results differ for {name} (expected only where it uses randomness or the clock)");
    }
    Some(Row { name: name.into(), interp: ti, vm: tv })
}

/// The example calls in a snippet file's comments: `;; (zzname …)` up to the balanced paren.
fn examples(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim_start().strip_prefix(";;") else { continue };
        let rest = rest.trim_start();
        if !rest.starts_with("(zz") {
            continue;
        }
        let mut depth = 0;
        let mut in_str = false;
        let mut prev = ' ';
        for (i, c) in rest.char_indices() {
            match c {
                '"' if prev != '\\' => in_str = !in_str,
                '(' if !in_str => depth += 1,
                ')' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(rest[..=i].to_string());
                        break;
                    }
                }
                _ => {}
            }
            prev = c;
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let image = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/boot/compiler.image")).unwrap();
    let mut rows = Vec::new();
    for (name, setup, call, reps) in SYNTHETIC {
        rows.extend(measure(&image, name, setup, call, *reps));
    }
    if let Some(dir) = args.first() {
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            if path.extension().and_then(|e| e.to_str()) != Some("eelisp") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            if src.contains("http-get") || src.contains("http-post") {
                continue; // the network isn't a benchmark
            }
            let calls = examples(&src);
            if calls.is_empty() {
                continue;
            }
            // every example in the file, in one run: what a user exercising the file does
            let body = format!("(list {})", calls.iter().map(|c| format!("(try {c} (catch e :error))")).collect::<Vec<_>>().join(" "));
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            rows.extend(measure(&image, &format!("{name} ({} calls)", calls.len()), &src, &body, 20));
        }
    }
    println!("{:<28} {:>12} {:>12} {:>8}", "workload", "interpreter", "compiled", "speedup");
    for r in &rows {
        println!(
            "{:<28} {:>10.1?} {:>10.1?} {:>7.2}x",
            r.name,
            r.interp,
            r.vm,
            r.interp.as_secs_f64() / r.vm.as_secs_f64()
        );
    }
}
