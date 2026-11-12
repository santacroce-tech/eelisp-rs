# 24. What the machine pays

*Branch: `exp/register-vm`. Tags: `register-vm-11` (where it starts), `register-vm-12`. Tools:
`scripts/ab.sh`, `examples/prof_one.rs`, Instruments' Time Profiler (`xctrace`) with `atos`, and
`examples/nanbox_proto.rs` as the yardstick.*

Chapter 23's prototype, with the same 32-byte values as the engine, ran `fib`, a counting loop
and the reader's whitespace loop twice as fast as `src/vm.rs`. Some of that is work the real
machine must do and the prototype doesn't: cells shared with closures, unbound slots meaning a
global, `try` handlers, errors. The question here is how much of it is just cost, and the first
suspect was named at the end of chapter 23.

## Redefinable arithmetic: not it

EELisp lets a program redefine `+` or `<`, so the real machine's `(< n 2)` is a `call2` that first
checks, through its global cache, that `<` is still the builtin it was: the same global scope,
the same layout of the globals, nobody changing them right now. The prototype has an `Lt`
instruction.

To bound what that check costs, it was switched off: one commit that trusts any filled cache
entry unchecked (unsafe if globals move, and only measured on a benchmark that defines everything
first), reverted in the next. The gain was 1–6%. Honouring redefinable arithmetic is cheap. Not
the gap.

## Profiling by source line

A sampling profiler only said that 64% (`fib`) to 83% (the loop) of the time is inside `step`, the
dispatch loop, which is one function. To see inside it, the profile was taken by address with
Instruments (8,500 samples), and each address mapped to its source line with `atos`, after
`dsymutil` had gathered the line tables. For the counting loop:

| where | samples |
|---|---|
| dispatching the next instruction | 1,359 |
| reading a `(local i name)` operand | 1,081 |
| `recur` rebinding the loop's slots, with `def_slot` and the drain | about 1,700 |
| checking that `=` and `+` are still the builtins | 711 |
| dropping values | 700 |
| `jump-if-not` reading its operand | 580 |

Two of those had plain, local causes, and both were fixed inside the instructions themselves, so
no new instruction was added to the dispatch loop (the lesson of chapters 14 and 17).

**`recur`.** Every turn of a loop collected the new values through a draining iterator, rebound
the slots through a general function, and copied the loop marker's environment into the frame: a
reference count up and one down, though in slot mode the environment never changes. Now each value
is popped straight into its slot, the environment is copied only when it actually differs, and
the handlers are truncated only when there are some to drop. `loop 1M` 1.12×, calls 1.10× (its
loop recurs too).

**Local operands.** `(local i name)` pointed into a side table holding the slot and the name, so
reading a local took two dependent loads before the slot itself. The name matters only when the
slot is unbound, to find the global of that name. A slot has exactly one name, since a name bound
again gets a new slot, so the operand now holds the slot number and each slot's name is kept in a
table read only on that slow path. Hand-written code that names one slot twice is now refused by
the assembler, and a test checks it. 1.01–1.06×.

## Where it stands

| `scripts/ab.sh` against `register-vm-11` | |
|---|---|
| `fib 25` | 1.05× |
| closures 200k | 1.05× |
| calls 300k | 1.15× |
| tail calls 300k | 1.04× |
| loop 1M | 1.14× |
| map/filter 100k | 1.03× |

Against the prototype, the loop's gap went from 2.1× to 1.8×; `fib`'s is still 2×. What `fib`
pays is calls. Each one takes a reference to the function, to its compiled body, to its
environment and to its captured cells, and gives them all back on return. Switching the dispatch
loop to another frame takes and drops one more. The prototype's frames are three numbers.
Making frames borrow instead of own is the next structural change. It has to be careful: a tail
call replaces the frame whose code is still running, and that code must not be freed under it.
