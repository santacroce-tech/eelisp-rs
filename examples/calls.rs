//! Call-heavy workloads for the VM, compiled mode: what the VM redesign is measured with
//! (docs/self-hosting/14-reshaping-the-vm.md).
//!
//!     cargo run --release --example calls
//!
//! Each line is the best of five runs. To compare two commits fairly on a noisy machine, use
//! `scripts/ab.sh <commit-a> <commit-b>`, which builds both and alternates their runs.

use std::time::Instant;

use eelisp::Interpreter;

const PROGRAMS: &str = r#"
(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
(defn make-adder (n) (fn (x) (+ x n)))
(defn clo (k) (loop (i 0 acc 0) (if (< i k) (recur (+ i 1) ((make-adder i) acc)) acc)))
(defn sq (x) (* x x))
(defn calls (k) (loop (i 0 acc 0) (if (< i k) (recur (+ i 1) (+ acc (sq i))) acc)))
(defn count-down (n acc) (if (= n 0) acc (count-down (- n 1) (+ acc 1))))
(defn sum-to (k) (loop (i 0 acc 0) (if (= i k) acc (recur (+ i 1) (+ acc i)))))
(defn mapped (k) (length (filter (fn (x) (> x 10)) (map (fn (x) (* x 3)) (range 0 k)))))
"#;

const RUNS: &[(&str, &str)] = &[
    ("fib 25", "(fib 25)"),
    ("closures 200k", "(clo 200000)"),
    ("calls 300k", "(calls 300000)"),
    ("tail calls 300k", "(count-down 300000 0)"),
    ("loop 1M", "(sum-to 1000000)"),
    ("map/filter 100k", "(mapped 100000)"),
];

fn best<F: FnMut()>(mut f: F) -> f64 {
    (0..5)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .fold(f64::MAX, f64::min)
}

fn main() {
    let it = Interpreter::new();
    it.set_compiled(true).expect("compiled mode");
    it.eval_str(PROGRAMS).expect("the programs load");
    for (name, src) in RUNS {
        let ms = best(|| {
            it.eval_str(src).expect("runs");
        });
        println!("{name:<18} {ms:8.1} ms");
    }
}
