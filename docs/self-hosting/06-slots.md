# 6. Locals in slots

*Code: `src/compiler.eel` (the context and resolution functions), `src/vm.rs` (slot mode).
Tests: the slot cases in `tests/compiler.rs`, `clean_break_eval_runs_at_the_top_level` in
`tests/lang.rs`.*

## Where we left off

At the end of the last chapter, compiled code was as fast as interpreted code and no faster.
Both engines kept every variable in the same place: a chain of scopes, each a small table
searched by name, with a new scope allocated for every call. The way past that is old and well
understood. The compiler works out, once, where each variable lives, and the machine keeps local
variables in numbered **slots** in its own frames.

## First, a decision about the language

Slots have a visible consequence. Once a local variable is slot 0 of a frame, its *name* is gone
at run time. Before this chapter, EELisp let `eval` see the caller's locals:

```lisp
(defn f (x) (eval 'x))
(f 42)   ; → 42, before this chapter
```

We looked at how other languages handle this:

- **Scheme** (R7RS) gives `eval` an explicit environment argument. It never sees lexical
  variables.
- **Common Lisp** evaluates in the "null lexical environment": globals only.
- **Clojure** compiles `eval`'s form in the current namespace, with no locals.
- **JavaScript** lets a *direct* `eval(…)` see locals, and engines like V8 keep slow,
  dictionary-like scopes for exactly the functions that contain one.

The Lisp family's rule is simpler, and it is the one EELisp now follows: **`eval` runs at the top
level.** It sees every global definition and none of the caller's locals. That is a clean break,
so it is documented (`docs/03-language.md`), it has its own test, and it was committed on its
own, before any compiler change:

```lisp
(def x "global")
(defn f (x) (eval 'x))
(f "local")                        ; → "global"
```

Code that needs a value in an `eval` puts the value in the code: `(eval (list '+ n 1))`. Agenda
rules and sheet formulas were never affected: they evaluate with their own environments from
Rust, not through the `eval` builtin.

## What the compiler knows

The compiler used to carry a plain list of local names. Now it carries a **context** describing
the program at the point being compiled:

- **the function** being compiled: how many slots it has handed out, which outer variables it
  captures, and which function it is inside;
- **the blocks** around this point, innermost first, each mapping names to slots. A function's
  parameters are a block; so are `let`, `loop`, `catch` and `for-each`.

A variable reference is resolved against that context, giving one of three answers:

| Answer | Instruction | Meaning |
|---|---|---|
| a slot of this function | `(load-local 2 x)` | read slot 2 of this frame |
| a slot of an enclosing function | `(load-cap 0 x)` | read the closure's captured variable 0 |
| not bound locally | `(load x)` | the global `x`, by name |

Here is a small function before and after:

```lisp
(defn dist (x1 y1 x2 y2)
  (let (dx (- x2 x1) dy (- y2 y1))
    (+ (* dx dx) (* dy dy))))
```

Its parameters are slots 0–3 and the `let` adds slots 4 and 5. The body reads them by number
(`(load-local 4 dx)`), and a call to `dist` allocates no scope at all. Only `-`, `+` and `*` are
looked up by name, because they are globals.

## Closures: sharing a variable

A closure that uses an outer variable has to *share* it, not copy it. This counter only works if
the closure and the `let` that made `n` see the same variable:

```lisp
(defn make-counter () (let (n 0) (fn () (set! n (+ n 1)) n)))
```

The compiler records, for each function, what it captures and where from: one of its maker's
slots, or one of its maker's own captures (for a function nested three deep). When the machine
creates the closure, each captured slot turns into a **cell**: a small shared box that the frame
and the closure both point to. From then on, `set!` in either place changes the same value.

## Keeping the meaning exactly

This was the hard part. The tree-walking interpreter has precise behaviors that a naive slot
compiler would quietly change. Each one was turned into an edge case in the differential test
before it was fixed:

- **`let` makes a fresh binding every time it runs.** A loop that creates closures over a
  `let` variable gives each closure its own: `(20 10 0)`, not `(20 20 20)`. So entering a `let`
  replaces the slot's contents (`init-local`) instead of writing through a shared cell.
- **`recur` and `def` rebind the same binding.** A closure created in an early iteration of a
  `loop` sees later values of the loop variable, as in the interpreter. So those write *through*
  the cell (`def-local`, and the `loop-slots` restart).
- **A `def` that hasn't happened yet means the global.** In
  `(defn maybe-def (flag) (if flag (def z "local z")) z)`, the final `z` is local only if the
  `def` ran. A slot starts *unbound*, and reading an unbound slot reads the global of the same
  name: `(list (maybe-def true) (maybe-def false))` → `("local z" "global z")`.
- **A block's `def`s start over each time the block runs.** A `def` inside a `let` inside a
  loop doesn't leak into the next iteration, because the compiler clears those slots
  (`clear-local`) at the start of the block.
- **A local function can call itself.** `(defn helper …)` inside a function binds its slot
  *before* compiling the body, so the body's reference to `helper` is the local slot, not a
  global.

The differential test now runs 517 snippets: 442 from the engine's test suites and 75 edge cases,
38 of them about slots. They agree in all four setups (interpreted compiler, compiled prelude,
self-compiled compiler, compiler from the image).

## One machine, two modes

Hand-written bytecode from chapter 2 still works. A proto with a `:slots` field runs in slot mode;
one without keeps the named scopes. The image shows the new form:

```lisp
(proto :name compiler-new-slot :params (fx) :source (…) :slots 2 :captures () :code (…))
```

The image grew from 36 to 56 protos: the compiler's own functions now include the context and
resolution code. It is still a fixed point. Stage 0 and stage 1 produce it identically.

## Results

| workload | before slots | with slots |
|---|---|---|
| fib | 0.89–1.09× | 1.08× |
| loop | 1.07–1.11× | 1.39–1.42× |
| let-heavy | 0.98–1.03× | 1.34–1.35× |
| strings | 1.01–1.05× | 1.19–1.21× |
| closures | 0.93–0.98× | 1.02–1.04× |
| lists | 0.78–0.88× | 0.95–0.96× |
| eeditor snippets | 0.87–1.16× | mostly 1.1–1.3× |

The compiler compiling itself got faster too: in the bootstrap, the compiled compiler (stage 1)
took 23 ms against 35 ms for the interpreted one.

## What's left

Compiled code is now faster than interpreted code on almost everything, but not dramatically:
1.1 to 1.4 times. `lists` is a little under 1×, because there the time goes into `map`, `filter`
and `reduce`, builtins both engines share. `fib` and `closures` are call-heavy and gain least. The
remaining cost is spread thinly: the dispatch loop, calls, copying values, and looking up globals
like `+` by name. None of it stands out.

Two known gaps from this step, added to the roadmap:

1. A `def` that comes *after* a closure in the same local scope isn't seen by that closure. The
   interpreter's closure would find it, because the closure keeps the whole scope; the compiled
   closure resolved that name, when it was compiled, to the global.
2. A malformed form left to the interpreter (`(set! (f) 1)`, say) no longer sees local
   variables, so its error may be "undefined symbol" rather than the syntax error.

The next step is the one the roadmap has been building toward: making compile-and-run the
engine's default `eval`.
