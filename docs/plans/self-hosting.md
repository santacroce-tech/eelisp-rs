# Self-hosting EELisp: a small kernel, the language above it, pluggable storage

*Status: first analysis / proposal — 2026-11-04. Not a commitment; a map to argue with.*

## Goal

Make EELisp **the language, not the Rust program**. Today the language is defined by ~10.6k lines
of Rust plus SQLite. The aim:

1. Most of EELisp is written in EELisp, on top of a **small, replaceable kernel**.
2. The kernel is small and tested enough to port (C, JS, Swift, Zig, Chez…) in days, not months.
3. EELisp keeps speaking **SQL**, but through a **driver interface**. SQLite becomes one backend,
   next to **eedb** (a native, portable SQL engine, its own project; see [eedb.md](eedb.md)) and
   remote ones (PostgreSQL, …).

"No Rust" is a side effect, not the goal. The goal is that no single host defines the language.

## Where we are

| Layer | Files | ~Lines | External deps |
|---|---|---|---|
| Language core | lexer, parser, eval, env, value, printer | 1.7k | — |
| Builtins | builtins, bytes, dates, host, editor | 2.1k | regex, serde_json |
| Domains | database, db_builtins, agenda, agenda_builtins, smart_parser, sheet, sheet_ref, sheet_builtins | 5.3k | **rusqlite (bundled SQLite C)**, ureq |
| Docs / self-doc | docs, prelude | 0.95k | — |
| Hosts | interpreter, server, bin, web | ~0.7k | wasm-bindgen |

The prelude (`src/prelude.rs`) is already EELisp — 84 lines. Everything else is Rust.

Things that tie the language to SQLite today, beyond the obvious:

- **`:where` is raw SQL.** `(query books :where "year > ?" :params (list 1980))` pastes the
  string into `SELECT … WHERE _deleted = 0 AND (…)`. User code is SQLite code.
- **The agenda uses SQLite functions**: `LIKE` on serialized categories/properties and
  `json_extract(properties,'$.when')` for dates.
- **Files are SQLite files.** A `.db` is a SQLite database; a `.eesheet` is a SQLite file
  recognised by `PRAGMA application_id` ("EESH") and versioned with `PRAGMA user_version`.
  `Database::to_bytes` / `from_bytes` ship SQLite's serialized image across the host boundary.
- **Transactions, `INSERT OR REPLACE`, `PRAGMA data_version`** (change detection).

SQL itself is fine. The coupling is that the engine calls SQLite directly and relies on
SQLite-specific pieces. Those must go behind a driver interface before a second backend can exist.

## Principles

- **The core does no I/O** (already true — keep it). Every storage backend reaches the outside
  world through host-installed callbacks, the same way the editor bridge does. The host decides
  the reach.
- **One test suite, many implementations.** Every phase is "done" when the existing tests pass
  against the new thing. Differential testing (same input → same printed output on two engines)
  is the main tool.
- **Subset, not clone.** The EELisp-native store implements what EELisp uses, not SQL.
- **Keep every current target green**: CLI, `--serve`, `EngineHandle`, `eelisp-web`, and the
  `language-only` wasm32 sandbox build.

---

## Phase 0 — EELisp in EELisp (proof and executable spec)

Write, in EELisp, run by the current Rust engine:

- `lib/reader.eel` — the reader (atoms, lists, `[]`, `{}`, quote prefixes, `;;` doc blocks).
- `lib/eval.eel` — a metacircular evaluator for the special forms (`if def let cond do fn set!
  defn defmacro quasiquote loop/recur try/catch`), delegating primitives to the host engine.

And a **differential harness**: take the cases in `tests/lang.rs` (and later the others), run each
through the native engine and through `eval.eel`, compare printed results and error kinds.

**Done when:** the language test cases pass through `eval.eel`.
**What it buys:** a list of every semantic that today exists only in Rust (argument modes, TCO,
error shapes, dict ordering, number formatting). That list *is* the kernel spec.
**Expected cost:** the metacircular eval will be slow (a tree-walker running a tree-walker —
plausibly 10–50× slower, unmeasured). That's fine; it's a spec and a test oracle, not the runtime.

## Phase 1 — Kernel / library split

Freeze a **kernel spec** and move everything else into EELisp libraries loaded at startup
(like the prelude, possibly precompiled/cached later).

Kernel (stays native, ~2–3k lines in any host):

