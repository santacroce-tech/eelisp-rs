//! The bootstrap (roadmap step 3, src/boot.rs): the compiler compiles itself to a fixed point,
//! and the committed image is that fixed point.

use eelisp::boot;
use eelisp::parser::parse;
use eelisp::printer::print_value;

fn committed() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/boot/compiler.image");
    std::fs::read_to_string(path).expect("boot/compiler.image exists (cargo run --example bootstrap)")
}

#[test]
fn the_committed_image_is_what_the_compiler_makes_of_itself() {
    let image1 = boot::stage0().unwrap();
    assert!(
        image1 == committed(),
        "boot/compiler.image is out of date with src/compiler.eel: run `cargo run --example bootstrap`"
    );
}

#[test]
fn the_compiled_compiler_compiles_itself_to_the_same_image() {
    // stage 1 from the committed image: the compiler's source is never interpreted
    let image = committed();
    let image2 = boot::stage1(&image).unwrap();
    if image2 != image {
        let (n, (a, b)) = image.lines().zip(image2.lines()).enumerate().find(|(_, (a, b))| a != b).unwrap();
        panic!("not a fixed point; line {}:\n  image:   {a}\n  stage 1: {b}", n + 1);
    }
}

#[test]
fn read_by_the_eelisp_reader_too_the_source_compiles_to_the_same_image() {
    // stage 2: the EELisp reader reads the source and the EELisp compiler compiles it; Rust only
    // runs the VM and the builtins
    let image = committed();
    let image3 = boot::stage2(&image).unwrap();
    if image3 != image {
        let (n, (a, b)) = image.lines().zip(image3.lines()).enumerate().find(|(_, (a, b))| a != b).unwrap();
        panic!("stage 2 differs; line {}:\n  image:   {a}\n  stage 2: {b}", n + 1);
    }
}

#[test]
fn an_image_reads_back_as_itself() {
    // the image is EELisp text: parsing and printing each proto gives the same text
    let image = committed();
    let protos: Vec<&str> = image.lines().filter(|l| !l.starts_with(";;")).collect();
    let parsed = parse(&image).unwrap();
    assert_eq!(parsed.len(), protos.len());
    assert_eq!(parsed.len(), parse(&boot::source()).unwrap().len(), "one proto per top-level form");
    for (line, form) in protos.iter().zip(&parsed) {
        assert_eq!(&print_value(form, true), line);
    }
}

#[test]
fn nothing_in_the_compiler_is_left_to_the_interpreter() {
    assert!(!committed().contains("(interp "));
}

#[test]
fn an_engine_started_from_the_image_compiles_and_runs_programs() {
    let it = boot::with_image(&committed()).unwrap();
    let run = |src: &str| {
        let mut last = eelisp::Value::Null;
        for form in parse(src).unwrap() {
            last = it.call("compile-run", &[form]).unwrap().unwrap();
        }
        print_value(&last, false)
    };
    run("(defn fact (n) (if (<= n 1) 1 (* n (fact (- n 1)))))");
    assert_eq!(run("(fact 20)"), "2432902008176640000");
    assert_eq!(run("(defmacro unless2 (c . body) `(if ,c nil (do ,@body))) (unless2 false :ran)"), ":ran");
    assert_eq!(run("(let (t 0) (for-each x (range 1 11) (set! t (+ t x))) t)"), "55");
}
