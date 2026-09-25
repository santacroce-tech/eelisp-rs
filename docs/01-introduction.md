# 1. Introduction

EELisp is the language inside [EEditor](https://github.com/santacroce-tech). It's a Lisp in the
usual sense, with prefix calls, code as data and macros. It also has three things most Lisps
don't:

- **A database.** `deftable`, `insert` and `query` work on SQLite tables, dBASE-style. A query
  result is a value, so it prints as a grid in a terminal, and a host application can draw it as
  an interactive table.
- **An agenda.** It's a personal organiser in the style of Lotus Agenda: free-text items,
  hierarchical categories, and *rules* written in EELisp that file items as they arrive.
- **Sheets.** These are spreadsheets whose formulas are ordinary EELisp expressions. In
  `=(sum B1:B9)`, `B1:B9` is just a list.

The engine is a Rust library. It's single-threaded, it has no I/O of its own beyond SQLite
files, and no platform types leak into its values. That's what lets the same code run behind a
desktop app, a command line, and a browser tab.

## A first taste

```lisp
(def pi 3.14159)
(defn area (r) (* pi r r))
(area 2)                                  ; → 12.56636

(map (fn (x) (* x x)) (range 1 6))        ; → (1 4 9 16 25)
(filter even? (range 1 10))               ; → (2 4 6 8)

(def ada {:name "Ada" :born 1815})
(dict-get ada :name)                      ; → "Ada"
```

## Running it

Build the command-line binary with Cargo:

```bash
cargo build --release          # target/release/eelisp
cargo test                     # the acceptance suite
```

The binary has four modes:

```bash
eelisp                         # an interactive REPL
eelisp script.eelisp           # run a file, top to bottom
eelisp -e '(+ 1 2)'            # evaluate one expression and print it
eelisp --serve                 # JSON-line RPC on stdin/stdout, for a host program
eelisp --serve --db notes.db   #   … keeping tables and agenda in a file
```

In the REPL, an expression that spans several lines keeps reading until its brackets balance:

```text
EELisp v0.2 (Rust) — :quit to exit
λ> (defn sq (x)
..   (* x x))
#<fn sq>
λ> (sq 9)
81
```

Files conventionally end in `.eelisp`. A file is read as a sequence of top-level forms, each
evaluated in turn.

## Design in one page

A few decisions shape everything else in these pages:

| Decision | Consequence |
|---|---|
| **One numeric type** (64-bit float) | `(/ 7 2)` is `3.5`; integers print without a decimal point |
| **Only `false` and `nil` are false** | `0`, `""` and `()` are all true |
| **Values are immutable** | `dict-set` returns a new dict; lists are never changed in place |
| **Tail calls don't grow the stack** | Recursion is the loop; 100 000 nested tail calls are fine |
| **Keywords name things** | `:name` is a dict key, an argument name (`:where`), and never needs quoting |
| **Clean tokenizing** | `-5` is a number and `-` is a symbol; write `(- 1 3)` to subtract |
| **Docs live in comments** | A `;;` block above a `defn` *is* its documentation, shown by `(source f)` |
| **The host decides the reach** | Editor access and the working folder come from callbacks the host installs |

EELisp began as a Swift engine. This Rust version is a deliberate clean break that fixes that
engine's known faults: tail-call optimisation, a working quasiquote, macros with rest parameters,
and tokenizing without special cases.

Continue with [Syntax and values](02-syntax.md).
