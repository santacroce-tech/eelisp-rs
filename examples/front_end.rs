//! Where a compiled evaluation's time goes: reading (EELisp `read-top`), compiling
//! (`compile`), assembling, running. Compiled mode, release.
//!
//!     cargo run --release --example front_end [-- <snippets dir>]
use std::time::Instant;

use eelisp::{Interpreter, Value};

fn per<F: FnMut()>(n: u32, mut f: F) -> f64 {
    f();
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed().as_secs_f64() * 1e6 / n as f64
}

fn main() {
    let t = Instant::now();
    let it = Interpreter::new();
    let new = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    it.set_compiled(true).unwrap();
    let on = t.elapsed().as_secs_f64() * 1e3;
    println!("Interpreter::new {new:.2} ms; set_compiled(true) {on:.2} ms");

    let texts = [
        ("(+ 1 2)", "(+ 1 2)".to_string()),
        ("small defn", "(defn sq (x) (* x x))".to_string()),
        ("let + map", "(let (xs (range 0 10)) (map (fn (x) (* x x)) xs))".to_string()),
    ];
    let dir = std::env::args().nth(1);
    let mut texts: Vec<(&str, String)> = texts.to_vec();
    if let Some(d) = &dir {
        let mut all = String::new();
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "eelisp") {
                all += &std::fs::read_to_string(p).unwrap();
                all += "\n";
            }
        }
        texts.push(("snippet files", all));
    }
    println!("{:<14} {:>10} {:>10} {:>10} {:>10} {:>10}", "", "read", "compile", "assemble", "run", "eval_str");
    for (name, src) in &texts {
        let n = if src.len() > 1000 { 5 } else { 2000 };
        let s = Value::Str(src.as_str().into());
        let forms = it.call("read-top", std::slice::from_ref(&s)).unwrap().unwrap();
        let Value::List(forms) = forms else { panic!() };
        let forms: Vec<Value> = forms.iter().map(|f| match f { Value::List(t) => t[0].clone(), _ => panic!() }).collect();
        let read = per(n, || { it.call("read-top", std::slice::from_ref(&s)).unwrap().unwrap(); });
        let protos: Vec<Value> = forms.iter().map(|f| it.call("compile", std::slice::from_ref(f)).unwrap().unwrap()).collect();
        let comp = per(n, || { for f in &forms { it.call("compile", std::slice::from_ref(f)).unwrap().unwrap(); } });
        let asm = per(n, || { for p in &protos { eelisp::vm::assemble(p).unwrap(); } });
        let eval = if src.len() > 1000 { f64::NAN } else { per(n, || { it.eval_str(src).unwrap(); }) };
        let run = eval - read - comp - asm;
        let f = |us: f64| if us.is_nan() { "-".to_string() } else if us > 1000.0 { format!("{:.2} ms", us / 1000.0) } else { format!("{us:.1} µs") };
        println!("{name:<14} {:>10} {:>10} {:>10} {:>10} {:>10}", f(read), f(comp), f(asm), f(run), f(eval));
    }
}
