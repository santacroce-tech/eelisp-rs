# 8. A self-documenting language

EELisp carries its own manual. Two functions answer most "what's this called?" questions from
inside a REPL.

## `functions`: what's in scope

```lisp
(functions)            ; every special form, builtin, macro and function, grouped
(functions "str")      ; only names containing "str"
(functions :date)      ; a keyword or symbol works too
```

```text
special forms
  and               (and a b …)
  begin             (begin e…)
  …
builtins
  str                (str a b …) → string
  str-contains       (str-contains s needle) → bool
  …
macros
  unless  (unless test . body)
  when    (when test . body)

functions
  compose  (compose f g)
  …
— 213 functions
```

Your own definitions appear last, with their real parameter lists.

## `source`: one definition

For anything written in EELisp, `source` echoes the definition back **as it was written**. That
includes the comment block above it, its indentation and the comments inside it:

```text
> (source when)
when — macro

  ;; Run the body only when test is true; otherwise nil.
  (defmacro when (test . body)
    `(if ,test (do ,@body) nil))
```

A builtin has no EELisp source, so `source` shows its manual entry instead. That's a signature,
one line of prose and a worked example:

```text
> (source map)
map — builtin

  (map f lst) → list
  Applies f to every element.

  (map (fn (n) (* n n)) '(1 2 3))   → (1 4 9)
```

`source` doesn't evaluate its argument, so `(source map)`, `(source 'map)` and `(source "map")`
all work. Special forms have entries too: `(source let)`.

## The same, as data

A program such as a form that browses the functions wants data, not printed text:

```lisp
(function-list "sheet-open")
; → ({:kind "builtin" :name "sheet-open" :sig "(sheet-open name) → dict" :summary "…"})

(source-text "map")    ; what (source map) prints, as a string
                       ; this one evaluates its argument, so the name can come from a variable
```

## Writing documentation

Because `source` shows the comment block above a definition, **the comment is the
documentation**. Nothing has to be repeated anywhere else:

```lisp
;; The first n elements.  (take 2 '(1 2 3)) → (1 2)
(defn take (n lst)
  (if (or (= n 0) (empty? lst))
    (list)
    (cons (head lst) (take (- n 1) (tail lst)))))
```

The rules for which comment belongs to which definition are short:

- The run of `;` lines **directly above** a top-level definition belongs to it.
- A **blank line**, or a line with code on it, ends the block. A comment only ever documents the
  definition immediately beneath it.
- A comment **after** a definition, on the same line, belongs to that definition too. That's how a
  file of one-line helpers documents itself.
- Redefining a name replaces its recorded source, which is what someone editing at a REPL expects.

```lisp
(defn inc2 (n) (+ n 2))    ; two more — this trailing comment documents inc2
```

The whole prelude is written this way. `(source take)` shows the comment above.

## Why it can't go stale

The engine's test suite checks that the builtin manual and the environment name **exactly the
same set** of functions. A builtin added without a manual entry fails the tests, and so does a
manual entry for a function that no longer exists. The [function reference](reference.md) in these
pages is generated from the same tables.

Continue with [Architecture](09-architecture.md).
