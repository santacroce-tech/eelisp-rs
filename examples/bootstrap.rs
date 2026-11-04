//! Regenerate `boot/compiler.image`: the EELisp compiler, compiled by itself.
//!
//!     cargo run --example bootstrap
//!
//! Stage 0 (the interpreter runs the compiler) produces image 1; stage 1 (the compiler loaded
//! from image 1, as bytecode) produces image 2. They must be identical before the image is
//! written. See `src/boot.rs`.

use std::time::Instant;

fn main() {
    let t = Instant::now();
    let image1 = eelisp::boot::stage0().unwrap_or_else(|e| panic!("stage 0 failed: {e}"));
    let t0 = t.elapsed();
    let t = Instant::now();
    let image2 = eelisp::boot::stage1(&image1).unwrap_or_else(|e| panic!("stage 1 failed: {e}"));
    let t1 = t.elapsed();
    if image1 != image2 {
        let line = image1.lines().zip(image2.lines()).position(|(a, b)| a != b);
        eprintln!("stage 0 and stage 1 disagree (first differing line: {line:?}); not writing the image");
        std::process::exit(1);
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/boot/compiler.image");
    std::fs::write(path, &image1).expect("write boot/compiler.image");
    println!(
        "boot/compiler.image: {} protos, {} bytes. Stage 0 (interpreted compiler) {t0:.2?}, stage 1 (compiled compiler) {t1:.2?}; identical.",
        image1.lines().filter(|l| l.starts_with("(proto")).count(),
        image1.len()
    );
}
