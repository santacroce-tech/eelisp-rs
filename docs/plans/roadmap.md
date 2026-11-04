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
