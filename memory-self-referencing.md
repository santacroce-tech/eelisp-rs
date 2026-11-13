# Where we are: a handoff for the next session

*Written 2026-11-13, at the end of a long session on branch `exp/register-vm` (tag
`register-vm-13`, plus one plan commit after it). Read this first, then the files it points to. It
says what was built, what was decided, how we work, and what's next.*

## The project in one paragraph

EELisp is a Lisp for an editor (eeditor). The goal is a **self-hosted EELisp**: the compiler
(`src/compiler.eel`) and the reader (`src/reader.eel`) are written in EELisp and emit bytecode for
a VM written in Rust (`src/vm.rs`). Rust stays as the host. The compiler compiles itself; the
result is committed as `boot/compiler.image`. The tree-walking interpreter remains, both as the
default engine and as the independent witness that rebuilds the image from source. Plan:
`docs/plans/roadmap.md` (steps 1–5) and `docs/plans/register-vm.md` (the register machine's own
plan, with a status list kept up to date).

## The book

Every step is written up as a chapter in `docs/self-hosting/` (index: its `README.md`), meant to
become a book. **Chapters 1–24 exist.** Every finding goes in, including dead ends and reverted
experiments, with their numbers. The user asked for this explicitly ("make sure you update our
book"). Style: plain, concrete, numbers in tables, honest about what didn't work.

| chapters | what |
|---|---|
| 1–13 | why self-host, the VM, the compiler, the bootstrap, slots, compiled mode, the EELisp reader, shared strings, cheaper calls |
| 14 | reshaping the VM (`exp/vm-redesign`) |
| 15 | the way in: reading is the cost; the read cache (`exp/compiled-default`) |
| 16–17 | registers: register instructions in the VM; the compiler's register back end |
| 18 | what a call costs |
| 19–20 | scanning builtins and a list builder for the reader; the reader's own costs |
| 21–22 | compiling and reading again; the compiler builds code in one list |
| 23 | word-sized values (NaN-boxing) prototyped, set aside |
| 24 | what the machine pays: line-level profiling of the dispatch loop |

## Branches (all pushed, none merged)

```
main (bb73ffa)
 └ feat/bytecode-vm   PR #34, CI green on 984d0a3 (chapters 1–13)
    └ exp/vm-redesign     tag vm-redesign-final (chapter 14)
       └ exp/compiled-default  tags compiled-default-1/2 (chapter 15)
          └ exp/register-vm    tags register-vm-base … register-vm-13 (chapters 16–24)
```

**The user decides about merging.** Their words: keep the branches "as a reference until we
finish the whole thing". Commit and push only on these experiment branches, never on `main`. Two
old CI runs on PR #34 (for 7346a02 and 271170b) are stuck "queued" on GitHub's side and can't be
cancelled; they don't matter.

## How we work (the user's rules)

- **Experiments as commits.** Every attempt is its own commit with its numbers in the message,
  including failures. A failed one is followed by a revert commit (`git revert --no-commit HEAD`,
  then a commit saying why). Tag milestones (`register-vm-N`), push branch and tag.
- **Measure, don't guess.** Two guesses in a row failed more than once; profiles found the real
  costs every time.
- **Don't propose language features for speed.** Returning multiple values was considered for the
  reader and **dropped** by the user: not core Lisp, not wanted. Keep EELisp's surface small.
  General builtins were accepted when the user chose them (`scan-while`, `scan-until`,
  `scan-back-until`, `list-builder`).
- **The engine reads in EELisp** in compiled mode; the Rust reader isn't put back.
- **Ask the user** for decisions that are theirs: merging, language changes, the default engine.
- Verification before any commit that touches the engine (below), and CI runs clippy with
  `-D warnings` on stable.

## Verification (run before committing engine changes)

```sh
cargo test --release                                  # 22 suites "test result: ok"
EELISP_COMPILED=1 cargo test --release                # 22, compiled mode
cargo test --release --no-default-features            # 21
EELISP_COMPILED=1 cargo test --release --no-default-features   # 21
cargo clippy --all-targets --release -- -D warnings   # and with: cargo +1.99.1 clippy …
cargo build --no-default-features --target wasm32-unknown-unknown --lib --release
```

Count with `| grep -c "test result: ok"`. `cargo build` doesn't rebuild examples: build them with
`--example name` before measuring. CI doesn't check `cargo fmt`, and some untouched files aren't
formatted.

**After changing `src/compiler.eel` or `src/reader.eel`, rebuild the image:**
`cargo run --release --example bootstrap`. It builds from the committed image in three
generations (2 and 3 must be identical) and cross-checks against the interpreter. Use
`--from-source` when the new source defines and uses a macro in the same file. Then commit
`boot/compiler.image` with the source. The referees: `tests/compiler.rs` (517 snippets against the
interpreter, four setups), `tests/reader.rs` (the EELisp reader against the Rust one on hostile
input), `tests/bootstrap.rs`, `tests/registers.rs`.

## Measuring

- `scripts/ab.sh <before> <after> [rounds]`: builds both commits in worktrees, alternates runs of
  `examples/calls.rs`, keeps the best. **Code layout moves results by ±3–5% for a given binary,
  deterministically** (chapters 14, 17, 24): every new dispatch arm costs others a little; always
  A/B, run twice when it matters.
- `examples/front_end.rs <dir>`: read / compile / assemble times. `examples/read_kinds.rs`: ns per
  character by kind of text. `examples/prof_compile.rs <dir>`: compiling alone.
  `examples/bench.rs <dir>`: eeditor's snippets, interpreter vs compiled.
- Benchmark corpus: `/Users/rob/projects/eeditor-next/src/snippets` (`*.eelisp`).
- `--features count-calls` with `examples/count_calls.rs <dir>`: calls per function/builtin, for
  reading and compiling.
- Sampling: `prof_one.rs "(expr)" N [defs.eel]`, `prof_read.rs`, `prof_compile.rs` under macOS
  `sample PID 5`. **Line-level profiling** of the dispatch loop: build with
  `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` into its own target dir, `xcrun xctrace record
  --template 'Time Profiler' --attach PID`, export the time-profile table, `dsymutil` the binary,
  map leaf addresses with `atos -o bin -l <load-addr> <addr-1>` (chapter 24 has the recipe).
- Prototypes kept as yardsticks: `examples/regvm_proto.rs` (register machine, chapter 16),
  `examples/nanbox_proto.rs` (8-byte values, chapter 23), `examples/fib_variants.rs`.

## Where the numbers stand

eeditor's snippet files (108 KB, 157 forms), compiled mode:

| | chapter 15 start | now |
|---|---|---|
| read | 43.4 ms | 19.5 ms |
| compile | 14.9 ms | 12.8 ms |
| assemble | 0.9 ms | 0.8 ms |
| total load | 65 ms | 33 ms |
| interpreter, same files | 1.4 ms | 1.4 ms |

Running compiled code is 1.0–2.0× faster than before the register work (loops and tail calls
most). The real VM is 1.4–1.7× slower than the minimal prototype; no single large cost is left in
it (chapter 24).

## What the machine is now (for orientation in `src/vm.rs`)

- One VM with **stack and register instructions mixed** in a proto. Registers are a slot-mode
  proto's slots: variables, then temporaries `(t k)` (`:slots n :temps m`).
- Register instructions: `move`, `global`, `push`, `pop-to`, `call1`, `call2` (fast arithmetic
  and **intrinsics**: `nth`, `dict-get`, `=`, `!=`, `head`, `length`, `empty?`, `nil?`, `list?`,
  `symbol?`, `not`), `call-global` / `tail-call-global` (arguments computed into the callee's
  first registers, entered in place), `call-at` / `tail-call-at` (the function in a register
  first, as Lua's CALL; for callees unknown at compile time), `jump-if-not`, `return-value`.
- A register write is a new binding (as `init-local`), not a write through a closure's cell.
- The compiler (`compiler-to-dest` and friends) emits register code for calls, `let`, `if`,
  `and`, `or`; builds each function's code in one `list-builder` (`out` parameter). `recur`,
  `loop`, `try`, closures, quasiquote are still stack code.

## Open, in order of the roadmap

1. **Finish step 4: compiled mode as the default.** The five known differences from the
   interpreter are listed in `docs/plans/roadmap.md` (Status, "Known differences"): decide for
   each to fix (with a test) or accept, one commit and one book section each. Then the user
   decides on compiled-by-default and on merging the branch stack. Also fix the stale roadmap line
   that says compiled code "looks variables up by name".
2. **Retire the stack instructions** (step 4 of `register-vm.md`) once compiled code rarely uses
   them (`recur`, `loop`, `try`, closures, quasiquote would need register forms first).
3. **Step 5: move libraries up into EELisp**: the prelude first, then agenda and sheet logic.
4. **Later (dream list):** native back ends (C, then RISC-V), eedb, a C compiler in EELisp, the
   OS.

My suggestion when this was written: start with item 1, one known difference at a time.

## Also see

- Claude's own memory for this project:
  `~/.claude/projects/-Users-rob-projects-eelisp-rs/memory/` (`self-hosting-roadmap.md` has the
  dated log of every tag and decision).
- The full conversation transcript of the session that produced chapters 15–24:
  `~/.claude/projects/-Users-rob-projects-eelisp-rs/c00e3cd8-0206-4241-babd-2907232250f3.jsonl`.
