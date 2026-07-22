# eelisp (Rust)

A ground-up Rust rewrite of the **EELisp** engine — the portable core of EEditor
(language + SQLite + Lotus-Agenda PIM). This crate is the reusable asset: it compiles to a
native library, a `eelisp` CLI, and (planned) WebAssembly, so one engine can back both a
desktop app (Tauri) and a browser app (PWA).

> Companion analysis: `../eeditor/docs/ANALYSIS.md` — full feature/mechanism inventory of the
> Swift original and the rationale for this rewrite.

## Status

**Language core: working.** Lexer, reader, environment, a tail-call-optimizing evaluator,
macros, quasiquote, a representative builtin set, and the standard prelude.

**Database layer: working (roadmap step 2 done).** dBASE-style `deftable / insert / query
(:where :params :order :asc :desc :limit :select) / update / delete (soft) / count-records /
pack / tables / describe / drop-table / field-get / field-set / record-id / records`, on
`rusqlite` (bundled SQLite). Schema is stored as JSON, so field defaults & choices survive a
reload (an ANALYSIS §6 fix). Result-sets print as ASCII grids.

**Agenda PIM: complete (roadmap step 3).** Items (`add-item`, `item-count/get/set/done`, `items`
with `:search/:category/:priority/:when-before/:when-after`, `items-on/between`, `add-item-today`);
`every` + recurrence date math (no external date crate); templates; multi-agenda
`open/use/close/agendas/export/import` with isolated per-file databases; **categories** (hierarchical
slash-paths + exclusive-parent enforcement); **rules** — the reflective engine (`defrule`,
`apply-rules`, `auto-categorize`, `rules`, `drop-rule`): conditions/actions are stored as Lisp,
re-parsed, and evaluated against each item with `text/notes/id/categories/props/get/has-category/
match` bound; **views** (`defview/show/views/drop-view`, filtered + grouped, `overdue?`); **smart NLP**
`add`/`smart-parse` (regex dates/priorities/people). _Transactions for batch ops still TODO._

**Host boundary: done (roadmap step 4).** Interactive views (`browse`→`.tableView`,
`edit`/`defform`→`.formView`, with computed fields); a JSON serialization bridge — `eval_host(src)`
returns a `{ok,result,output}` envelope and `eval_json(src)` returns the tagged JSON a TypeScript
client discriminates (`$tableView`, `$formView`, `$record`, `$dict`, …); `print`/`println` output
capture; the editor RPC (`buffer-text`, `current-file`, `cursor-pos`, `insert-at`, `replace-range`,
`selection`, `set-cursor`) backed by host-installed callbacks; and `json-parse`/`json-stringify`.
_HTTP builtins (the one network dependency) are the small remaining piece._

**Engine complete + native binding (roadmap step 5).** HTTP builtins (`http-get`/`http-post`, sync,
`ureq`); batch agenda ops (`apply-rules`, `import-agenda`) run in **transactions**. `EngineHandle`
(`src/server.rs`) owns the interpreter on a thread and exposes a `Send + Sync` `eval(src) -> String`
for a Tauri app to hold in state; `eelisp --serve` gives the same as a JSON-line stdin/stdout RPC.

**Test suite: GREEN — 72 tests, 0 ignored.** `tests/lang.rs` (25) + `tests/db_agenda.rs` (27) +
`tests/agenda_advanced.rs` (9) + `tests/host.rs` (7) + `tests/server.rs` (4).

**Remaining**: the WASM binding (`wasm-bindgen`) — deferred; it needs a non-C SQLite backend
(wa-sqlite/OPFS) behind the `database` module's method surface. The editor RPC over the *threaded*
`EngineHandle` also needs a UI return-channel (single-thread `Interpreter` hosts install callbacks
directly today).

## Bugs fixed vs. the Swift original

This is a **clean break**, not a bug-for-bug port. Deliberately corrected (see ANALYSIS §6):

- **Tail-call optimization** — the eval loop is a trampoline; tail recursion runs in O(1) stack
  (`tests/lang.rs::tail_call_optimization` runs 500 000 tail calls).
- **Quasiquote / unquote / unquote-splicing** are implemented (Swift parsed but never evaluated them).
- **Macro `. rest` params** work, so `when`/`unless`/`pipe`-style macros are correct.
- **Clean tokenization** — no operator-number splitting, no positional `-` heuristic. Write
  `(- 1 3)` to subtract, `-5` is a negative literal.

## Build & run

```bash
cargo test                                   # acceptance suite
cargo run                                    # REPL
cargo run -- -e '(map (fn (x) (* x x)) (range 1 6))'
cargo run -- script.el                       # run a file
```

## Layout

```
src/
  value.rs        Value enum, OrderedDict, LispError
  lexer.rs        tokenizer
  parser.rs       reader (quote/quasi/unquote, [..]=list, {..}=dict)
  env.rs          lexical scope chain
  eval.rs         TCO evaluator: special forms, macros, quasiquote, application
  builtins.rs     arithmetic / comparison / string / list / dict / type / io / meta
  printer.rs      Value -> string
  prelude.rs      standard library (EELisp source)
  interpreter.rs  high-level API (new / eval_str / eval_all)
  bin/eelisp.rs   REPL + file/-e runner
tests/lang.rs     acceptance spec
```

## Roadmap (from ANALYSIS §8)

1. ✅ Language core (lexer → parser → env → eval(+TCO) → builtins → prelude → printer)
2. ✅ Database + builtins (`rusqlite`, soft-delete/pack, JSON schema store)
3. ✅ Agenda layer — items/recurrence/templates/multi-agenda; categories/rules/views; SmartParser
   (`add`/`smart-parse`). _(TODO: wrap batch ops apply-rules/import in transactions.)_
4. ✅ Host boundary — structured view values (`.tableView`/`.formView`), serde JSON bridge
   (`eval_host`/`eval_json`), output capture, editor RPC, json-parse/stringify
5. ✅ Engine closed (HTTP builtins + transactional batch ops) + native binding (`EngineHandle`,
   `--serve`). Tauri command scaffolded in `../eeditor-next/src-tauri`. ⬜ WASM (`wasm-bindgen`) +
   the TS/CodeMirror UI are next.
