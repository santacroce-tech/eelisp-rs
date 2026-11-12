//! The EELisp reader (`read-top`) in a loop over a directory's .eelisp files, for the sampler.
//!     cargo run --release --example prof_read -- <dir> [times] [small]
use eelisp::{Interpreter, Value};
fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().unwrap();
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let small = a.next().is_some();
    let mut all = String::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "eelisp") {
            all += &std::fs::read_to_string(p).unwrap();
        }
    }
    if small { all = "(+ 1 2)".into(); }
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    let s = Value::Str(all.as_str().into());
    for _ in 0..n { it.call("read-top", std::slice::from_ref(&s)).unwrap().unwrap(); }
}
