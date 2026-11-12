//! The same `fib` as register code, one difference at a time, to find what the compiler's output
//! costs against hand-written code.
//!
//!     cargo run --release --example fib_variants
use std::time::Instant;

use eelisp::Interpreter;

const VARIANTS: &[(&str, &str)] = &[
    (
        "compiled (push + return)",
        "(call2 (t 0) < (local 0 n) (const 2)) (jump-if-not (t 0) 1) (return-value (local 0 n)) (jump 2) (label 1)
         (call2 (t 0) - (local 0 n) (const 1)) (call-global (t 0) F (t 0) 1)
         (call2 (t 1) - (local 0 n) (const 2)) (call-global (t 1) F (t 1) 1)
         (call2 push + (t 0) (t 1)) (label 2)",
    ),
    (
        "return-value of a register",
        "(call2 (t 0) < (local 0 n) (const 2)) (jump-if-not (t 0) 1) (return-value (local 0 n)) (label 1)
         (call2 (t 0) - (local 0 n) (const 1)) (call-global (t 0) F (t 0) 1)
         (call2 (t 1) - (local 0 n) (const 2)) (call-global (t 1) F (t 1) 1)
         (call2 (t 0) + (t 0) (t 1)) (return-value (t 0))",
    ),
    (
        "push + explicit return",
        "(call2 (t 0) < (local 0 n) (const 2)) (jump-if-not (t 0) 1) (return-value (local 0 n)) (label 1)
         (call2 (t 0) - (local 0 n) (const 1)) (call-global (t 0) F (t 0) 1)
         (call2 (t 1) - (local 0 n) (const 2)) (call-global (t 1) F (t 1) 1)
         (call2 push + (t 0) (t 1)) (return)",
    ),
];

fn main() {
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    for (i, (name, code)) in VARIANTS.iter().enumerate() {
        let f = format!("fib{i}");
        let src = format!(
            "(def {f} (vm-assemble '(proto :name \"{f}\" :params (n) :slots 1 :temps 2 :code ({}))))",
            code.replace('F', &f)
        );
        it.eval_str(&src).unwrap();
        let call = format!("({f} 25)");
        let ms = (0..7)
            .map(|_| {
                let t = Instant::now();
                assert_eq!(eelisp::printer::print_value(&it.eval_str(&call).unwrap(), false), "75025");
                t.elapsed().as_secs_f64() * 1e3
            })
            .fold(f64::MAX, f64::min);
        println!("{name:<30} {ms:6.1} ms");
    }
    it.eval_str("(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))").unwrap();
    let ms = (0..7)
        .map(|_| {
            let t = Instant::now();
            it.eval_str("(fib 25)").unwrap();
            t.elapsed().as_secs_f64() * 1e3
        })
        .fold(f64::MAX, f64::min);
    println!("{:<30} {ms:6.1} ms", "fib from defn");
}
