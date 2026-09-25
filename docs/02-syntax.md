# 2. Syntax and values

EELisp source is read in two steps. The **lexer** cuts the text into tokens. The **reader** turns
the tokens into values: lists, symbols, numbers and the rest. Code is data, so what the reader
produces is exactly what the evaluator runs and what a macro receives.

## Atoms

```lisp
42        -3.5      1e3        ; numbers — one type, a 64-bit float
"hello"   "a\tb\n"             ; strings, with \n \t \r \\ \" escapes
true      false                ; booleans
nil                            ; nothing
foo       str-len   a.b        ; symbols — names
:name     :where               ; keywords — names that stand for themselves
```

The lexer reads an atom whole and then decides what it is. If it starts with `:`, it's a
keyword. If it parses as a number, it's a number. Otherwise it's a symbol. There's no splitting
of operators from numbers, so:

| You write | It reads as |
|---|---|
| `-5` | the number −5 |
| `-` | the symbol `-` (subtraction) |
| `(- 1 3)` | a call: −2 |
| `1e3` | the number 1000 |
| `a.b`, `->string`, `even?`, `set!` | ordinary symbols |

Whole numbers print without a decimal point: `(* 2 21)` prints `42`, and `(/ 7 2)` prints `3.5`.

## Lists, brackets and braces

```lisp
(f a b)              ; a list — evaluated as a call to f
[1 2 (+ 1 2)]        ; square brackets are shorthand for (list …) → (1 2 3)
{:name "Ada" :born 1815}   ; a dict literal; values are evaluated, keys aren't
```

Square brackets read as a call to `list`. `[1 2]` is exactly `(list 1 2)`, so the elements are
evaluated. Quoting a bracket list shows this: `'[1 2]` is the three-element list `(list 1 2)`.

A dict literal keeps its keys in the order they were written. Keys are usually keywords.

## Comments

```lisp
; a comment runs to the end of the line
;; by convention, a ;; block directly above a definition documents it
(defn inc (n) (+ n 1))   ; a trailing comment belongs to the form on its line
```

Comments aren't just ignored. The reader remembers the comment block written directly above each
top-level definition, and `(source name)` prints it back. See
[A self-documenting language](08-self-documenting.md).

## Quoting

Evaluation can be switched off, in whole or in part. The four prefix characters are shorthand
for special forms:

| Shorthand | Long form | Meaning |
|---|---|---|
| `'x` | `(quote x)` | the value `x` itself, unevaluated |
| `` `x `` | `(quasiquote x)` | a template |
| `,x` | `(unquote x)` | inside a template: evaluate `x` and put the value here |
| `,@x` | `(unquote-splicing x)` | inside a template: evaluate `x` and splice the list in |

```lisp
'(1 2 3)                        ; → (1 2 3), not a call to "1"
(def xs '(2 3))
`(1 ,(+ 1 1) ,@xs 4)            ; → (1 2 2 3 4)
```

Templates are what [macros](03-language.md#macros) are built from.

## Truth

Only two values are false: `false` and `nil`. Everything else is true, including `0`, the empty
string and the empty list:

```lisp
(if 0 "yes" "no")        ; → "yes"
(if "" "yes" "no")       ; → "yes"
(if (list) "yes" "no")   ; → "yes"
(if nil "yes" "no")      ; → "no"
```

Test emptiness explicitly with `empty?`, `zero?` or `(= s "")`.

## Types

Every value has a type, and `(type x)` names it:

| Type | Example | Notes |
|---|---|---|
| `number` | `42`, `-3.5` | a 64-bit float |
| `string` | `"hi"` | Unicode; lengths count characters |
| `bool` | `true` | |
| `nil` | `nil` | |
| `symbol` | `'foo` | a name |
| `keyword` | `:foo` | a name that evaluates to itself |
| `list` | `(1 2 3)` | immutable, cheap to share |
| `dict` | `{:a 1}` | insertion-ordered, immutable |
| `function` | `(fn (x) x)`, `inc` | written in EELisp |
| `builtin` | `map`, `+` | written in Rust |
| `macro` | `when` | |
| `table`, `record`, `result-set` | `(query books)` | [database](05-database.md) values |
| `item` | `(item-get 1)` | an [agenda](06-agenda.md) item |
| `table-view`, `form-view` | `(browse books)` | interactive views for a host to draw |

Values compare structurally. `(= '(1 2) '(1 2))` is true, and two dicts with the same keys in the
same order and the same values are equal. Functions never compare equal, not even to themselves.

Continue with [The language](03-language.md).
