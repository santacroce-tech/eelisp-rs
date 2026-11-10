# EELisp, compiled by itself

Articles about making EELisp a self-hosted language: a compiler written in EELisp that compiles
EELisp, itself included. They are written as the work happens, one per step, and are meant to
become a book.

| # | Chapter | What it covers |
|---|---|---|
| 1 | [Why self-host, and how](01-why-and-how.md) | What "self-hosted" means, the options we weighed (C, Chez Scheme, Rust source, bytecode), and why a compiler in EELisp on a VM in Rust won |
| 2 | [A virtual machine](02-a-virtual-machine.md) | The bytecode VM: a stack machine, its instructions, tail calls, `loop`/`recur`, `try`/`catch`, and how compiled and interpreted code live side by side |
| 3 | [A compiler in EELisp](03-a-compiler-in-eelisp.md) | `src/compiler.eel`: compiling each form, macros at compile time, and the differential test that keeps it honest |
| 4 | [The bootstrap](04-the-bootstrap.md) | The compiler compiling itself, stage 0 and stage 1, and the fixed point that proves it works |
| 5 | [Measuring](05-measuring.md) | A benchmark from real editor snippets, a compiler that was first slower than the interpreter, what fixed that, and the language decision that real speed needs |
| 6 | [Locals in slots](06-slots.md) | `eval` joins the Lisp family's rule; the compiler resolves every variable to a slot, a captured cell or a global, keeping the interpreter's meaning exactly |
| 7 | [Switching it on](07-switching-on.md) | Compiled mode as the engine's own `eval`: one switch, the whole test suite run both ways, and what compiling every form costs an editor |
| 8 | [Profiling the compiler](08-profiling-the-compiler.md) | Counting an EELisp program's calls to find its costs: 6,779 calls per form compiled, two list walks to blame, and half of it gone |
| 9 | [A reader in EELisp](09-a-reader-in-eelisp.md) | Text to forms in EELisp, agreeing with the Rust reader on a megabyte of hostile input; stage 2 of the bootstrap, where no Rust stands between the source and the bytecode |

The plan these chapters follow is [docs/plans/roadmap.md](../plans/roadmap.md). The code is in
`src/vm.rs`, `src/compiler.eel`, `src/reader.eel` and `src/boot.rs`; the tests that back every
claim made here are `tests/vm.rs`, `tests/compiler.rs`, `tests/reader.rs` and `tests/bootstrap.rs`; the numbers come from
`examples/bench.rs`.
