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
| 10 | [The old compiler builds the new one](10-the-old-builds-the-new.md) | The committed image builds the next one in three generations; a new feature needs two to settle; the interpreter as an independent witness against "trusting trust" |
| 11 | [The engine reads in EELisp](11-the-engine-reads-in-eelisp.md) | Compiled mode reads every form with the EELisp reader: source text and comments for `(source f)`, parse errors of the right kind, and what reading costs |
| 12 | [Sharing strings](12-sharing-strings.md) | Strings that share their text and interned keywords: 1.2–1.5× on text-heavy code in both engines, and why making `Value` smaller first made it slower |
| 13 | [A cheaper call](13-a-cheaper-call.md) | Arithmetic computed inline, globals read from where they live; measuring changes smaller than the noise of code layout; compiled loops 1.35× faster |
| 14 | [Reshaping the VM](14-reshaping-the-vm.md) | Four experiments on the VM's shape, each a commit on `exp/vm-redesign`: lighter frames and local dispatch state kept (about 1.1×), superinstructions measured as no gain, and a mistake in the record corrected |
| 15 | [The way in](15-the-way-in.md) | Builtins calling compiled code 1.44× faster, the reader linear on large files, and where compiled mode's cost really is: reading, at 100–700 ns per character, with a wrong guess measured and reverted |
| 16 | [Registers](16-registers.md) | Register instructions in the VM, mixing with stack code: calls and loops 1.2–1.8× faster on hand-written code, the reader's loop not at all, and a dispatch loop that slowed stack code until it was made smaller again; then intrinsics for `nth` and `dict-get`, and the reader's loop 1.89× |

The plan these chapters follow is [docs/plans/roadmap.md](../plans/roadmap.md). The code is in
`src/vm.rs`, `src/compiler.eel`, `src/reader.eel` and `src/boot.rs`; the tests that back every
claim made here are `tests/vm.rs`, `tests/compiler.rs`, `tests/reader.rs`, `tests/registers.rs` and `tests/bootstrap.rs`; the numbers come from
`examples/bench.rs`.
