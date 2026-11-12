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
| 17 | [A register back end](17-a-register-back-end.md) | The compiler emits register code for calls, and the crossing is ordinary; calls not worth registers, a slot that costs more than it saves, a function that isn't defined while it is compiled (`call-at`), and running code up to 1.9× faster; then counting calls to find the reader's and compiler's costs, intrinsics for `=` and one-argument builtins, reading 12% faster and compiling back where it was; `let`, `if`, `and`, `or` in registers |
| 18 | [What a call costs](18-what-a-call-costs.md) | Register calls that leave the caller's dead slots alone (a few percent), copying values inline (nothing), and the finding that reading's time is no longer in the machine's calls but in the reader's instructions per character and what it allocates |
| 19 | [Scanning](19-scanning.md) | Two scanning builtins run the reader's per-character loops while every decision stays in EELisp, and a list builder ends three allocations per element: reading 35.1 → 27.6 ms, half the calls |
| 20 | [The reader's own costs](20-the-readers-own-costs.md) | Fewer comparisons changed nothing; a profile by builtin found the costs: walking back over comments, `str-join` and `str-chars` allocating per character. Reading 27.6 → 20.2 ms, 2.1× faster than chapter 15 |
| 21 | [Compiling and reading, again](21-compiling-and-reading-again.md) | The compiler profiled by builtin (one `append` instead of a `reduce`; a `box` builtin measured as no gain), atoms read without a result pair; loading compiled code 65 → 35 ms since chapter 15, and the three larger moves left |
| 22 | [Compiling into one list](22-compiling-into-one-list.md) | Every compiling function adds to one list instead of returning one for its caller to copy; the same code comes out, 14% faster; loading compiled code half what it cost in chapter 15 |
| 23 | [Word-sized values](23-word-sized-values.md) | A NaN-boxing prototype, generic over its value type: 1.4× where values are copied, 0.88× on pure arithmetic, so it's set aside; and the same machine with today's values runs twice as fast as the real one |
| 24 | [What the machine pays](24-what-the-machine-pays.md) | Redefinable arithmetic bounded at 1–6%; a profile by source line inside the dispatch loop; `recur` and local operands made cheaper inside their instructions: calls and loops 1.14–1.15× |

The plan these chapters follow is [docs/plans/roadmap.md](../plans/roadmap.md). The code is in
`src/vm.rs`, `src/compiler.eel`, `src/reader.eel` and `src/boot.rs`; the tests that back every
claim made here are `tests/vm.rs`, `tests/compiler.rs`, `tests/reader.rs`, `tests/registers.rs` and `tests/bootstrap.rs`; the numbers come from
`examples/bench.rs`.
