# 16. Registers

*Branch: `exp/register-vm`, on top of `exp/compiled-default`. Tags: `register-vm-base` (the
prototype and the plan), `register-vm-1` (register instructions in the machine). Plan:
[docs/plans/register-vm.md](../plans/register-vm.md). Tools: `examples/regvm_proto.rs`,
`scripts/ab.sh`, `examples/prof_one.rs`.*

Chapter 15 ended on one fact: reading code in costs 100–700 ns per character, because the reader
is EELisp, and every instruction it runs on this VM has a price. The answer it chose was a faster
machine. This chapter is the first half of that: what a register machine buys, measured before
the compiler is touched.

## The idea

A stack machine moves values to compute with them. `(< n 2)` is four instructions: load `<`, push
`n`, push `2`, call. The call finds its arguments on the stack, copies them into the callee's
frame, and the result is pushed back. A register machine names where values are. The same test is
one instruction, `(call2 1 < (local 0 n) (const 2))`: register 1 gets `<` applied to the local `n`
and the constant 2. And a compiled function's arguments can be computed straight into the
registers its frame will start at, so a call copies nothing. Lua moved from a stack VM to this in
version 5.

## Is it worth it?

Chapter 14 found that fusing pairs of instructions (superinstructions), which removed a fifth of
all dispatches, gained nothing. So fewer instructions alone is not a reason. Before writing a
back end, `examples/regvm_proto.rs` ran three programs written by hand in register code, against
the same programs compiled for today's VM, both using the same global cache, builtins and
arithmetic fast path:

| | stack VM | prototype | with the real costs |
|---|---|---|---|
| `fib 25` | 18.7 ms | 10.0 ms (1.86×) | 12.0 ms (1.55×) |
| a loop of 1M | 50.6 ms | 24.6 ms (2.05×) | 35.3 ms (1.43×) |
| the reader's whitespace loop | 30.4 ms | 20.8 ms (1.46×) | 23.2 ms (1.31×) |

The first prototype left out what the real machine has to carry, so the worry was that the gain
came from what was missing. The second runner put it back: registers that may be unbound (a
variable not yet defined means the global of that name) or hold a cell shared with a closure,
checked on every read and write; frames holding their environment and captured cells; callees
entered through their function value. Most of the gain survived. Worth building.

## One machine, not two

The plan said "a new machine beside the old one". What was built is narrower and safer: register
instructions *in* the existing machine. A slot-mode proto's slots, where chapter 6 put local
variables, are also its registers; temporaries are more slots after the variables. An operand is a
register, `(local i name)` (read exactly like `load-local`, including the global fallback),
`(cap i name)`, or `(const v)`. A destination is a register or `push`, the stack. So register and
stack code mix in one proto:

```lisp
(call2 push + (local 0 a) (const 1))   ; a register instruction feeding stack code
(load +) (const 2) (const 3) (call 2) (pop-to 1)   ; stack code feeding a register
```

That changes the next step. A compiler can switch to registers one construct at a time, with the
differential test as referee after each, instead of all at once. And the crossing of chapter 10
stays ordinary: the committed image can already run the new instructions.

`fib` in register code:

```lisp
(proto :name "fib" :params (n) :slots 4 :code (
  (call2 1 < (local 0 n) (const 2))
  (jump-if-not 1 more)
  (return-value (local 0 n))
  (label more)
  (call2 2 - (local 0 n) (const 1))
  (call-global 1 fib 2 1)        ; argument in register 2: fib's frame starts there
  (call2 3 - (local 0 n) (const 2))
  (call-global 2 fib 3 1)
  (call2 1 + 1 2)
  (return-value 1)))
```

Eight instructions run per call, against twenty for the stack code the compiler emits today.

The one new obligation is in `call-global`. The callee's frame starts at the caller's argument
registers, so while it runs, the caller's registers from there up are the callee's. On return
(and when an error lands in a caller's `try`, or a `recur` travels back to it), the caller gets
them back, unbound. The compiler must put nothing live there, and an argument register must not
be a slot a closure captured. `tests/registers.rs` holds the machine to all of it: 14 tests,
covering recursion, loops, constant-space tail calls, calls to every kind of callee (builtin,
interpreted, compiled with registers, compiled stack code, rest parameters, too few arguments),
unbound locals, captures, an error landing in a `try` with the caller's registers back, deep
recursion, and malformed code refused by the assembler.

## What the real machine gives

The same three programs, assembled and run by the real machine, `src/vm.rs`:

| | stack code | register code |
|---|---|---|
| `fib 25` | 19.7 ms | 15.9 ms (1.24×) |
| a loop of 1M | 52.6 ms | 29.4 ms (1.79×) |
| the whitespace loop | 29.9 ms | 29.5 ms (1.01×) |

Getting there took four commits, each measured:

1. **The instructions themselves.** First run: `fib` 1.13×, the loop 1.43×, whitespace 1.06×.
   The profile showed `Vec::resize`, which isn't inlined, refilling slots on every call and
   return. An inlined loop instead gave `fib` 1.20×.
2. **Not copying the builtin.** Every `call2` copied `<` out of its global, a non-inlined clone
   of a `Value`, and dropped the copy afterwards, even when the fast path never called it. Reading
   it in place, which is safe because nothing runs between the read and the arithmetic, gave
   `fib` 1.41× and the loop 1.91×.
3. **The cost to stack code.** `scripts/ab.sh` against the commit before showed today's stack
   code **7–19% slower**, and stack code is what everything still runs. Two causes were found in
   turn. First, every stack call's result went through a check of where it should go, and every
   return checked whether the caller's registers needed restoring. Making those paths what they
   were (a register destination only for register calls) left a loss of 5–13%. It stayed on
   `loop 1M`, which makes no calls to compiled code at all.
4. **The dispatch loop's size.** As in chapter 14, the cost was the loop itself: more code in
   one function changes how all of it compiles. Moving the slow paths of `call2` and all of
   `call-global` into out-of-line methods brought stack code back to 0.96–0.99× (within the
   noise for most lines). It cost register calls some of their gain: `fib` is 1.24× instead of
   1.41×.

## What it says

Registers pay where the machine itself is the cost: arithmetic, branches, calls between compiled
functions (1.2–1.8×). They give nothing on the whitespace loop, and that loop matters most,
because it is the reader's. Each turn of it calls `nth` and `dict-get`, and those calls cost the
same on both machines. The builtin is copied out of its global and dropped. Its arguments are
copied into an array. `nth` checks its arguments' kinds, and `dict-get` hashes a one-character
string with SipHash. Changing how operands reach a call doesn't change what the call costs.

So the reader needs more than registers. Two candidates, both measurable on their own:

- **Intrinsics for the builtins the reader lives on.** `nth`, `dict-get`, `length`, `=` on
  strings: done inside the instruction when the global is still the original builtin, as
  arithmetic already is. No copy of the builtin, no argument array.
- **A cheaper hash for dicts.** SipHash protects against crafted keys; a reader's class table and
  a compiler's environment don't need that. This helps both engines.

The compiler back end (step 2 of the plan) comes next either way. Calls and loops gain now, and
every gain to the instructions themselves lands on code that is already compiled to them.
