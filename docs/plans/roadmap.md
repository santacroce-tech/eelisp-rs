# Roadmap: a self-hosted EELisp

*2026-11-04. This is the plan. The other files in this folder are background notes:
[self-hosting.md](self-hosting.md) (kernel/library split, storage drivers),
[eedb.md](eedb.md) (a database engine of our own), [riscv-os.md](riscv-os.md) (the OS dream).*

## Goal

**The EELisp compiler is written in EELisp and compiles itself.** Self-hosting is the point.
Getting rid of Rust isn't.

The shape is the one Erlang, Java, OCaml and Smalltalk use: a compiler in the language, and a
small virtual machine underneath it.

```
          compiler.eel  (reader, macro expansion, bytecode generation, in EELisp)
                │  compiles itself
                ▼
          compiler image (bytecode)
                │
  snippet ──►  VM in Rust  ──► result
               (Value, builtins, database, agenda, sheets, EngineHandle, WASM: unchanged)
```

## Why a bytecode VM in Rust

| Option | Verdict |
|---|---|
| Compile to C, plus a C runtime | We'd lose Rust's memory safety, and eeditor would need a new integration. |
| Compile to Rust source, then `rustc` | A snippet typed in the editor would need `rustc` at runtime, which is too slow. |
| **Compile to bytecode for a Rust VM** | **Chosen.** The compiler lives inside the engine, snippets compile in milliseconds, and the whole host API stays the same. |

What stays the same for hosts (the CLI, `--serve`, `EngineHandle`, `eelisp-web`, eeditor-next):
everything. Only what happens inside `eval` changes: compile the form to bytecode, then run it.

## The bootstrap

1. **Stage 0:** the current tree-walking interpreter runs `compiler.eel` on its own source and
   produces image 1.
2. **Stage 1:** the VM loads image 1, and the compiler compiles itself again into image 2.
3. **Check:** image 1 and image 2 must be byte-for-byte identical.
4. Commit the image, and the VM starts from it.

The tree-walker stays. It can rebuild the image from source at any time (so we never depend on a
binary nobody can reproduce), and it is the reference for testing.

## Steps

| # | Step | Done when |
|---|---|---|
| 1 | **Bytecode + VM in Rust.** An instruction set, a VM loop, closures, tail calls; builtins are called as primitives. Rust tests feed it hand-written bytecode. | A `vm` test module runs arithmetic, `if`, closures, recursion and deep tail calls. |
| 2 | **Compiler in EELisp, run by the tree-walker.** Special forms, macro expansion, `loop`/`recur`, `try`/`catch`. It calls the native reader at first. | Differential tests: language test cases give the same result on the tree-walker and on compile + VM. |
| 3 | **Bootstrap.** Stage 0 → stage 1 → stage 2. | Images 1 and 2 are identical; the image is committed, with a script that regenerates it. |
| 4 | **Switch `eval` to compile + VM.** Behind a feature or option first. | Full `cargo test` passes on the VM; an eeditor-based benchmark is at least even. |
| 5 | **Move libraries up into EELisp.** The prelude first, then agenda and sheet logic. | Tests stay green; less Rust in `builtins.rs`, `agenda.rs` and `sheet.rs`. |

## Status

- **Step 1 — done.** `src/vm.rs`, `tests/vm.rs`. Bytecode is written as EELisp data and turned
  into functions by `vm-assemble` / `vm-run`.
- **Step 2 — done.** `src/compiler.eel`, `tests/compiler.rs`. Every special form compiles,
  `quasiquote`, `for-each` and `defmacro` included (compiled macros expand by running bytecode).
  The differential test reads 434 snippets out of seven test suites (lang, book_gaps, bytes,
  csv, host, agenda_advanced, db_agenda), adds 37 edge cases, and runs all 471 on the
  interpreter and through the compiler. All agree in three setups: compiler interpreted, prelude
  compiled, and **compiler compiled by itself**. The only forms left to the interpreter
  (`(interp form)`) are malformed ones (`(def x)`, `(defmacro)`…), so their error is its own.

- **Step 3 — done.** `src/boot.rs`, `boot/compiler.image`, `examples/bootstrap.rs`,
  `tests/bootstrap.rs`. Stage 0 (interpreted compiler) and stage 1 (compiler loaded from the
  image, bytecode only) produce the same image, character for character: 36 protos, ~50 KB of
  readable EELisp, with no `(interp …)` in it. A test fails if the committed image is stale, and
  the differential suite runs a fourth time with the compiler loaded from the image alone.
- **Measuring — first round done.** `examples/bench.rs` runs synthetic workloads and the example
  calls from eeditor's snippet files on both engines. Compiled code started up to a third slower;
  six fixes in the VM (copyable instructions, arguments passed in place, `recur` without the
  error path, builtins called by reference, a fused `def-pop`, reused machines) brought it to
  even: 0.9–1.15×, most real snippets ~1.05×. Big gains need locals in slots, which changes
  whether `eval` can see a caller's locals. See docs/self-hosting/05-measuring.md.
- **Locals in slots — done.** Decision: `eval` runs at the top level (Scheme / Common Lisp /
  Clojure rule), a documented clean break. The compiler resolves each variable to a frame slot, a
  captured cell or a global; the VM's slot mode keeps locals in frames, so calls make no scope.
  517 differential snippets (75 edge cases, 38 about slots) agree in all four setups; the image is
  still a fixed point. Compiled code: 1.1–1.4× the interpreter on most workloads; `lists` ~0.95×
  (time is in shared builtins). See docs/self-hosting/06-slots.md.

Known differences from the interpreter (deliberate for now, each with a test to write when fixed):

1. Calling something that isn't a function: the interpreter reports "expected function" before
   evaluating the arguments; compiled code evaluates them first, so an error in an argument wins.
2. A macro defined *after* a function that uses it: the interpreter expands it when the function
   runs; compiled code needed it when the function was compiled, and stops with "macro … reached
   at run time".
3. A macro is expanded once, when its caller is compiled, not every time the caller runs.
4. A `def` made *after* a closure in the same local scope isn't seen by that closure (the
   interpreter's closure keeps the whole scope and would find it; compiled code resolved the name
   to the global when it compiled the closure).
5. A malformed form left to the interpreter (`(set! (f) 1)`…) doesn't see local variables, so its
   error may read "undefined symbol" instead of the syntax error.

Not yet measured: compiled code still looks variables up by name, like the interpreter, so it
isn't expected to be much faster until the compiler resolves locals to slots.

## Later (dream list, independent of the steps above)

- Native backends from the same compiler: C, then RISC-V.
- eedb: our own SQL engine behind a driver interface ([eedb.md](eedb.md)).
- A C compiler in EELisp ("inception"), so libc applications can run.
- The OS ([riscv-os.md](riscv-os.md)).

## Rules for the work

- The test suite is the referee. Every step keeps `cargo test --all` green and the
  `language-only` wasm32 build working.
- Small steps: each step lands as its own PR.
- Measure before switching: the VM replaces the tree-walker only on evidence.
