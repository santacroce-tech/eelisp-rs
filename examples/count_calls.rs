//! Which EELisp functions and builtins the front end calls, and how often: compiled mode reads
//! and compiles every top-level form of a directory's .eelisp files, and the counts are printed.
//!
//!     cargo run --release --features count-calls --example count_calls -- <dir> [top]
//!
//! Counting starts after the engine is up, so it is the reading and compiling alone.
use eelisp::{vm, Interpreter, Value};

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("usage: count_calls <dir> [top]");
    let top: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(40);
    let mut src = String::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "eelisp") {
            src += &std::fs::read_to_string(p).unwrap();
        }
    }
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    let text = Value::Str(src.as_str().into());
    vm::take_counts();
    let forms = it.call("read-all", std::slice::from_ref(&text)).unwrap().unwrap();
    let read = vm::take_counts();
    let Value::List(forms) = forms else { panic!("read-all gave no list") };
    for f in forms.iter() {
        it.call("compile", std::slice::from_ref(f)).unwrap().unwrap();
    }
    let compile = vm::take_counts();
    if read.is_empty() {
        eprintln!("nothing counted: build with --features count-calls");
        return;
    }
    for (what, counts) in [("reading", read), ("compiling", compile)] {
        let total: u64 = counts.iter().map(|c| c.1).sum();
        println!("── {what} {} forms: {total} calls ({:.0} per form)", forms.len(), total as f64 / forms.len() as f64);
        for (name, n) in counts.iter().take(top) {
            println!("{n:>9}  {:5.1}%  {name}", 100.0 * *n as f64 / total as f64);
        }
    }
}