- Value representation, environments, the evaluator and special forms, macros, TCO.
- Primitive arithmetic, comparison, strings (incl. one regex primitive), lists, dicts, bytes.
- Reader and printer (the EELisp reader in `reader.eel` becomes the reference; the native one is
  the fast path).
- Error values, `try`/`catch`.
- The host boundary: `eval_host` envelope, `define_fn` / `call`, callback installation.
- **The SQL driver interface** (Phase 2), nothing above it.

Candidates to move up into EELisp:

| Today (Rust) | Moves to | Notes |
|---|---|---|
| Much of `builtins.rs` (list/HOF/string helpers) | `lib/core.eel` | keep hot paths native if profiling says so |
| `dates.rs` | `lib/dates.eel` | pure arithmetic; needs only "now" from the host |
| CSV | `lib/csv.eel` | pure string work |
| JSON parse/stringify | `lib/json.eel` | keep a native fast path; the EELisp one is the spec |
| `smart_parser.rs` | `lib/smart.eel` | string heuristics + regex primitive |
| `agenda.rs` (rules, categories, recurrence, templates) | `lib/agenda.eel` | logic over the SQL driver |
| `sheet.rs` recalculation (dependency order, cycles) | `lib/sheet.eel` | Tarjan, formula eval — already EELisp formulas |
| `docs.rs` manual / source index | `lib/docs.eel` | manual/environment parity test still applies |

**Done when:** the full test suite passes with the Rust kernel + EELisp libraries, and the
`language-only` wasm32 build is the kernel alone.

## Phase 2 — SQL through a driver interface

SQL stays. EELisp speaks SQL as a first-class language for data; what changes is that the
engine stops calling `rusqlite` directly and talks to a **SQL driver interface** instead.

### 2a. Keep SQL, add structure where it helps

- `(query books :where "year > ?" :params (list 1980))` stays as it is.
- Add `(sql "SELECT … WHERE year > ?" 1980)`, a direct escape to the driver for joins and
  aggregates.
- Optionally, add an s-expression SQL library that compiles to SQL text, for macros and generated
  queries. It is sugar, not a replacement:

  ```lisp
  (select [title year] :from books :where (> year ?) :order-by year)
  ```

