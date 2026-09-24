# eelisp-web — the engine in a browser

The EELisp interpreter and SQLite, compiled to WebAssembly. One `Engine` is one interpreter with its
own in-memory database; `eval(src)` returns the same JSON envelope the native hosts get.

```js
import init, { Engine } from "./pkg/eelisp_web.js";
await init();
const e = new Engine();
JSON.parse(e.eval('(deftable pets (name:string)) (insert pets {:name "Rex"}) (query pets)'));
```

Build with `./build.sh` (see the top of it for the one-time setup). `spike/index.html` is the
feasibility check: serve this folder (`python3 -m http.server -d web`) and open `/spike/`.
`spike/prelude.eelisp` and `spike/Books.eeform` are copies from eeditor-next, to run a real form.

## What the spike showed (2026-09-24, headless Chromium, M-series Mac)

| | |
|---|---|
| builds | the whole engine, SQLite included (`rusqlite` 0.40 → `sqlite-wasm-rs` on this target) |
| size | 2.7 MB `.wasm`, **1.06 MB gzipped** (release, `opt-level = "s"`, LTO) |
| start | `init()` + `new Engine()` in ~21 ms |
| SQLite | 10,000 inserts in ~51 ms; a filtered, sorted query over them in ~2 ms |
| language | tail calls, strings, dates (clock from `Date.now()`), the agenda, `function-list`/`source-text` |
| forms | the forms prelude and `Books.eeform` load; its handlers return the same UI queue as in the app |
| not here | `http-get`/`http-post` (the `http` feature is off; they say so instead of crashing), `std::fs` (agenda export/import fail cleanly), `EngineHandle` (no threads — the page calls the engine directly) |

Still to decide: where the database lives between visits (in memory today; OPFS or an
export/import of the `.db` file are the candidates), and whether the engine runs on the page or in
a Web Worker.
