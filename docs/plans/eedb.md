# eedb: a native, portable SQL engine for EELisp (and anyone else)

*Status: idea / first discussion — 2026-11-04. Companion to [self-hosting.md](self-hosting.md).*

## The idea in one paragraph

SQL is good. EELisp keeps speaking it, as a first-class language for data. What changes is that
**SQLite becomes one backend behind a SQL driver interface**, and next to it sits **eedb**: a
separate project, an embedded SQL database engine that is fast, native, portable (including
WASM with no C toolchain), with storage that can live in a file, in memory, in the browser, or
in object storage. EELisp is its first user; it should be useful without EELisp too.

## Why bother, when SQLite exists?

SQLite is one of the best pieces of software ever written. eedb only makes sense if it does
things SQLite structurally doesn't. Candidates, strongest first:

1. **WASM and sandboxes without C.** `rusqlite` bundles SQLite's C source; that's why the
   `language-only` wasm32 build runs with no database at all. An engine in a language that
   targets `wasm32-unknown-unknown` natively gives the sandbox and `eelisp-web` a real database.
2. **Pluggable storage devices.** SQLite's VFS can do this, but it's a C-level, page-oriented
   interface. eedb designed around a small device interface from day one: file, memory,
   OPFS/IndexedDB, S3-style object segments.
3. **EELisp-native values.** Dicts, lists, dates, items as real column types instead of JSON
   text + `json_extract`. The agenda's `LIKE '%"cat"%'` on serialized categories becomes a
   proper `contains` over a list column, with an index.
4. **Change feeds.** Today the engine polls `PRAGMA data_version`. An engine that can say *which*
   rows of *which* tables changed makes sheet recalculation, agenda views and editor refresh
   reactive and cheap.
5. **History for free.** With a log-structured design, "what did this record look like last
   Tuesday" and undo are natural, not a feature bolted on.
6. **Deterministic snapshots.** One canonical byte image of a database, good for
   `to_bytes`/`from_bytes` across the host boundary, sync, and content-addressed storage.

If none of these matter, use SQLite. If two or three do, eedb is a real project.

## What it is not

- Not a SQLite clone. No promise of reading or writing SQLite's file format natively; SQLite
  files come in through an importer (via the SQLite driver).
- Not a server, at first. Embedded, single process, one writer. Networking is a host concern.
- Not "all of SQL". A defined subset, grown by real need, tested against SQLite.

## Layers

```
EELisp code ── (query …), (sql "…" params), optional s-expr SQL
      │
SQL driver interface (in the EELisp kernel/host boundary)
  open · prepare · bind · step · exec · begin/commit/rollback · snapshot/restore · changes
      │
 ┌────┴──────────────┬─────────────────────┬──────────────────┐
SQLite driver     eedb driver          PostgreSQL driver   (others)
(today's code)    (new project)        (dialect: $1, jsonb)
                      │
              eedb: SQL parser → planner → executor
                      │
              storage engine (tables, indexes, transactions)
                      │
              device: file · memory · OPFS · S3 segments · host callback
```

Two points about this shape:

- **The driver interface is SQL-level** (like JDBC/DBI), not record-level. EELisp sends SQL and
  parameters, gets rows back. Dialect differences live in a thin dialect layer per driver.
- **eedb is a library with a C ABI** (plus a native API in its own language), so any EELisp kernel
  — Rust, C, Swift, JS via WASM — can link it. It does not depend on EELisp.

## How EELisp talks SQL

Keep what works, add structure where it helps:

- `(query books :where "year > ?" :params (list 1980))` — stays. It's SQL; that's fine.
- `(sql "SELECT title FROM books WHERE year > ?" 1980)` — a direct escape to the driver for
  anything `query` doesn't cover (joins, aggregates, `GROUP BY`).
- **Optional** s-expression SQL, compiled to text by an EELisp library, never replacing strings:

  ```lisp
  (select [title year] :from books :where (> year ?) :order-by year)
  ```

  Useful for macros and generated queries; parameters stay bound, never pasted.

## The SQL subset EELisp needs today

Surveyed from the engine source (2026-11-04). This is eedb's **v0 target**:

| Feature | Used by |
|---|---|
| `CREATE TABLE IF NOT EXISTS`, `DROP TABLE IF EXISTS`, `INTEGER PRIMARY KEY`, `TEXT`, `REAL`, `NOT NULL` | database, sheets, schema table |
| `INSERT`, `INSERT OR REPLACE` | database, sheets (cells, widths, heights, meta) |
| `UPDATE … SET … WHERE` | records, soft delete (`_deleted = 1`) |
| `DELETE … WHERE` | `pack`, sheet cells/rows/cols |
| `SELECT cols FROM t WHERE … ORDER BY … ASC/DESC LIMIT n` | `query`, agenda, sheets |
| `COUNT(*)` | `count-records`, agenda |
| `?` / `?N` bound parameters | everywhere |
| `AND`, `OR`, comparisons, `LIKE` | user `:where`, agenda search |
| `json_extract(col, '$.when')` | agenda date queries |
| `BEGIN` / `COMMIT` / `ROLLBACK` | batches, rules, sheet edits |
| `PRAGMA application_id`, `user_version`, `data_version` | sheet file identity, versioning, change detection |
| Serialize / deserialize whole DB to bytes | `to_bytes`/`from_bytes`, web export |

No joins, no subqueries, no aggregates beyond `COUNT(*)`, no triggers. That is a small,
well-defined engine. User `:where` strings can contain anything SQLite accepts, so eedb should
reject what it doesn't support **with a clear error**, never guess.

