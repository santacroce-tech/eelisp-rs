# 10. The old compiler builds the new one

*Code: `rebuild`, `next_image` and `stage0_of` in `src/boot.rs`; `examples/bootstrap.rs`. Tests:
`tests/bootstrap.rs`.*

## Who builds the compiler?

Since chapter 4, every new image started with the interpreter. Change `src/compiler.eel`, run the
bootstrap, and the tree-walking interpreter in Rust ran the new compiler's source once (stage 0),
before the compiler took over and compiled itself.

That's how a self-hosted compiler is *born*, not how it lives. Rust's compiler is built by the
previous release of Rust's compiler, Go's by the previous Go, OCaml's by the previous OCaml. None
of them goes back to the language it was first written in. Each version of the compiler is the
seed of the next.

This chapter makes EELisp work the same way. The committed image, which is the previous compiler
as bytecode, builds the next image. The interpreter keeps a different job: an independent check.

## One generation

A generation is one step: an image reads and compiles a source.

```rust
pub fn next_image(image: &str, src: &str) -> Result<String, LispError> {
    let it = with_image(image)?;                     // the old compiler, as bytecode
    let forms = it.call("read-all", &[src])…;        // its reader reads the new source
    image_of(&it, forms)                             // its compiler compiles it
}
```

Since chapter 9 the image contains a reader as well as a compiler, so a generation needs nothing
from Rust except the VM, the builtins, and reading the image itself. That last one is the
irreducible seed: an image is EELisp text, and *something* has to read the first one.

## Three generations

Building the image for a new source takes three generations:

```
old image ──compiles new source──▶ generation 1    the new compiler, built by the old one
gen 1     ──compiles new source──▶ generation 2    the new compiler, built by itself
gen 2     ──compiles new source──▶ generation 3    must equal generation 2
```

Generation 1 is the new compiler, but compiled by the *old* compiler. If the new source changed
what the compiler emits, generation 1 is compiled the old way, so it can differ from the rest.
Generation 2 is the new compiler compiled by the new compiler. Generation 3 compiles it once
more. If 2 and 3 are identical, the new compiler is a fixed point, and that's the image.

GCC's build does the same thing. `make bootstrap` builds GCC three times and compares the last
two, a step it calls "compare".

## A new feature in the compiler's own source

The interesting case is when the compiler's source starts using something the old compiler
doesn't know. Here's that case in miniature, as a test: the source gains a macro, and a function
that uses it.

```lisp
(defmacro compiler-twice (x) (list 'do x x))
(defn compiler-demo (y) (compiler-twice (+ y 1)))
```

Here is what each generation makes of `compiler-demo`:

- **Generation 1.** The old compiler has never heard of `compiler-twice`. To it, that's a call to
  a global function by that name, so it emits a call:
  `(load compiler-twice) … (tail-call 1)`. That's wrong, because `compiler-twice` is a macro.
- **Generation 2.** Loading generation 1 *ran* every form of the new source, `defmacro` included.
  So when generation 1 compiles, the macro exists, and it expands it:
  `(load +) (load-local 0 y) (const 1) (call 2) (pop) …`. That's right.
- **Generation 3.** The same as 2.

The source needed two generations to settle. That's the rule every self-hosted language lives
by: **a compiler can use a new feature only once a compiler that understands it exists.** A Rust
compiler developer adds a feature in one release and uses it in the compiler's own code in the
next. Here a generation is a few dozen milliseconds, so the bootstrap simply runs enough of them.

## The interpreter as a witness

With the old image building the new one, the interpreter isn't needed to build anything. It
still runs on every bootstrap, as a cross-check: stage 0, the interpreter running the source
directly, read by the Rust reader, must produce exactly generation 2. If it doesn't, the image
isn't written.

That check matters because of Ken Thompson's 1984 lecture *Reflections on Trusting Trust*.
Thompson showed that a compiler can carry behavior that is in no source code. A compiler binary
can be taught to recognize when it's compiling itself and re-insert that behavior, so the
behavior survives every rebuild from clean source. Once compilers are built by compilers, reading
the source doesn't prove what's in the binary.

The usual defense, David A. Wheeler's *diverse double-compiling*, is to build the compiler a
second time with an unrelated implementation and compare the results. EELisp has one built in.
The interpreter and the Rust reader share no code with the image, and the bootstrap requires them
to agree with it byte for byte. Anything in the image that isn't in the source would show up as
a difference.

## Running it

```
$ cargo run --example bootstrap
boot/compiler.image: 89 protos, 112394 bytes, unchanged.
  built from the committed image in three generations (133ms): the first already identical; generations 2 and 3 identical
  cross-check: the interpreter makes the same image (11ms)
```

When the source changes, the bootstrap reports whether generation 1 differed. If the committed
image can't build the new source at all (because it's broken, or the change is too big for the old
compiler to compile), `--from-source` starts from the interpreter, as before:

```
$ cargo run --example bootstrap -- --from-source
```

Two tests keep this honest. One checks that the committed image rebuilds itself unchanged, with
no interpreter involved. The other is the macro case above: generation 1 calls the macro,
generation 2 expands it, generation 3 changes nothing, and the interpreter agrees with generation
2.
