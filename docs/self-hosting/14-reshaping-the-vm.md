# 14. Reshaping the VM

*Branch: `exp/vm-redesign`. Tags: `vm-redesign-base`, `vm-redesign-1`, `vm-redesign-2`. Tools:
`examples/calls.rs` and `scripts/ab.sh`. Every experiment in this chapter is a commit on that
branch, including the ones that didn't work, so any two steps can be compared again later.*

## Where chapter 13 left off

After chapter 13, a compiled call of `fib` cost about 80 ns, and the profile said where:

| | |
|---|---|
| dispatching instructions (`Vm::step`) | about 48% |
| the call itself (`Vm::call`) | about 20% |
| dropping values | about 10% |
| setting up and leaving a frame (`enter`, `ret`) | about 10% |

Global lookups and the arithmetic builtins, the targets of chapter 13, were gone from the top of
the list. What was left was the shape of the VM itself: how it fetches instructions, and what a
frame is. This chapter tries four changes to that shape, and keeps two.

## The method, made permanent

Chapter 13 showed that on this machine, timings drift by a few percent, and that a small edit to
the dispatch loop can move every benchmark by a similar amount just by changing how the compiler
lays the loop out. So before any experiment, two tools went into the repository:

- `examples/calls.rs` runs six call-heavy programs in compiled mode: `fib`, closures, a loop of
  calls, tail calls, a plain loop, and `map`/`filter` with small functions. Each line is the best
  of five runs.
- `scripts/ab.sh <before> <after>` checks out both commits in their own worktrees, builds both,
  and alternates their runs, keeping each line's best.

The first thing measured was the tool itself: the baseline against itself. Every line came out
between 0.975× and 1.000×. That's the noise band. A result inside about ±2.5% says nothing.

## The experiments

| step | idea | result | kept? |
|---|---|---|---|
| 1 | frames without their own mark and handler vectors | calls 1.05×, the rest even | yes, `vm-redesign-1` |
| 2a | dispatch keeps the code and program counter in locals | loop 1.08×, map/filter 0.96× | step towards 2b |
| 2b | …and the frame is looked up only by the instructions that need it | loop 1.13×, tail calls 1.07×, `fib` 1.06×, map/filter 0.95× | yes, `vm-redesign-2` |
| 2c | dispatch borrows the frame's proto instead of copying a reference to it | 1.00–1.02×, noise | no: not worth an `unsafe` block |
| 3 | five superinstructions, fused by the assembler | 0.82–0.97× at first, 0.98–1.01× when fixed | no: no gain |
| 4 | a call moves the function off the stack instead of copying it | 0.98–1.01×, noise | no |

### 1. Lighter frames

Each frame carried two vectors: the `loop` marks that `recur` jumps back to, and the `try`
handlers an error lands in. Most calls use neither, but every call still built two empty vectors,
moved them as part of the frame, and dropped them on return. Now both live on two stacks owned by
the VM. Each frame records only where its own entries begin (`mark0`, `handler0`). Calls got 5%
faster; nothing else moved.

### 2. Keeping the machine's state in locals

The dispatch loop used to start every instruction the same way: find the current frame
(`frames.last_mut()`), follow it to its prototype and from there to the code, check the bounds,
and fetch. The frame doesn't change between most instructions, only when control moves to another
frame: a call that enters a compiled function, a tail call, a return.

So now the loop has two levels. The outer one loads the frame's code, program counter and slot base
into local variables. The inner one runs instructions until control moves elsewhere, and then goes
back to the outer one. `call` says what it did: nothing to the frames (a builtin ran), switched to
another frame, or finished the run. Only the instructions that really need the frame (a global
load, a closure, a `def`) look it up.

This gave the biggest win of the chapter: a loop 1.13×, tail calls 1.07×, `fib` 1.06×. It also
has a cost. Entering the dispatch loop now does more setup, and for a function of three or four
instructions called afresh for every element of a list, as `map` and `filter` do, that setup isn't
paid back. That benchmark lost 5%.

A further step (2c) avoided copying the reference to the frame's prototype, by borrowing it
through a raw pointer. It measured within noise, and an `unsafe` block needs to earn its keep, so
it went.

### 3. Superinstructions: an instructive failure

Counting which instructions run next to each other, over eeditor's snippets and the call
benchmark, gave a clear picture. `LoadLocal` alone was 27% of everything run, and `Load` 23%. The
commonest pairs were:

| pair | share of instructions run |
|---|---|
| `load` → `load-local` (a function, then its first argument) | 16% |
| `load-local` → `call` | 9% |
| `const` → `call` | 6.5% |
| `load-local` → `const` | 5.7% |
| `jump-if-false` → `load` | 5.7% |

