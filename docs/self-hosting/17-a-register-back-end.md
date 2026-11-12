# 17. A register back end

*Branch: `exp/register-vm`. Tags: `register-vm-2` (where it starts), `register-vm-3`. Tools:
`scripts/ab.sh`, `examples/front_end.rs`, `examples/read_kinds.rs`, `examples/bench.rs`, and a
new one, `examples/fib_variants.rs`.*

Chapter 16 gave the machine register instructions and showed, with code written by hand, what
they could do. This chapter is the compiler learning to write that code itself. It's also step 3
of the plan, the crossing, which turned out to need nothing special.

## What the compiler emits

Only calls changed. Everything else (`let`, `loop`, `try`, closures, quasiquote) is still stack
code, and the two mix: a register result is pushed for stack code to use, and a stack result is
popped into a register.

- **Temporaries** are written `(t k)`. The assembler places them after all of a function's
  variables (`:slots n :temps m`), so every argument register is above every variable. The
  compiler hands them out like a stack: an expression computed into `(t d)` uses only `(t d)`
  and the ones above it. A callee entered at its argument registers owns everything above them
  while it runs, so it can never overwrite a value the caller still needs.
- **A builtin of two arguments** becomes a `call2` whose operands are read where they are: a
  local `(local i name)`, a captured variable, a constant, or a temporary the operand was
  computed into. This is where the arithmetic and the intrinsics of chapter 16 run.
- **A call of a known function** becomes `call-global`, the arguments computed straight into the
  callee's first registers; in tail position, `tail-call-global`.
- **`if` and `cond` tests** jump on a register (`jump-if-not`) instead of going through the stack.
- A **variable or constant in tail position** is returned where it is (`return-value`).

The compiler learns what a name means from `vm-global-kind`, which now also answers "builtin" and
"function". The compiler in the committed image knows only the old answers ("macro", "raw",
"table-first"), and treats anything else as an ordinary call. That's what made the crossing
ordinary.

## The crossing

Chapter 10's procedure, used for what the plan built it for. The committed image (stack code)
compiled the new compiler's source; that generation, stack code that writes register code,
compiled the source again; and once more. Generations 2 and 3 were identical, and the
interpreter, starting from the source alone, made the same image. It worked the first time, and
after every later change to the back end. The differential test (517 snippets against the
interpreter, in four setups) passed on its first run as well.

## What measuring changed

The first version was slower to compile and gained little on reading. Each correction below was
found by measuring or by reading what the compiler emitted, and each is its own commit.

**Not every call is worth registers.** The first version sent every call of a builtin through
registers. For `(length x)` that meant moving `x` into a temporary, and then `call-global`
copying it back onto the stack, because a builtin takes its arguments as a slice. That's more
work than the stack code it replaced. Only two kinds of call gain: a two-argument builtin
(`call2`, with its fast paths) and a compiled function (entered in place). Every other builtin
call stays stack code.

**A copy only when it's needed.** In `(= class reader-space)`, the first operand was copied into
a temporary in case computing the second one changed it. That rule is right in general: in
`(+ x (do (set! x 5) 1))` the stack code reads `x` first. But reading a global can't change
anything, so the copy is now made only when the second operand is computed by code.

**Returning a register costs a slot.** Returning a builtin's result straight from its register,
instead of pushing it and falling through to the return, measured as *slower*: map/filter
0.89×, calls 0.94×. The register is a temporary, and a temporary is a slot that every call of
the function fills when it starts and drops when it ends. In `(fn (x) (* x 3))` that's a third
more slots. Reverted for builtin calls; variables and constants, which need no temporary, are
still returned in place.

**The function that doesn't exist yet.** `fib` from `defn` ran in 16.5 ms. The same instructions
assembled by hand ran in 11 ms (`examples/fib_variants.rs`, which also showed that the way of
returning made no difference). The compiled code had the answer: while `(defn fib …)` is
compiled, `fib` isn't defined yet. The compiler didn't know it was a function, so the recursive
calls fell back to stack code. That's every self-recursive function, and every call of a
function defined further down a file, which is most of the reader and of the compiler itself.

Compiling such calls with `call-global` would have changed what a program does. Stack code reads
the function *before* computing the arguments, so calling a name that isn't defined fails before
any argument's side effects; `call-global` reads it after. Lua's answer fits: the function goes
into a register first, the arguments into the registers after it, and `(call-at d r n)` calls
whatever `r` holds. It keeps the stack's order exactly and works for any callee: a function
defined later, a local variable, a computed value. `fib` went from 16.5 to 13.3 ms.

**The dispatch loop, once more.** `call-at`'s arm in the loop cost `loop 1M` 5% and `calls` 7%,
with identical bytecode for both: neither calls anything `call-at` handles. Folding the four
register calls into one arm made it worse (0.87×), and was reverted. Running the same A/B twice
gave the same numbers both times. So code layout's effect is deterministic for a given binary,
not noise: a change to one instruction moves the speed of others, and only an A/B shows it.

## Where step 2 leaves things

Against the start of this chapter (`scripts/ab.sh`, compiled):

| | before | after | |
|---|---|---|---|
| `fib 25` | 18.4 ms | 13.7 ms | 1.34× |
| closures 200k | 39.6 ms | 36.0 ms | 1.10× |
| calls 300k | 29.1 ms | 21.6 ms | 1.35× |
| tail calls 300k | 20.4 ms | 10.7 ms | 1.91× |
| loop 1M | 50.3 ms | 26.5 ms | 1.90× |
| map/filter 100k | 9.4 ms | 7.4 ms | 1.27× |

eeditor's snippets, run compiled (`examples/bench.rs`): `loop` 1.97×, `let-heavy` 1.41×, `fib`
1.35×, `closures` 1.12×, `lists` 1.05×. The snippets that are mostly builtin calls are unchanged
(0.96–1.00×), as expected after the first correction.

Getting code in (`examples/front_end.rs`, the snippet files):

| | before | after |
|---|---|---|
| read | 45.7 ms | 42.7 ms |
| compile | 15.6 ms | 18.4 ms |

Running code got up to twice as fast. Reading gained 7%. Compiling lost 18%: the compiler now
works out, for each call, what its head means, which operands can be read in place, and where
each value goes, and that work is EELisp running on this VM.

So the front end, the cost chapter 15 found, is not yet where it needs to be. What's left there:

- **The compiler's own cost.** It asks the same questions of the same forms more than once
  (`compiler-operand` on a form, then again on its parts), and much of its analysis goes
  through dicts and boxes. Profiling it the way chapter 8 did is the next measurement.
- **Stack code that remains in the reader's loops.** `let` bindings go through the stack
  (`init-local` pops), and so do `or`, `and` and `recur`. Each is a construct the back end can
  learn, one at a time, with the differential test as referee.
- **More intrinsics.** The reader's loops call `length`, `str-sub`, `=` on strings and `append`,
  which are still general calls.
