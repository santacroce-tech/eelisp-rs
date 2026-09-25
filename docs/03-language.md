# 3. The language

A list is evaluated as a call. The first element says what to do, and the rest are the
arguments. `(+ 1 2)` calls `+` with `1` and `2`. A symbol evaluates to whatever it's bound to.
Everything else (numbers, strings, keywords, `true`, `false`, `nil`) evaluates to itself.

There are three kinds of thing that can appear at the head of a list:

- **Special forms**, such as `if`, `def` and `let`. The evaluator handles these itself, and each
  one decides which of its arguments to evaluate.
- **Macros**, such as `when` and `unless`. These receive their arguments *unevaluated* and return
  code, which is then evaluated in their place.
- **Functions**, either builtins written in Rust or functions written in EELisp. Their arguments
  are evaluated first, left to right.

## Definitions

```lisp
(def pi 3.14159)                  ; a global binding
(defn area (r) (* pi r r))        ; a function
(defun area (r) (* pi r r))       ; the Common Lisp spelling, same form
(def (square x) (* x x))          ; def with a call shape is also a function

(fn (x) (* x x))                  ; an anonymous function
(lambda (x) (* x x))              ; the same
((fn (x) (* x x)) 5)              ; → 25
```

`set!` reassigns a binding that already exists. Using it on a name that was never defined is an
error:

```lisp
(def n 1)
(set! n (+ n 1))                  ; → 2
(set! nope 1)                     ; Error: Undefined symbol: nope
```

### Local bindings

`let` takes its bindings as **one flat list** of name–value pairs. Each binding can see the ones
before it:

```lisp
(let (a 2 b (* a 3)) (+ a b))     ; → 8
```

The Scheme shape, with nested pairs, is accepted too: `(let ((a 1) (b 2)) (+ a b))`.

## Control flow

```lisp
(if (> x 10) "big" "small")       ; else is optional; a missing else is nil

(cond (< n 0) "negative"          ; cond is flat: test, result, test, result …
      (= n 0) "zero"
      true    "positive")         ; true, else or otherwise catch everything

(and a b c)                       ; the first false value, or the last value
(or a b c)                        ; the first true value, or the last value
(not x)

(do (println "first") 42)         ; evaluate in order, return the last
(begin (println "first") 42)      ; the Scheme spelling of do

(when ready (println "go") 1)     ; macros from the prelude: a body of several forms
(unless ready (println "wait"))

(for-each x (list 1 2 3)          ; a loop for effects; returns nil
  (println (* x 10)))
```

`cond` pairs aren't wrapped in parentheses. That's the one place EELisp differs visibly from
Scheme and Common Lisp.

## Functions

Functions close over the scope they were created in:

```lisp
(defn make-counter ()
  (let (n 0)
    (fn () (set! n (+ n 1)) n)))

(def c (make-counter))
(c) (c) (c)                       ; → 3
```

A parameter list can end with `. rest`. The rest parameter collects the remaining arguments into
a list:

```lisp
(defn total (first . others)
  (+ first (reduce (fn (a b) (+ a b)) 0 others)))

(total 1 2 3 4)                   ; → 10
```

A missing argument is bound to `nil` rather than raising an error:
`((fn (a b) (list a b)) 1)` gives `(1 nil)`.

Functions are values. You can pass them, return them and store them:

```lisp
(map (fn (x) (* x x)) (range 1 6))    ; → (1 4 9 16 25)
(apply + (list 1 2 3))                ; → 6
((compose inc inc) 5)                 ; → 7
```

## Tail calls and loops

The evaluator is a *trampoline*. When the last thing a function does is call another function,
the evaluator reuses the current frame instead of stacking a new one. The same goes for the taken
branch of an `if`, the last form of a `do` or `let`, the chosen branch of a `cond`, and the last
operand of `and` and `or`. So recursion in tail position runs in constant stack space, including
mutual recursion:

```lisp
(defn even2? (n) (if (= n 0) true  (odd2?  (- n 1))))
(defn odd2?  (n) (if (= n 0) false (even2? (- n 1))))
(even2? 100001)                    ; → false, without exhausting the stack
```

For an explicit loop, use `loop` with `recur`. `loop` introduces bindings as `let` does, and
`recur` jumps back to the top with new values:

```lisp
(defn fact (n)
  (loop (i n acc 1)
    (if (<= i 1)
        acc
        (recur (- i 1) (* acc i)))))

(fact 10)                          ; → 3628800
```

`recur` outside a `loop` is an error: `recur used outside of a loop`.

## Macros

A macro is a function that runs *before* evaluation. It receives its arguments as unevaluated
code and returns new code, which is evaluated in its place. Quasiquote makes the returned code
easy to write:

```lisp
(defmacro my-unless (c body)
  `(if ,c nil ,body))

(my-unless false "ran")            ; → "ran"
```

Macros take rest parameters, which is how a macro accepts a body of several forms. This is
`when`, exactly as the prelude defines it:

```lisp
(defmacro when (test . body)
  `(if ,test (do ,@body) nil))
```

Macros aren't hygienic. A name the template introduces can capture a name at the call site, so
pick unusual names for temporaries:

```lisp
(defmacro swap! (a b)
  `(let (tmp ,a) (set! ,a ,b) (set! ,b tmp)))

(def p 1) (def q 2)
(swap! p q)
(list p q)                         ; → (2 1)
```

## Code as data

The reader and evaluator are available from inside the language:

```lisp
(parse "(+ 1 2)")                  ; → the list (+ 1 2), not evaluated
(eval (parse "(+ 1 2)"))           ; → 3
(eval (list '* 6 7))               ; → 42
```

The agenda's [rules](06-agenda.md#rules) depend on this. A rule's condition is stored as text in
the database, then read back and evaluated against each item.

## Errors

An error stops evaluation and reports one line:

| Error | Example cause |
|---|---|
| `Undefined symbol: x` | using a name that isn't bound |
| `Type mismatch: expected list, got number` | `(car 5)` |
| `Arity mismatch: f expects …, got …` | a builtin called with the wrong number of arguments |
| `Division by zero` | `(/ 1 0)` |
| `Invalid syntax: …` | a malformed special form, such as `(defn)` |
| `Parse error: unclosed list` | unbalanced brackets |
| `Database error: …` | SQLite refused, for example a bad `:where` clause |
| `Error: …` | a runtime failure reported by a builtin |

There's no `try`/`catch` in the language. Errors are for the host to handle. The REPL prints them
and carries on, and the host API returns them as `{"ok": false, "error": "…"}` (see
[Embedding](10-embedding.md)). Some builtins return `nil` for a missing value instead of failing.
For example, `(nth lst 99)` is `nil`.

Continue with [Working with data](04-data.md).
