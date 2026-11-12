# 5. Measuring

*Code: `examples/bench.rs`. Run it with
`cargo run --release --example bench -- <eeditor>/src/snippets`.*

## The question a compiler has to answer

A compiler that makes code slower is a liability, however elegant. So before the engine switches
from interpreting to compiling, we measured, and the first results were humbling.

## The benchmark

`examples/bench.rs` runs each workload on two engines. One interprets it. The other loads the
compiler from `boot/compiler.image` and compiles every definition. Both then call the same
`(bench-run)` the same number of times, and the results must agree.

There are two kinds of workload:

- **Synthetic**: naive recursion (`fib`), a tight `loop`, `map`/`filter`/`reduce` over a range,
  a `let`-heavy arithmetic function, closures created in a loop, and string building.
- **Real**: the snippet files that ship with the eeditor app (`zz-math`, `zz-text`, `zz-date`,
  `zz-crypto`…). Their comments document example calls (`;; (zzromanos 2024) → "MMXXIV"`). The
  benchmark collects those calls and runs each file's examples as one workload. It skips files
  that reach the network, because a benchmark of HTTP measures the network.

Some workloads draw random numbers or read the clock (a password generator, a timestamp), so
their results can't agree between two runs. The benchmark says so instead of hiding it.

## First result: slower

The first run, straight after the bootstrap, compared like this (release build):

| workload | compiled vs interpreted |
|---|---|
| fib | 1.07× |
| loop | 0.73× |
| lists | 0.74× |
| let-heavy | 0.66× |
| closures | 0.75× |
| eeditor snippets | 0.79×–0.98× |

Compiled code was up to a third *slower*.

## Where the time went

A profile of a compiled loop (macOS `sample`) showed:

| | share |
|---|---|
| the VM's dispatch loop | ~30% |
| copying and dropping values | ~28% |
| variable lookup | ~15% |
| calling builtins | the rest |

Variable lookup, the cost we had expected to dominate, was a minor one. The machine was paying
for things the interpreter had already learned to avoid:

1. **Every instruction was cloned** before it ran, because `load`, `def` and `set` carried their
   symbol inside the instruction.
2. **Every call allocated a vector** for its arguments. The interpreter passes up to four
   arguments to a builtin on the Rust stack and allocates nothing.
3. **Every `recur` was an error.** It went out through the machine's error path to find its
   loop, even when the loop was in the same frame.
4. **The function being called was cloned** to get at it.
5. **Each binding copied its value**: `(def x) (pop)` cloned the value into the environment,
   then dropped the original.
6. **`map` over a compiled function built a new machine per element**: a fresh stack and frame
   list each time.

## What we changed

Each fix was small, and the differential test (471 snippets in four setups) re-ran after every
one:

1. Instructions refer to symbols by index into the proto's symbol table, so an instruction is a
   plain copy.
2. Arguments are passed from where they lie on the machine's stack; a builtin is called right
   there.
3. A `recur` whose loop is in the same frame rebinds and jumps directly. Only a `recur` that has
   to travel to a caller's loop takes the error path, as in the interpreter.
4. A builtin is called through a reference, with no clone.
5. The assembler fuses `(def x) (pop)` into one instruction, `def-pop`, that moves the value.
6. Finished machines are kept and reused by the next run.

## Second result: even

| workload | compiled vs interpreted |
|---|---|
| fib | 0.89–1.09× |
| loop | 1.07–1.11× |
| lists | 0.78–0.88× |
| let-heavy | 0.98–1.03× |
| closures | 0.93–0.98× |
| strings | 1.01–1.05× |
| eeditor snippets | 0.87–1.16×, most around 1.05× |

(Ranges are across runs; a few percent is noise.)

Compiled code is now as fast as interpreted code, a little faster on most of the real snippets.
That makes switching safe, but it isn't yet a reason to switch.

## Why not faster

Both engines keep variables in the same environments, chains of scopes searched by name. That
was deliberate: it is what makes compiled code *mean* exactly what interpreted code means, which
the differential test depends on. It also means every function call allocates a scope, and every
variable is found by searching, in both engines. And both engines share every builtin, which is
where much of the remaining time goes.

The way past this is the classic one: the compiler resolves each local variable to a numbered
**slot**, and the machine keeps locals in its own stack instead of in an allocated scope. A call
then allocates nothing, and reading a variable is an index.

But it changes something visible. Today, `eval` called inside a function can see that function's
local variables by name:

```lisp
(defn f (x) (eval 'x))
(f 42)   ; → 42
```

With slots, the name `x` no longer exists at run time, so `eval` can't find it. Most languages
draw the line there (in JavaScript, Python or Clojure an `eval` doesn't see the caller's
locals, or sees them only in special cases), but EELisp currently doesn't. Deciding where that
line goes is a decision about the language, not about the compiler, so it comes before the code.
[Chapter 6](06-slots.md) makes that decision and builds the slots.
