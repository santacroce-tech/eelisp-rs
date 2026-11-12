//! The pieces of `read-top` on a tiny form, timed 20,000 times each (compiled mode).
use std::time::Instant;
use eelisp::Interpreter;
fn main() {
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    it.eval_str(r#"(def src "(+ 1 2)") (def cs (str-chars src)) (def n (length cs))
                   (def tok (reader-token cs n 0))"#).unwrap();
    let k = 20000;
    for (name, e) in [
        ("nothing", "nil"),
        ("str-chars", "(str-chars src)"),
        ("reader-token", "(reader-token cs n 0)"),
        ("reader-form", "(reader-form tok cs n)"),
        ("reader-spans", "(reader-spans cs n)"),
        ("comments-above", "(reader-comments-above cs 0)"),
        ("trailing", "(reader-with-trailing-comment cs n 7)"),
        ("reader-text", "(reader-text cs 0 7)"),
        ("read-all", "(read-all src)"),
        ("read-top", "(read-top src)"),
    ] {
        let prog = format!("(loop (i 0) (if (< i {k}) (do {e} (recur (+ i 1))) nil))");
        let t = Instant::now();
        it.eval_str(&prog).unwrap();
        println!("{name:<16} {:7.2} µs", t.elapsed().as_secs_f64() * 1e6 / k as f64);
    }
}