- The agenda's `LIKE` and `json_extract` stay SQL. They become part of the SQL subset every
  driver must support (see [eedb.md](eedb.md#the-sql-subset-eelisp-needs-today)).

### 2b. The driver interface

It works at the SQL level, like JDBC or DBI:

```
open / close · prepare · bind · step · exec
begin / commit / rollback
snapshot() → bytes · restore(bytes)
changes()            ; replaces PRAGMA data_version; ideally per-table/row
```

Each driver carries a small **dialect** layer (parameter syntax, JSON functions, upsert spelling,
file identity and versioning in place of `PRAGMA application_id` / `user_version`).

**Done when** the whole suite runs with SQLite behind the driver interface, and nothing above it
calls `rusqlite`.

## Phase 3 — eedb

A separate project: an embedded SQL engine that is native, fast and portable, including WASM
with no C toolchain, with pluggable storage devices. EELisp is its first user. Full plan:
**[eedb.md](eedb.md)**.

Short version: v0 implements exactly the SQL subset EELisp uses today (surveyed from the source).
It is tested with sqllogictest files and differentially against SQLite. Storage is a
log-structured, in-memory store on a host device first, with a paged B-tree later if data
outgrows memory. It is ready for EELisp when the database, agenda and sheet suites pass on it.

The point is a database in the wasm32 sandbox and in `eelisp-web`, which have none today.

## Phase 4 — Other backends ("am I hallucinating?" — no)

Once the driver interface exists, a backend is either a SQL driver (SQLite, eedb, PostgreSQL) or,
for eedb, a **storage device** under it (file, memory, OPFS, S3 segments).

| Backend | Fit | How |
|---|---|---|
| **SQLite** | excellent | today's code; stays the default for desktop and compatibility |
| **PostgreSQL** | good | a SQL driver with a dialect layer (`$1` params, `jsonb` for properties); transactions map 1:1; host provides the connection (a network client is host-side, the core still does no I/O) |
| **S3 / object storage** | good *as an eedb device* | objects are immutable, so the append-only design maps well: log **segments** as objects, a small manifest object pointing at the current set, `pack` writes a new segment set + manifest. Not good for row-level in-place updates — don't try. Single writer, or a lock/lease object, unless you want real concurrency work |
| **Browser (OPFS / IndexedDB)** | good | an eedb device; finally a durable database in `eelisp-web` |
| **Plain files on any filesystem** (incl. FUSE-mounted, NFS, Dropbox…) | trivial | that's just the file device; FUSE is invisible to us here |
| **FUSE, the other direction** | interesting, separate | *exposing* an EELisp database or agenda **as** a filesystem (tables as dirs, records as files) is a host program using the driver interface, not a backend. Nice for grep/editors/backups; a later side project |

The honest constraints:

- **Semantics differ at the edges**: transaction isolation, what "durable" means on S3, ordering
  guarantees. The driver interface should state the minimum guarantees (single writer, atomic commit,
  read-your-writes) and backends must meet them or refuse.
- **Network backends are synchronous calls from the evaluator's point of view** (as `http-get`
  is today). Fine for scripts and an editor; a real server would want the host to batch.
- **Feature subsets**: a driver may not support all of SQL; it must reject unsupported
  statements with a clear error, never guess.

## Phase 5 — Other kernels and (optionally) a compiler

With Phases 1–3 done, the kernel is ~2–3k lines with a written spec and a differential test
suite. Porting it is a bounded job. Reasonable candidates:

- **JS/TS kernel** for the web (instead of, or alongside, Rust→WASM).
- **Swift kernel** for Apple hosts (where EELisp started).
- **C kernel** next to SQLite, for the smallest dependency footprint.
- **Chez Scheme** as a fast native host for the CLI/server (see below).

Optionally, later: a **compiler written in EELisp** (to WASM, C, or Scheme), bootstrapped by
running it on today's interpreter (stage 0), then compiling itself (stage 1 = stage 2 check).
Only worth it if performance becomes the bottleneck after Phase 1.

Taken all the way, a compiler that emits RISC-V machine code, plus a runtime written in a low-level
EELisp subset, is a path to EELisp on bare metal. See [riscv-os.md](riscv-os.md).

### On Chez Scheme specifically

Good as *one* kernel host, not as *the* foundation:

- **For:** native speed, mature GC, proper tail calls for free (the trampoline goes away),
  hygienic macros, a stable ecosystem.
- **Against:** the native backend generates machine code, which hurts the WASM, browser and iOS
  targets (as far as I know they'd need the portable-bytecode backend, which is slower — not
  verified); embedding (`eval_host`, `define_fn`, callbacks) would be rebuilt over its C FFI; and
  Scheme semantics (numeric tower, mutable pairs, conditions) fight EELisp's (one float type,
  immutable values, ordered dicts, errors as dicts). Most effort would go into making Chez
  behave like EELisp instead of like Scheme.

## Compatibility

- Existing `.db` and `.eesheet` files are SQLite files. Keep the SQLite backend indefinitely as
  the reader/writer for them, and add `export`/`import` between backends (`agenda` already has
  `export-agenda` / `import-agenda` as a model).
- Give eedb its own file magic and format version from day one, the way `.eesheet` uses
  `application_id` / `user_version`.

## Order of work and rough size

| Phase | Depends on | Size (rough guess) | Unlocks |
|---|---|---|---|
| 0. Metacircular eval + diff harness | — | days | kernel spec; confidence |
| 2a. `sql` escape + optional s-expr SQL | — | days | structured queries, still SQL |
| 1. Kernel / library split | 0 | weeks, incremental | portable kernel |
| 2b. SQL driver interface (SQLite behind it) | — | ~week | any SQL backend |
| 3. eedb (see eedb.md milestones) | 2b | months, staged | DB without C; DB in wasm sandbox/browser |
| 4. PostgreSQL driver; OPFS / S3 eedb devices | 2b (+3 for devices) | ~week each | remote and browser storage |
| 5. Second kernel / compiler | 1 | weeks each | no single host defines EELisp |

0 and 2b are independent and cheap; doing both first gives the most information for the least
commitment.

## Open questions

- Should library code (`lib/*.eel`) be embedded in the binary like the prelude, or loaded from
  disk with an embedded fallback?
- How much performance can we lose in Phase 1 before hot builtins must stay native? Needs a
  benchmark set (agenda rules over N items, sheet recalculation, a long `loop`).
- s-expression SQL: quoted data or a macro that captures variables? Data is simpler to compile;
  a macro is nicer to write.
- Minimum guarantees in the driver interface (single writer? isolation level?).
