# 3. A compiler in EELisp

*Code: `src/compiler.eel` (about 300 lines). Tests: `tests/compiler.rs`.*

## The shape of it

The compiler is an EELisp program with two entry points:

```lisp
(compile form)       ; → a (proto …) form: the bytecode for one top-level form
(compile-run form)   ; → compiles the form and runs it at the top level
```

Inside, one function does most of the work. `compiler-expr` takes a form, the names bound in
enclosing scopes, and whether the form is in **tail position**, and returns a list of
instructions that leaves the form's value on the stack:

```lisp
(defn compiler-expr (x locals tailp)
  (cond
    (symbol? x) (list (list 'load x))
    (= (type x) "dict") (compiler-dict x locals)
    (and (list? x) (not (empty? x))) (compiler-form x locals tailp)
    true (compiler-const x)))
```

A symbol becomes a `load`. A non-empty list is a special form, a macro call or a function call.
Anything else (a number, a string, a keyword, the empty list) is a constant.

## Compiling a function

Here is what the compiler makes of a factorial:

```lisp
(compile '(defn fact (n) (if (<= n 1) 1 (* n (fact (- n 1))))))
```

The function body comes out as:

```lisp
(load <=) (load n) (const 1) (call 2) (jump-if-false 1)
(const 1) (jump 2)
(label 1)
(load *) (load n) (load fact) (load -) (load n) (const 1) (call 2) (call 1) (tail-call 2)
(label 2)
```

Read it as the `if`: compute `(<= n 1)`; if false, jump to label 1. Otherwise push `1` and jump
to the end. At label 1, build `(* n (fact (- n 1)))`. The inner calls are ordinary `call`s: their
results are needed. The outer `*` is the last thing the function does, so it is a `tail-call`.
The tail-position flag is what decides this: `compiler-if` passes `tailp` down to both branches,
and every argument is compiled with `tailp` false.

## Every special form, the interpreter's way

The rule for each special form was: read the interpreter's code in `src/eval.rs`, and produce
instructions that do the same thing in the same order, quirks included. Some examples:

- **`let`** binds sequentially, like Scheme's `let*`, and accepts both `(let (a 1 b 2) …)` and
  `(let ((a 1) (b 2)) …)`. The compiler emits `push-scope`, each value followed by `def`, the
  body, then `pop-scope`.
- **`cond`** is EELisp's flat form, `(cond test result test result …)`, where `else` and
  `otherwise` always match. Each test becomes a `jump-if-false` to the next pair.
- **`loop`** has exactly one body form, and **`recur`** compiles to its arguments then
  `(recur n)`:

  ```lisp
  (compile '(loop (n 5 acc 0) (if (= n 0) acc (recur (- n 1) (+ acc n)))))
  ;  (push-scope) (const 5) (def n) (pop) (const 0) (def acc) (pop) (loop n acc)
  ;  … (load -) (load n) (const 1) (call 2) (load +) (load acc) (load n) (call 2) (recur 2) …
  ;  (end-loop) (pop-scope)
  ```

- **`try`**: its body is never compiled as a tail call, because an error in it has to land in
  *this* frame's handler.
- **quasiquote** becomes a `make-list` that knows which elements to splice:

  ```lisp
  (compile '`(a ,x ,@xs))
  ;  (const a) (load x) (load xs) (make-list item item splice)
  ```

- **`for-each`** steps through its list with `next-item`, binding the variable in a fresh scope
  for each element, just as the interpreter does.

## Macros, at compile time

When the head of a form names a global macro, the compiler expands it *now* and compiles the
expansion. The VM has a helper for each half:

- `vm-global-kind` says what a name means to the compiler: a macro, a builtin with unevaluated
  arguments, or neither.
- `vm-macroexpand` expands a macro call once.

So `(when ok (go))` compiles exactly as `(if ok (do (go)) nil)` would:

```lisp
(load ok) (jump-if-false 7) (load go) (tail-call 0) (jump 8) (label 7) (const nil) (label 8)
```

`defmacro` itself compiles too. The expander becomes a proto and `(macro …)` makes the macro, so
expanding a compiled macro runs bytecode.

The same helper handles builtins that don't evaluate all their arguments. `query` takes its
table name as written, so its first argument becomes a constant while the rest are evaluated:

```lisp
(compile '(query books :where "year > ?" :params (list 1980)))
;  (load query) (const books) (const :where) (const "year > ?") (const :params)
;  (load list) (const 1980) (call 1) (tail-call 5)
```

## The escape hatch

There is one more instruction, `(interp form)`, which asks the interpreter to evaluate a form in
the current scope. Early on, anything the compiler didn't understand yet became an `interp`, so
*every* program ran from the first day and the compiler could grow one form at a time. The
compiler counts these places in `compiler-fallbacks`.

The count went from 112 (database and agenda forms), to 3, to what it is now: only **malformed**
forms like `(def x)` or `(defmacro)` fall back, on purpose, so that their error message is the
interpreter's own.

## Keeping it honest: the differential test

A compiler that is "mostly right" is worse than none. So `tests/compiler.rs` doesn't test the
compiler with examples written for it. It uses the examples written for the *interpreter*.

It reads seven of the engine's own test files (`lang.rs`, `book_gaps.rs`, `bytes.rs`, `csv.rs`,
`host.rs`, `agenda_advanced.rs`, `db_agenda.rs`), pulls out every EELisp snippet they evaluate
(434 of them), and adds 37 edge cases for quasiquote, `for-each` and `defmacro`. For each test
function it creates two fresh engines, each in its own scratch folder. One interprets the
snippets; the other compiles each top-level form and runs the bytecode. Every result **and
every error** must match.

It runs that comparison four times:

1. with the interpreter running the compiler;
2. with the prelude compiled as well;
3. with the compiler having compiled itself;
4. with the compiler loaded from the bootstrap image (next chapter).

All 471 snippets agree in all four. And the test can fail: when we deliberately broke `if`, or
quasiquote's splicing, it reported the disagreements at once.

## The differences we know about

Three behaviors differ, and they're written down in the roadmap rather than hidden:

1. **Calling something that isn't a function.** `(1 (error "x"))`: the interpreter complains
   that `1` isn't a function before evaluating the arguments; compiled code evaluates the
   arguments first, so the `error` wins.
2. **A macro defined after the function that uses it.** The interpreter expands macros when the
   code runs; the compiler needed the macro when it compiled the function.
3. **Expanded once.** A macro is expanded when its caller is compiled, not every time the caller
   runs. That only matters for a macro with side effects.

Next: the compiler compiles itself.
