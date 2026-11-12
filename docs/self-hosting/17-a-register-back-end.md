# 17. A register back end

*Branch: `exp/register-vm`. Tags: `register-vm-2` (where it starts), `register-vm-3`, `register-vm-4`. Tools:
`scripts/ab.sh`, `examples/front_end.rs`, `examples/read_kinds.rs`, `examples/bench.rs`, and new
ones: `examples/fib_variants.rs`, `examples/prof_compile.rs`, and `examples/count_calls.rs` with
the `count-calls` feature.*

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

## The compiler's own cost

Chapter 8 profiled the compiler by counting its calls, with a counter that was thrown away
afterwards. It's now a cargo feature, `count-calls`, off by default. It counts every frame the VM
enters and every builtin it calls, by name, and `examples/count_calls.rs` prints the counts for
reading and for compiling a directory of files. The same counter was put into the commit from
before this chapter to compare like with like:

| compiling eeditor's snippets | calls per form | time (`prof_compile`) |
|---|---|---|
| before the register back end | 2,134 | 15.5 ms |
| after | 2,355 | 17.5 ms |

Ten percent more calls, twelve percent more time. Sampling both at the machine level gave the
same picture: the call machinery takes about the same share in each, and everything else grew in
proportion. The register back end doesn't make the compiler run slower; it makes it *do more*.
For every call it compiles, the compiler now works out what the head means, which operands can be
read in place, and which temporary each value goes to. And the compiler's own code is mostly
calls to builtins on lists and dicts, the kind registers don't speed up.

The counts show where its calls go. `list` and `dict-get` are a tenth each: instructions are
built as lists, and contexts are dicts. Then come `head`, `=`, `symbol?` and `empty?`, and 23,000
calls of anonymous functions. Two changes were tried:

- **Boxes.** A context's mutable counters are boxes, closures over a variable. A box was
  `(fn (. args) (if (empty? args) val …))`, so every read built a list for the rest parameter
  and called `empty?`. A box never holds nil, so it is now `(fn (x) (if (nil? x) val …))`.
  Compiling went from 18.4 to 17.7 ms, about 4%. Kept.
- **Special forms through a table.** `compiler-special` compared the form's name with each
  special form in turn (`(= h 'quote)`, `(= h 'if)`…), and on symbols every `=` is a general
  builtin call. A dict of handlers made that one lookup and one call: `=` calls fell from 29,700
  to 24,200, and compiling went from 17.7 to 17.9 ms. No gain; committed with its numbers and
  reverted.

So the compiler is like the builtin-heavy snippets: no single call is expensive, each costs
roughly 50 ns, and it makes 2,300 of them per form. What would change that is the cost of a call
and of copying values: `Value::clone` and dropping values are a quarter of every profile in this
chapter. Or the compiler could do less, for instance by asking what a name means once per name
rather than once per use.

## Back to the reader, with counts

The same counter, on reading: 1.14 million calls for the snippet files, about ten per character.
A fifth of them were `=` (236,000). With two numbers it takes the arithmetic fast path, but the
reader also compares characters as strings, and the compiler compares symbols. Those were
general builtin calls. `=` is value equality, which can't fail, so as an intrinsic in `call2` it
covers every case. `!=` came with it.

| | before | `=`, `!=` | `call1` |
|---|---|---|---|
| read the snippet files | 42.6 ms | 38.3 ms | 37.7 ms |
| compile them | 17.7 ms | 17.1 ms | 16.1 ms |
| strings, per character | 260 ns | 217 ns | 211 ns |
| symbols, per character | 368 ns | 347 ns | 331 ns |

The next counts down were one-argument builtins: `head` (67,000 in reading), `length`, and in the
compiler `nil?`, `empty?`, `not`, `symbol?`, `list?`. A new instruction, `(call1 d f x)`, reads
its operand in place and runs these seven inside it. Anything else, or a value of a kind they
don't cover (`head` of a string), goes to the builtin and fails as it does. That brought
compiling back to where it was before the register back end (15.6 ms then, 16.1 ms now), with
reading 12% faster than then.

Each intrinsic has a test that runs it against its builtin on every kind of value, comparing
results and errors.

**Layout, a third time.** `call1`'s arm in the dispatch loop cost the call benchmark 3–5%, a
benchmark that hardly uses it. Sharing `call2`'s arm instead (a `call2` with no second operand)
moved the cost: 0.94× on `loop 1M`, whose additions now pay the extra check, and about even
elsewhere. Neither version was clearly better, so the clearer one stays. The pattern of this
chapter and of chapter 14 is now plain: in a dispatch loop this size, adding an instruction
costs other instructions a few percent, by an amount no one can predict and only an A/B shows.
Each new instruction has to earn more than that.

Against `register-vm-3`: reading 43.0 → 38.0 ms, compiling 18.4 → 16.2 ms, running code within
0.97–1.05×.