A superinstruction does the work of such a pair in one dispatch. The assembler can make them
without the compiler knowing, by fusing true neighbours in the instruction list. A label between
two instructions keeps them apart, so no jump can land in the middle of one. Five were added.
Their pairs account for about 40% of the instructions run, so fusing them removes up to a fifth
of all dispatches. The image didn't change, and neither did any result.

The first measurement was **3–18% slower everywhere**. Two hypotheses were tested in turn:

- **Bigger instructions.** Three operands made `Op` 16 bytes instead of 12. Packing slot and name
  indices into 16 bits brought it back to 12. Still slower.
- **The new code itself.** With the fusion switched off entirely, so the new instructions never
  ran, it was *still* slower. The culprit was a small helper, `self.local(…)`, written to share
  the code that reads a local variable between `LoadLocal` and the new instructions. As a method
  taking `&self`, it made the compiler treat the whole VM as borrowed, and the generated code for
  the entire dispatch loop got worse, even though the helper was marked `#[inline(always)]`.
  Turned into a macro, so its code is written out at each use, the slowdown disappeared.

With that fixed, the superinstructions measured **0.98–1.01×**: nothing. Removing about a fifth of
the dispatches saved exactly as much as the bigger dispatch loop cost. That is the useful
finding: the number of dispatches is no longer what a call costs here; the work inside each
instruction is. The three commits were reverted together, and the reasons are in the revert's
message.

### 4. One reference count less

A call copied the function value from the stack, which bumps its reference count, and later
dropped the stack's copy. Moving it out instead saves an increment and a decrement per call. That
measured as noise, and was reverted.

## A mistake in the record

The history of this branch has a commit titled *Revert "wip: dispatch borrows the frame's proto"*
that is in fact the change itself. The revert command had failed: it was given a flag it doesn't
take. The next command, meant to reword the revert's message, ran anyway, because the two were
joined by `;` rather than `&&`. So it reworded the experiment's own commit. The mistake surfaced
only later, when a lint pointed at code that shouldn't have been there.

The commit was left as it is, since rewriting history would make the branch harder to compare
against. A later commit, *Really revert the borrowed proto; correct the record*, removes the
change and explains what happened. The experiments made in between had the change on both sides of
every comparison, so their results stand. The tag `vm-redesign-2` points at the state actually
measured as step 2.

The lesson isn't really about git. When the result of a step matters, check that the step happened
(here, the diff against the expected tree) before building on it.

## The result

Best of four, alternating `vm-redesign-base` with the final state, release, compiled mode:

| | base | redesign | |
|---|---|---|---|
| `loop` | 54.9 ms | 48.5 ms | 1.13× |
| `let-heavy` | 48.2 ms | 44.3 ms | 1.09× |
| `fib` | 9.0 ms | 8.4 ms | 1.07× |
| `closures` | 54.0 ms | 51.1 ms | 1.06× |
| `strings` | 35.0 ms | 34.0 ms | 1.03× |
| `lists` (`map`/`filter` with small functions) | 18.5 ms | 19.7 ms | 0.94× |
| eeditor's snippet examples | | | 1.00–1.10× |
| one compiled evaluation (read, compile, run) | | | 1.08–1.10× |
| load eeditor's snippet files, compiled | 73.5 ms | 66.8 ms | 1.10× |

The interpreter column of the same runs stayed at 0.96–1.04×, as it should, since nothing here
touches it.

## What it says about the next step

Four experiments on the VM's shape gave about 10% on call-heavy code. Two of them gave nothing,
and the most promising-looking one (superinstructions) gave nothing at all. The profile is now
flat. Dispatch, calls, frames and dropping values each take their share, and there's no single
cost left to remove.

The bigger gains left are bigger changes:

- **Values that fit in a machine word.** A `Value` is 32 bytes, and every push, pop and copy moves
  all of it, with a check of its kind to decide whether it holds a reference count. Packing
  numbers, booleans and pointers into 8 bytes ("NaN-boxing", as LuaJIT and several JavaScript
  engines do) changes what every instruction costs. It also touches every line of Rust that looks
  inside a `Value`.
- **A register machine.** Instead of pushing operands and popping them, instructions name the slots
  they read and write: `(add r2 r0 r1)`. Lua moved from a stack VM to a register VM for this
  reason. It means a new instruction set and a new back end for the compiler, and so a new
  bootstrap. Chapter 10's procedure is how the compiler would cross over to it.
- **`map` and `filter` inside the VM.** The one regression here comes from builtins calling compiled
  functions by starting a fresh VM run for each element. Letting them call back into the running
  VM would remove that cost entirely.

Each is a project of its own. The tools from this chapter are what will measure them.
