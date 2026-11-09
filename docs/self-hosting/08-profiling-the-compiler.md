# 8. Profiling the compiler

*Code: `src/compiler.eel`. The numbers come from `examples/bench.rs`, run against eeditor's
snippet files.*

## The problem

Chapter 7 left a clear cost on the table. In compiled mode, every evaluation compiles first, and
loading eeditor's snippet files took 54 ms compiled against 1.5 ms interpreted. The compiler is an
EELisp program, so speeding it up means profiling an EELisp program.

## The native profiler doesn't say much

A sampling profiler on the running engine (macOS `sample`) shows where the *machine* spends its
time: the VM's dispatch loop, its call path, copying and dropping values, `malloc` and `free`.
That's true, but it doesn't tell you which part of the compiler to change. Every EELisp function
looks the same from there: calls, values, allocation.

## Counting calls instead

What was needed was a profile at the level of the program, saying which EELisp functions are
called and how often. The VM can answer that with a few lines of temporary instrumentation: a
counter, keyed by function name, bumped every time a frame is entered or a builtin is called.
We compiled every top-level form of the snippet files (157 of them) and printed the counts:

```
157 forms, 1064314 calls (6779 per form)
  178427  builtin empty?
  169962  builtin head
  114036  builtin =
  113429  builtin tail
   87950  <anon>
   61988  compiler-lookup
   50732  some?
   49665  builtin dict-get
   …
```

**6,779 function calls to compile one form.** And the list reads like a diagnosis. `empty?`,
`head`, `tail` and `=` are the steps of walking a list. `some?` is the prelude's "is any element
like this?". `<anon>` is the anonymous function passed to it. `compiler-lookup` is the
compiler's own list walk.

## Two culprits

**Is this a special form?** For every form it compiles, the compiler asks whether the head is one
of the twenty special forms (`if`, `let`, `defn`…). It asked like this:

```lisp
(defn compiler-member? (x names) (some? (fn (n) (= n x)) names))
(compiler-member? h compiler-specials)   ; a 20-element list
```

That's up to twenty rounds of a recursive function calling a closure, for *every* form, nested
ones included. It produced the 50,000 `some?` calls and most of the 88,000 anonymous ones.

**Where does this name live?** Every variable reference walked every enclosing block, and each
block kept its names as a list of `(name slot)` pairs. Worse, the head of a call was resolved
three times, once for each question: is it a macro? A builtin taking raw arguments? One taking a
table name first?

## The fixes

Both are the textbook fix, a hash lookup instead of a list walk, in EELisp's own terms:

```lisp
;; special forms: a dict, so asking is one lookup
(def compiler-specials
  (reduce (fn (d s) (dict-set d s true)) {} '(quote quasiquote if do …)))
(dict-has compiler-specials h)

;; each block's names: a dict of name → slot
(defn compiler-slot-in (name block)
  (dict-get ((dict-get block :names)) name))
```

And `compiler-form` now resolves a call's head once and branches on what it learned:

```lisp
(let (kind (if (compiler-local? h cx) nil (vm-global-kind h)))
  (cond
    (= kind "macro") …
    (= kind "raw") …
    (= kind "table-first") …
    true (compiler-call x cx tailp)))
```

Nothing about *what* the compiler produces changed. The image it makes of itself differs only
where the compiler's own source changed, and it is still a fixed point. All 528 differential
snippets and the whole test suite in compiled mode still pass.

## The result

| | before | after |
|---|---|---|
| calls per form compiled | 6,779 | 2,569 |
| load eeditor's snippet files, compiled | 54 ms | 26 ms |
| evaluate `(+ 1 2)`, compiled | 12 µs | 5.8 µs |
| evaluate a small `defn`, compiled | 30 µs | 18 µs |
| the bootstrap's compiled stage | 23 ms | 14.6 ms |

The last row is the one particular to self-hosting. The compiler is also its own heaviest
user, so speeding it up speeds up building it.

## What's left

At 2,569 calls per form, nothing dominates any more. The rest is the compiler's context being
dictionaries holding closures (every field read is a `dict-get`, every mutable field a call), and
instruction lists built with `list` and `append`. Further gains would come from restructuring
that context, which is a larger change.

The profile also points somewhere other than the compiler. Every one of those calls pays the VM's
full call cost: a frame, its slots, copying arguments. A cheaper call path, or a few builtins the
compiler leans on (a fast `concat`, a symbol set), would speed up every EELisp program, not just
this one.
