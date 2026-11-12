//! Compile a directory's top-level forms over and over, for a sampling profiler, or to time
//! compiling alone.
//!     cargo run --release --example prof_compile -- <dir> [times]
use std::time::Instant;

use eelisp::{Interpreter, Value};

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("usage: prof_compile <dir> [times]");
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut src = String::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "eelisp") {
            src += &std::fs::read_to_string(p).unwrap();
        }
    }
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    let Some(Ok(Value::List(forms))) = it.call("read-all", &[Value::Str(src.as_str().into())]) else { panic!("read-all") };
    let mut best = f64::MAX;
    for _ in 0..n {
        let t = Instant::now();
        for f in forms.iter() {
            it.call("compile", std::slice::from_ref(f)).unwrap().unwrap();
        }
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("compile {} forms: {best:.2} ms (best of {n})", forms.len());
}
