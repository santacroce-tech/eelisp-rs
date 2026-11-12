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

That's how the first image was made. Since then (chapter 10) the committed image builds the next
one: the old compiler compiles the new source, which then compiles itself until it reaches a fixed
point, and stage 0 runs alongside as a cross-check.

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

- **Step 4 — compiled mode, behind a switch.** `Interpreter::set_compiled(true)` (or `--compiled`,
  or `EELISP_COMPILED=1`) loads the compiler from the embedded image, recompiles the prelude, and
  compiles every top-level form. The whole test suite passes in compiled mode (20 suites, with
  and without default features) and CI runs it both ways. Off by default because compiling costs
  10–60 µs per form: loading eeditor's snippet files goes from 1.5 ms to 54 ms. See
  docs/self-hosting/07-switching-on.md.
- **Compiler speed — first round.** Counting calls showed 6,779 per form compiled, mostly list
  walks (special-form membership, name lookup). Dicts and resolving a call's head once: 2,569
  calls per form; loading eeditor's snippets compiled 54 ms → 26 ms; `(+ 1 2)` 12 → 5.8 µs.
  See docs/self-hosting/08-profiling-the-compiler.md.
- **Compiler and VM speed — second round.** A declared-names set, `dict?` instead of `(type x)`
  strings, direct argument walking; numbers copied without `Value::clone`. About 2,000 calls per
  form; snippets compiled 25.2 → 22.7 ms; compiled `loop` 1.45×, `let-heavy` 1.43×. Dropped
  after measuring: LTO, per-instruction global caches. Remaining costs are spread thinly; bigger
  gains need bigger changes (value representation in the VM, a compiler without dict/closure
  records).

- **A reader in EELisp.** `src/reader.eel`: text to forms, built into the image next to the
  compiler; `(read-all src)` in compiled mode. New builtins: `str-chars`, `symbol`, `keyword`.
  `tests/reader.rs` checks it against the Rust reader on 75 of the repo's files (~1 MB, Rust and
  Markdown included), 39 edge cases and 1,500 random strings: same forms or the same error, both
  interpreted and from the image. **Stage 2** of the bootstrap reads the source with it and
  compiles it with the EELisp compiler: same image, byte for byte. Two printer fixes it forced: dict
  keys that aren't keyword-safe print as strings, readable strings escape `\n \t \r`. Speed: 20.7
  ms for 33K chars vs 0.78 ms in Rust; the cost is allocation (strings and keywords own their
  text), which points at the value representation. Not yet the engine's reader: needs source
  positions for `(source f)`, a parse error kind, and speed. See docs/self-hosting/09-a-reader-in-eelisp.md.

- **The old image builds the new one.** `boot::rebuild`: the committed image reads and compiles
  the new source (generation 1), that compiler does it again (2) and once more (3); 2 and 3 must
  match. The interpreter only cross-checks: stage 0 must equal generation 2. `--from-source`
  starts from the interpreter when there's no usable image. A test shows a new macro in the
  compiler's own source settling in two generations. See docs/self-hosting/10-the-old-builds-the-new.md.

- **The engine reads in EELisp.** In compiled mode `eval_str`/`eval_all` (and the prelude
  recompile) read with `read-top` from `src/reader.eel`: forms, source text and comments, exactly
  as `parser::top_forms`; errors are parse errors via `(error msg :parse)`. New builtin `slice`.
  Cost: compiled `(+ 1 2)` 6 → 22 µs, eeditor snippets 24 → 89 ms (reading is per-character call
  and allocation cost). The Rust reader remains for loading the image (the seed) and the
  interpreter. See docs/self-hosting/11-the-engine-reads-in-eelisp.md.

- **Shared strings (merged).** `Value::Str` is `Rc<str>`, `Value::Keyword` an
  interned `Sym`; `Value` kept at 32 bytes with `repr(align(16))` (24 bytes made number loops ~9%
  slower). Text-heavy snippets 1.3–1.56× faster in both engines, EELisp reader 1.22×, compiled
  per-evaluation 1.16–1.18×, synthetic workloads 1.00–1.05×. Public type change. See
  docs/self-hosting/12-sharing-strings.md.

