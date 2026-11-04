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

The plan these chapters follow is [docs/plans/roadmap.md](../plans/roadmap.md). The code is in
`src/vm.rs`, `src/compiler.eel` and `src/boot.rs`; the tests that back every claim made here are
`tests/vm.rs`, `tests/compiler.rs` and `tests/bootstrap.rs`.
