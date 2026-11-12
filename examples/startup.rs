//! What starting an engine in compiled mode costs, piece by piece (release).
use std::time::Instant;
use eelisp::{boot, parser, Interpreter};
fn ms(t: Instant) -> f64 { t.elapsed().as_secs_f64() * 1e3 }
fn main() {
    for round in 0..3 {
        let t = Instant::now(); let it = Interpreter::new(); let new = ms(t);
        let t = Instant::now(); let forms = parser::parse(boot::IMAGE).unwrap(); let parse = ms(t);
        let t = Instant::now(); boot::load_image(&it, boot::IMAGE).unwrap(); let load = ms(t);
        let t = Instant::now(); it.set_compiled(true).unwrap(); let on = ms(t);
        let t = Instant::now(); let it2 = Interpreter::new(); it2.set_compiled(true).unwrap(); let both = ms(t);
        println!("round {round}: new {new:.2}  parse image {parse:.2} ({} forms)  load image {load:.2}  set_compiled after load {on:.2}  new+set_compiled {both:.2} ms", forms.len());
    }
}
