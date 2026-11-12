//! Regenerate `boot/compiler.image`: the EELisp compiler and reader, compiled by the compiler.
//!
//!     cargo run --example bootstrap                  # the committed image builds the next one
//!     cargo run --example bootstrap -- --from-source # the interpreter builds it from scratch
//!
//! By default it works the way self-hosted compilers build their next version. The committed
//! image (the previous compiler, as bytecode) reads and compiles the current source; that compiler
//! compiles the source again, and once more. The last two must be identical (a fixed point).
//! Then the cross-check: the interpreter, running the source directly (stage 0), must produce the
//! same image. Only then is the image written.
//!
//! `--from-source` starts from the interpreter instead, for when there is no usable image: a
//! fresh checkout with a broken one, say. See `src/boot.rs` and docs/self-hosting/10.

use std::time::Instant;

use eelisp::boot;

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/boot/compiler.image");

fn first_difference(a: &str, b: &str) -> Option<usize> {
    a.lines()
        .zip(b.lines())
        .position(|(x, y)| x != y)
        .map(|n| n + 1)
}

fn fail(msg: String) -> ! {
    eprintln!("{msg}\nnot writing the image");
    std::process::exit(1);
}

fn main() {
    let from_source = std::env::args().any(|a| a == "--from-source");
    let src = boot::source();

    // the seed: the committed image, or one made by the interpreter
    let t = Instant::now();
    let (seed, seed_name) = if from_source {
        let image = boot::stage0().unwrap_or_else(|e| fail(format!("stage 0 failed: {e}")));
        (image, "the interpreter")
    } else {
        let image = std::fs::read_to_string(PATH)
            .unwrap_or_else(|e| fail(format!("can't read {PATH}: {e} (try --from-source)")));
        (image, "the committed image")
    };

    let r = boot::rebuild(&seed, &src).unwrap_or_else(|e| {
        fail(format!("building from {seed_name} failed: {e}\n(if the committed image can't build the new source, try --from-source)"))
    });
    let t_rebuild = t.elapsed();

    // the cross-check: the interpreter and the Rust reader, which share nothing with the image
    let t = Instant::now();
    let check = boot::stage0().unwrap_or_else(|e| fail(format!("stage 0 failed: {e}")));
    let t_check = t.elapsed();
    if check != r.second {
        fail(format!(
            "the rebuilt image and the interpreter's disagree (first differing line {:?})",
            first_difference(&check, &r.second)
        ));
    }

    let old = std::fs::read_to_string(PATH).unwrap_or_default();
    std::fs::write(PATH, &r.second).expect("write boot/compiler.image");
    println!(
        "boot/compiler.image: {} protos, {} bytes{}.",
        r.second.lines().filter(|l| l.starts_with("(proto")).count(),
        r.second.len(),
        if old == r.second {
            ", unchanged"
        } else {
            ", updated"
        }
    );
    println!(
        "  built from {seed_name} in three generations ({t_rebuild:.2?}): {}; generations 2 and 3 identical",
        if r.first == r.second {
            "the first already identical"
        } else {
            "the first differs (the source changes what the compiler emits)"
        }
    );
    println!("  cross-check: the interpreter makes the same image ({t_check:.2?})");
}