v1 candidates, driven by users rather than completeness: `GROUP BY` + `SUM/MIN/MAX/AVG`,
inner joins, secondary indexes (`CREATE INDEX`), `IN (…)`, list/dict column types.

## Inside eedb (first design sketch)

- **Parser** → AST for the subset; errors with positions.
- **Planner**: deliberately simple — full scan, primary-key lookup, single secondary-index
  lookup; `ORDER BY` by index or sort; `LIMIT` early-out. No cost-based optimizer until a real
  workload needs one.
- **Executor**: iterator (volcano) pipeline — scan → filter → sort → limit → project.
- **Storage engine**, in two steps:
  1. **Log-structured + in-memory**: append-only operation log, tables and indexes rebuilt in
     memory on open, `COMMIT` markers for atomicity, compaction (which is what `pack` already
     means). Simple, crash-safe, fits S3 segments and snapshots, gives history. Limit: data must
     fit in memory — fine for agendas, sheets, personal databases.
  2. **Paged B-tree** (later, if needed): when datasets outgrow memory. Same device interface,
     same SQL layers above.
- **Device interface**: read-range, append, write, sync, truncate, rename — or just
  load-bytes/save-bytes for the simplest hosts. Implemented by the host (keeps the "core does no
  I/O" rule for WASM and sandboxes).
- **Types**: SQLite-compatible affinities for v0 (INTEGER, REAL, TEXT, BLOB, NULL) so the SQLite
  and eedb drivers return identical results; EELisp-native types (list, dict, date) are a v1
  extension behind a flag.

## Testing: borrow the industry's oracle

- **sqllogictest** format (used by SQLite, DuckDB, CockroachDB and others): plain-text files of
  SQL statements and expected results. Write the v0 subset as sqllogictest files.
- **Differential against SQLite**: run every test, and fuzzed queries, on both; results must
  match for the supported subset.
- **The EELisp suite on both drivers**: `tests/database.rs`, `db_agenda.rs`, `sheet.rs` must pass
  with SQLite and with eedb. That's eedb's definition of "ready for EELisp".
- **Crash tests**: kill mid-commit, reopen, check atomicity — the log design makes this testable
  with a fault-injecting device.

## Prior art to study first

Not to adopt blindly — to learn from, and to check whether eedb's niche is already filled.
(Status of each should be re-checked; this is from memory.)

- **GlueSQL** (Rust): SQL engine with pluggable storage backends (memory, sled, JSON,
  IndexedDB…). Closest to the "SQL front + storage trait" shape; read its storage trait.
- **Limbo / Turso's Rust SQLite rewrite**: SQLite-compatible engine in Rust; shows what full
  compatibility costs.
- **redb, sled** (Rust KV stores): candidate storage layers under a SQL front end.
- **DuckDB**: columnar and analytical — a different niche, but its sqllogictest usage and
  extension model are worth reading.
- **SQLite's own docs** on file format, VFS, and testing — the bar to measure against.

An honest outcome of this research could be "use GlueSQL (or similar) as eedb's core and
contribute the missing parts". That's still a win: the driver interface is what EELisp needs.

## Language for eedb

eedb is a separate project, so it doesn't inherit the "EELisp shouldn't be defined by Rust"
concern. The requirements are: native speed, `wasm32-unknown-unknown` with no C, a C ABI, no
mandatory threads.

- **Rust**: meets all of them; the existing team and code are here; most prior art is Rust.
- **Zig / C**: smallest footprint and easiest C ABI; C reintroduces the WASM toolchain issue
  that motivated eedb.
- **EELisp itself**: the long-term dream (an EELisp reference implementation of the storage and
  SQL layers is a great spec and test oracle), but too slow as the production engine until
  EELisp has a compiler.

Leaning Rust for the engine, with an EELisp reference model of the log store as a later spec.

## Milestones

| # | Milestone | Done when |
|---|---|---|
| 0 | SQL driver interface in EELisp; SQLite behind it | full test suite passes, nothing above the driver calls rusqlite |
| 1 | sqllogictest files for the v0 subset, verified on SQLite | they pass on SQLite |
| 2 | eedb v0: parser, simple planner, executor, in-memory storage | sqllogictest v0 passes |
| 3 | eedb log store on file + memory devices; snapshots | crash tests pass; `to_bytes`/`from_bytes` round-trips |
| 4 | eedb driver in EELisp | database, agenda, sheet suites pass on eedb |
| 5 | wasm32 sandbox and `eelisp-web` ship with eedb | `language-only` CI job has a database |
| 6 | OPFS and S3-segment devices; change feed | browser persistence; reactive sheet/agenda refresh |

Milestones 0 and 1 are useful even if eedb never ships: a clean driver interface (and the
PostgreSQL driver it enables) plus a precise spec of the SQL EELisp depends on.

## Open questions

- **Name and home**: its own repo (`eedb`), same org? Its own license?
- **SQLite file compatibility**: import/export only (recommended), or read SQLite files natively?
- **Dialect**: strict SQLite-subset compatibility, or allow eedb extensions (list/dict columns,
  `contains`) when the EELisp driver knows it's talking to eedb?
- **Concurrency**: single writer forever, or plan for multiple processes on one file?
- **Who else is it for?** If eedb should stand alone, which non-EELisp use (a game mod sandbox,
  a browser app, an edge function) is its second user and shapes the API?