- **A cheaper call.** Arithmetic and comparison builtins carry a `FastOp` tag; the VM computes
  them inline on two numbers (a redefined `+` has no tag). `Load` caches where a global's value
  lives, valid while `env::layout()` (bumped only when a global scope is made or gains a name)
  hasn't moved; `set!`/redefinition don't invalidate. Tail calls no longer allocate. Compiled:
  `loop` 1.35×, `let-heavy` 1.27×, `fib` 1.22×; interpreter unchanged. Known limit: the reader
  builds top-level form lists with `append` (quadratic for files with thousands of forms). See
  docs/self-hosting/13-a-cheaper-call.md.

- **Reshaping the VM (branch `exp/vm-redesign`).** Experiments as commits, tagged
  `vm-redesign-base`, `-1`, `-2`; measured with `examples/calls.rs` and `scripts/ab.sh`. Kept:
  frames without their own mark/handler vectors, dispatch with code/pc/slot base in locals.
  Dropped after measuring: superinstructions (no net gain), borrowing the proto, moving the
  function off the stack. Compiled: `loop` 1.13×, `let-heavy` 1.09×, `fib` 1.07×, snippets
  1.00–1.10×; `lists` (map/filter calling small functions) 0.94×. Next big steps would be
  word-sized values, a register machine, or map/filter calling back into the running VM. See
  docs/self-hosting/14-reshaping-the-vm.md.

- **The way in (branch `exp/compiled-default`, on top of `exp/vm-redesign`).** Builtins call
  compiled functions with borrowed arguments, into reused machines: `map/filter` 1.44×. The reader
  builds lists and escaped strings in chunks: linear (4,000 forms 168 → 64 ms). `reader-class` is a
  macro. Dropped after measuring: scanning code points instead of one-character strings. Running
  compiled code beats the interpreter everywhere (1.1–2.1×). Getting code in doesn't: loading
  eeditor's snippets takes 65 ms against 1.4 ms, 43 ms of it reading at 100–700 ns per character.
  Decided: a read cache now (reading is pure; compiling isn't, so only reading is cached):
  reloading the snippets 60 → 16 ms, a repeated `(+ 1 2)` 18 → 5.4 µs. A faster VM is the real
  fix, next. See docs/self-hosting/15-the-way-in.md.

- **A register machine (branch `exp/register-vm`), started.** A hand-written prototype with the
  real global cache, builtins and fast path: `fib` 1.84×, a loop 2.13×, the reader's whitespace
  loop 1.44× over the stack VM. Plan, steps and the bootstrap crossing:
  [register-vm.md](register-vm.md). Step 1 done: register instructions in the existing machine,
  mixing with stack code; hand-written register code runs `fib` 1.24× and a loop 1.79× faster,
  the whitespace loop no faster (its cost is in `nth` and `dict-get`). With intrinsics for those
  two, run inside `call2` with borrowed operands: `fib` 1.57×, the loop 2.66×, the whitespace
  loop 1.89×. See docs/self-hosting/16-registers.md. Step 2: the compiler emits register code
  for calls and the image crossed over in one go; compiled loops and tail calls 1.9×, calls 1.35×,
  reading 7% faster, compiling 18% slower. Then intrinsics for `=` and one-argument builtins,
  found by counting calls: reading 43.0 → 38.0 ms, compiling 18.4 → 16.2 ms; then `let`, `if`,
  `and`, `or` in registers: reading 35.3 ms, compiling 15.1 ms. Chapter 18: the machine's calls
  are no longer what reading costs; the reader's per-character work and allocation are. Chapter
  19: scanning builtins and a list builder, reading 35.1 → 27.6 ms. Chapter 20: the reader's
  own costs (comment search, `str-join`, `str-chars`), reading 20.2 ms, 2.1× chapter 15. Chapter
  21: compiler by builtin, atoms without pairs; loading compiled code 65 → 35 ms. Chapter 22:
  the compiler builds code in one list, compiling 14.5 → 12.5 ms; loading 33 ms. Chapter 23:
  word-sized values prototyped, mixed (0.88–1.4×), set aside; the machine's own structure costs
  twice what a minimal machine does. Chapter 24: profiled by source line; `recur` and local
  operands cheaper, calls and loops 1.14–1.15×. See
  docs/self-hosting/17-a-register-back-end.md.

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
