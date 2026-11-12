//! How the EELisp reader's time grows with the number of top-level forms (compiled mode).
//!     cargo run --release --example read_scale
use std::time::Instant;
use eelisp::Interpreter;
fn main() {
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    for n in [250, 500, 1000, 2000, 4000] {
        let src: String = (0..n).map(|i| format!("(def v{i} {i})\n")).collect();
        let list: String = format!("(quote ({}))", (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(" "));
        let t = Instant::now();
        it.eval_str(&format!("(length (read-all {:?}))", src)).unwrap();
        let top = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        it.eval_str(&format!("(length (read-all {:?}))", list)).unwrap();
        let inner = t.elapsed().as_secs_f64() * 1000.0;
        println!("{n:>5} forms {top:8.1} ms    one list of {n:>5} {inner:8.1} ms");
    }
}
