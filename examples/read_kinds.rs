//! The EELisp reader on one kind of text at a time, compiled mode: cost per character and per
//! token, to see where reading's time goes.
use std::time::Instant;
use eelisp::{Interpreter, Value};
fn main() {
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    let k = 2000;
    let kinds: Vec<(&str, String, usize)> = vec![
        ("comment lines", (0..k).map(|_| ";; a comment of some forty characters..\n").collect(), 0),
        ("whitespace", " ".repeat(40 * k), 0),
        ("symbols", format!("({})", (0..k).map(|i| format!("sym-{i}")).collect::<Vec<_>>().join(" ")), k),
        ("numbers", format!("({})", (0..k).map(|i| i.to_string()).collect::<Vec<_>>().join(" ")), k),
        ("strings", format!("({})", (0..k).map(|i| format!("\"text {i}\"")).collect::<Vec<_>>().join(" ")), k),
        ("small lists", (0..k).map(|i| format!("(f {i})\n")).collect(), 4 * k),
    ];
    for (name, src, tokens) in kinds {
        let s = Value::Str(src.as_str().into());
        for f in ["read-all", "read-top"] {
            it.call(f, std::slice::from_ref(&s)).unwrap().unwrap();
            let t = Instant::now();
            for _ in 0..5 { it.call(f, std::slice::from_ref(&s)).unwrap().unwrap(); }
            let us = t.elapsed().as_secs_f64() * 1e6 / 5.0;
            let per_tok = if tokens > 0 { format!("{:7.2} µs/token", us / tokens as f64) } else { String::new() };
            println!("{name:<14} {f:<9} {:7.1} ns/char {per_tok}", us * 1000.0 / src.chars().count() as f64);
        }
    }
}
