# EELisp documentation

EELisp is a small Lisp for personal data. It's a language with a database, a Lotus-Agenda-style
organiser and a spreadsheet built in. It's written in Rust, and SQLite is compiled in. The same
engine runs as a REPL, as a library inside a desktop app, as a JSON-line server, and in a browser
as WebAssembly.

```lisp
(deftable books (title:string author:string year:number))
(insert books {:title "Thinking Forth" :author "Brodie" :year 1984})
(query books :where "year > ?" :params (list 1980) :order "year")

(add "call Bob tomorrow !!")          ; an agenda item, date and priority parsed out
(defrule calls :when (str-matches text "call|phone") :assign "work/calls")
(apply-rules)                         ; → 1

(sheet-set "Budget" "B3" "=(sum B1:B2)")   ; a spreadsheet formula is just EELisp
```

These pages cover the whole language, from reading your first expression to embedding the
engine in an application. Every example was run against the engine, and the results shown
after `→` are what it printed.

| # | Chapter | What's in it |
|---|---|---|
| 1 | [Introduction](01-introduction.md) | What EELisp is for, running it, the design in one page |
| 2 | [Syntax and values](02-syntax.md) | How source text is read: atoms, lists, dicts, quoting, truth |
| 3 | [The language](03-language.md) | Definitions, control flow, functions, tail calls, macros, errors |
| 4 | [Working with data](04-data.md) | Lists, dicts, strings, numbers, dates, JSON, HTTP, output |
| 5 | [The database](05-database.md) | dBASE-style tables on SQLite: define, insert, query, forms |
| 6 | [The agenda](06-agenda.md) | Items, natural-language capture, categories, rules, views |
| 7 | [Sheets](07-sheets.md) | Spreadsheets whose formulas are EELisp |
| 8 | [A self-documenting language](08-self-documenting.md) | `functions`, `source`, and writing docs as comments |
| 9 | [Architecture](09-architecture.md) | How the engine is built, with sequence diagrams |
| 10 | [Embedding](10-embedding.md) | Using the engine from Rust, over JSON, and in a browser |
| — | [Function reference](reference.md) | Every special form, builtin, macro and prelude function |

## One file to read offline

[`eelisp.html`](eelisp.html) is all of the above in one self-contained HTML page. It has no
external requests, the diagrams are drawn inline, and it follows your system's light or dark
mode. Rebuild it after editing any chapter:

```bash
python3 scripts/build-docs.py
```

The same build writes `site/docs.html`, the copy published on
[eelisp.app](https://eelisp.app/docs.html) with the site's navigation. The script also regenerates
[`reference.md`](reference.md) from the engine's built-in manual in
`src/docs.rs` and from the prelude in `src/prelude.rs`. The reference can't drift from the engine,
because it's made from the same tables that `(functions)` and `(source …)` read.
