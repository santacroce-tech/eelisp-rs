//! Run one call-benchmark program repeatedly, for a sampling profiler.
//!     cargo run --release --example prof_one -- "(mapped 100000)" 50 [definitions.eel]
use eelisp::Interpreter;
fn main() {
    let mut a = std::env::args().skip(1);
    let src = a.next().unwrap_or_else(|| "(mapped 100000)".into());
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(50);
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    it.eval_str(r#"
(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
(defn mapped (k) (length (filter (fn (x) (> x 10)) (map (fn (x) (* x 3)) (range 0 k)))))
"#).unwrap();
    if let Some(path) = a.next() {
        it.eval_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    }
    for _ in 0..n { it.eval_str(&src).unwrap(); }
}
