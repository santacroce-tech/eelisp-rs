# A register machine for EELisp

*Branch `exp/register-vm`, tag `register-vm-base`. Prototype: `examples/regvm_proto.rs`.*

## Why

Chapter 15 found that compiled mode runs faster than the interpreter everywhere, but reading
code in costs 100–700 ns per character, because the reader is EELisp and every instruction it
runs has a price. Small changes to the reader gave 5–15%. What would change that price is
the machine.

A stack machine spends most of its instructions moving values: `(< n 2)` is four instructions
(load `<`, push `n`, push `2`, call). In a register machine, instructions name where their
operands are, so that's one: `(call2 r1 < r0 #2)`. A compiled function's arguments are left in
the registers its frame starts at, so a call copies nothing into a new frame. Lua made this move
in version 5 for the same reasons.

## What the prototype says

Hand-written register code, run with the same global cache, the same builtins and the same fast
path for arithmetic as the real VM:

| | stack VM | registers | | with real costs | |
|---|---|---|---|---|---|
| `fib 25` | 18.7 ms | 10.0 ms | 1.86× | 12.0 ms | 1.55× |
| a loop of 1M | 50.6 ms | 24.6 ms | 2.05× | 35.3 ms | 1.43× |
| the reader's whitespace loop | 30.4 ms | 20.8 ms | 1.46× | 23.2 ms | 1.31× |

The first runner leaves out what the real machine must do: slots that aren't bound yet (whose
name then means the global), slots captured by closures (cells), `loop` marks and `try`
handlers, `def` into a named scope, closures, macros, errors. Chapter 14 found that cutting
dispatches alone (superinstructions) gained nothing, so the obvious worry was that the gain came
from those omissions, not from registers.

The second runner (`run_real`) pays the costs the stack VM's frames and slots carry: registers
that can be unbound or hold a cell, checked on every read and write; a frame holding its
environment, captured cells and mark/handler bases; a callee entered through its function value,
whose closure becomes the frame's environment. Most of the gain survives: 1.3–1.55×. What's left
out (marks for `loop`, which the hand-written loop replaces with a jump) would cost the loop
some of its 1.43×.

## Steps

Each step is a commit (or several) on this branch, measured with `scripts/ab.sh`,
`examples/front_end.rs` and `examples/read_kinds.rs`, and written up in chapter 16.

1. **The machine, beside the old one.** New instructions in `src/vm.rs` (or a new `src/rvm.rs`),
   assembled from EELisp data like the current ones: `move`, `const`, `global`, `call` (callee
   and arguments in consecutive registers), `call2` with register-or-constant operands and the
   fast path, `tail-call`, `jump`, `jump-if-not`, `return`, plus the slot, cell, closure, `def`,
   `loop`/`recur`, `try` and `dict` instructions the current set needs. Both machines run
   side by side; a proto says which code it carries.
2. **A back end in the compiler.** `src/compiler.eel` already resolves every local to a slot.
   Registers are those slots plus temporaries allocated like a stack: the temporaries for a
   call's arguments sit just above the live ones, so the callee's frame starts there. The
   differential test (517 snippets, four setups) is the referee, as it was for the first back end.
3. **The crossing.** The committed image runs on the stack machine and compiles the new source,
   which emits register code (generation 1, stack code that writes register code). That compiler
   compiles itself (generation 2, register code), and once more (3). 2 and 3 must match, and the
   interpreter must make the same image. That is chapter 10's procedure, used for the purpose it
   was built for.
4. **Retire the stack machine**, once nothing loads stack code: the old instructions and their
   assembler go, and the image is register code only.

## Status

- **Step 1 done** (tag `register-vm-1`, chapter 16). Not a second machine: register instructions
  *in* `src/vm.rs`, the slots of a slot-mode proto being its registers, a destination either a
  register or the stack, so register and stack code mix in one proto. That lets step 2 switch
  one construct at a time, and the crossing (step 3) needs nothing special: the committed image
  can already run the new instructions. Hand-written register code on the real machine: `fib`
  1.24×, a loop 1.79×, the reader's whitespace loop 1.01×. Stack code within 0.96–0.99× of
  before, after moving the register calls' slow paths out of the dispatch loop.
- **Found:** registers alone don't help loops whose cost is in builtin calls (`nth`,
  `dict-get`), which is the reader. A cheaper dict hash: no gain, reverted. **Intrinsics** (tag
  `register-vm-2`): `call2` runs `nth` and `dict-get` in place with borrowed operands, giving
  hand-written register code `fib` 1.57×, a loop 2.66× and the whitespace loop 1.89×, with
  stack code unchanged.
- **Step 2 done, step 3 with it** (tag `register-vm-3`, chapter 17). The compiler emits
  `call2` for two-argument builtins, `call-global` for known functions and `call-at` (function
  in a register, then its arguments, as Lua's CALL) for anything else; temporaries `(t k)`
  after the variables; `if`/`cond` test registers. The committed image built it directly:
  generations 2 and 3 identical, the interpreter's image the same. Compiled code: loops and
  tail calls 1.9×, calls 1.35×, `fib` 1.34×; reading 7% faster; compiling 18% slower.
- **Then** (tag `register-vm-4`): a `count-calls` feature and `examples/count_calls.rs`
  profile EELisp at its own level. The compiler's extra cost was extra work (10% more calls),
  not slower code. Intrinsics for `=`/`!=` and a `call1` with intrinsics for `head`, `length`
  and the predicates: reading 43.0 → 38.0 ms, compiling back to 16.2 ms. Cheaper boxes kept;
  a special-form table and a shared call1/call2 arm measured and reverted.
- **Next:** `let`/`or`/`recur` in registers; the cost of a call itself (frames, slot churn on
  register calls, `Value` clone/drop, a quarter of every profile). Step 4 (retiring stack code)
  waits until stack code is rare.

## What has to stay true

- Every program gives the same result as the interpreter (the five known differences aside).
- `(source f)`, error messages and error kinds are unchanged.
- `vm-assemble` / `vm-run` still take bytecode as EELisp data that can be read and printed.
- The image is a fixed point after the crossing.
