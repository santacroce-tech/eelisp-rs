//! High-level API — the surface a host (REPL, native binding, WASM) calls (ANALYSIS §5).

use std::cell::RefCell;
use std::rc::Rc;

use crate::agenda::{self, Agendas};
use crate::database::Database;
use crate::editor::EditorHost;
use crate::env::{self, Env};
use crate::output::OutputState;
use crate::value::{LispError, Value};
use crate::docs::{self, SourceIndex};
use crate::{agenda_builtins, builtins, db_builtins, editor, eval, output, parser, prelude, sheet_builtins};

pub struct Interpreter {
    pub global: Env,
    pub database: Rc<RefCell<Database>>,
    pub agendas: Rc<RefCell<Agendas>>,
    pub output: Rc<RefCell<OutputState>>,
    /// Host-installed editor callbacks (buffer-text / insert-at / …). Empty ⇒ headless.
    pub editor: Rc<RefCell<EditorHost>>,
    /// How each top-level definition was written — what `(source f)` reads back.
    pub sources: Rc<RefCell<SourceIndex>>,
    /// Why the database in use isn't the one the host asked for, when it isn't. `(database-info)`
    /// reports it, so a host can tell the user their data is not being saved.
    pub database_error: Rc<RefCell<Option<String>>>,
}

impl Interpreter {
    /// In-memory database (the default).
    pub fn new() -> Self {
        Self::with_database(":memory:")
    }

    /// Open (or create) a database at `path` — `:memory:` for in-memory. Panics if it can't be
    /// opened; a host that must keep running uses [`Interpreter::with_database_or_memory`].
    pub fn with_database(path: &str) -> Self {
        Self::try_with_database(path).unwrap_or_else(|e| panic!("failed to open database {}: {}", path, e))
    }

    /// Open (or create) a database at `path`, creating its folder if needed.
    pub fn try_with_database(path: &str) -> Result<Self, LispError> {
        Ok(Self::build(open_database_file(path)?))
    }

    /// Open `path`, or — if that fails — run on an in-memory database and remember why, so the
    /// engine still answers and `(database-info)` can say the data isn't being kept.
    pub fn with_database_or_memory(path: &str) -> Self {
        match Self::try_with_database(path) {
            Ok(it) => it,
            Err(e) => {
                let it = Self::new();
                *it.database_error.borrow_mut() = Some(format!("couldn't open {}: {}", path, e));
                it
            }
        }
    }

    fn build(database: Database) -> Self {
        let global = env::root();
        builtins::register(&global);

        let reg = Rc::new(RefCell::new(Agendas::new(agenda::agenda_name_from_path(database.path()))));
        let db = Rc::new(RefCell::new(database));
        let database_error = Rc::new(RefCell::new(None));

        db_builtins::register(&global, db.clone());
        db_builtins::register_info(&global, db.clone(), database_error.clone());
        agenda_builtins::register(&global, db.clone(), reg.clone());

        // output capture (echo to stdout by default — CLI/REPL) overrides the stdout print/println
        let out = Rc::new(RefCell::new(OutputState { buffer: String::new(), echo: true }));
        output::register(&global, out.clone());

        // editor RPC bridge
        let ed = Rc::new(RefCell::new(EditorHost::default()));
        editor::register(&global, ed.clone());

        // sheets — their names resolve against the host's (current-dir)
        sheet_builtins::register(&global, ed.clone());

        // (functions …) / (source …) — needs the output channel and the definition index
        let sources = Rc::new(RefCell::new(SourceIndex::default()));
        docs::register(&global, out.clone(), sources.clone());

        let it = Interpreter {
            global,
            database: db,
            agendas: reg,
            output: out,
            editor: ed,
            sources,
            database_error,
        };
        if let Err(e) = it.eval_str(prelude::PRELUDE) {
            panic!("prelude failed to load: {}", e);
        }
        it
    }

    /// Point the engine at another database file — a host whose workspace moved calls this. The
    /// definitions in the environment stay; the tables, items and agenda files are the new
    /// file's. Agendas opened with `open-agenda` are closed.
    ///
    /// If the file can't be opened the engine moves to an in-memory database rather than staying
    /// on the old one: writing into the previous workspace's data would be the worse surprise.
    pub fn open_database(&self, path: &str) -> Result<(), LispError> {
        let (next, result) = match open_database_file(path) {
            Ok(d) => (d, Ok(())),
            Err(e) => (open_database_file(":memory:")?, Err(e)),
        };
        let mut reg = self.agendas.borrow_mut();
        reg.active_name = agenda::agenda_name_from_path(next.path());
        reg.inactive.clear();
        *self.database.borrow_mut() = next;
        *self.database_error.borrow_mut() =
            result.as_ref().err().map(|e| format!("couldn't open {}: {}", path, e));
        result
    }

    /// The engine's database as the bytes of a SQLite file — tables, items, rules, views.
    pub fn export_database(&self) -> Result<Vec<u8>, LispError> {
        self.database.borrow().to_bytes()
    }

    /// Replace the engine's database with one given as bytes (see [`Interpreter::export_database`]).
    /// Like [`Interpreter::open_database`], definitions in the environment stay; the data is the
    /// new one's. Bytes that aren't a SQLite database are refused and the current data kept.
    pub fn import_database(&self, bytes: &[u8]) -> Result<(), LispError> {
        let mut next = Database::from_bytes(bytes)?;
        agenda::ensure_agenda_tables(&mut next)?;
        let mut reg = self.agendas.borrow_mut();
        reg.active_name = agenda::agenda_name_from_path(next.path());
        reg.inactive.clear();
        *self.database.borrow_mut() = next;
        *self.database_error.borrow_mut() = None;
        Ok(())
    }

    /// Changes to the database's rows since it was opened or imported.
    pub fn database_changes(&self) -> u64 {
        self.database.borrow().total_changes()
    }

    /// Evaluate all top-level forms, return the last result.
    pub fn eval_str(&self, src: &str) -> Result<Value, LispError> {
        let forms = parser::top_forms(src)?;
        let mut result = Value::Null;
        for f in forms {
            result = eval::eval(f.value.clone(), self.global.clone())?;
            self.sources.borrow_mut().record(&f);
        }
        Ok(result)
    }

    /// Evaluate all top-level forms, return each result.
    pub fn eval_all(&self, src: &str) -> Result<Vec<Value>, LispError> {
        let forms = parser::top_forms(src)?;
        let mut out = Vec::with_capacity(forms.len());
        for f in forms {
            out.push(eval::eval(f.value.clone(), self.global.clone())?);
            self.sources.borrow_mut().record(&f);
        }
        Ok(out)
    }

    // ── host boundary (ANALYSIS §5) ──────────────────────────────────

    /// Evaluate and return the result value as JSON (tagged encoding, see `host::to_json`).
    pub fn eval_json(&self, src: &str) -> Result<String, LispError> {
        Ok(crate::host::to_json(&self.eval_str(src)?).to_string())
    }

    /// Evaluate for a frontend: returns a JSON envelope with the result, captured output, and any
    /// error — `{ "ok": true, "result": <json>, "output": "…" }` or `{ "ok": false, "error": "…" }`.
    pub fn eval_host(&self, src: &str) -> String {
        self.output.borrow_mut().buffer.clear();
        let result = self.eval_str(src);
        let captured = std::mem::take(&mut self.output.borrow_mut().buffer);
        let envelope = match result {
            Ok(v) => serde_json::json!({ "ok": true, "result": crate::host::to_json(&v), "output": captured }),
            Err(e) => serde_json::json!({ "ok": false, "error": e.to_string(), "output": captured }),
        };
        envelope.to_string()
    }

    /// Toggle whether captured output also mirrors to stdout (on for CLI, off for a host).
    pub fn set_echo(&self, echo: bool) {
        self.output.borrow_mut().echo = echo;
    }

    /// Drain and return whatever `print`/`println` have accumulated.
    pub fn take_output(&self) -> String {
        std::mem::take(&mut self.output.borrow_mut().buffer)
    }
}

/// Open a database with the agenda tables in place. A file's folder is created first — a fresh
/// workspace has no `.eeditor/` yet, and SQLite won't make one.
fn open_database_file(path: &str) -> Result<Database, LispError> {
    if path != ":memory:" && !path.is_empty() {
        if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| LispError::Database(format!("can't create the folder {}: {}", dir.display(), e)))?;
        }
    }
    let mut database = Database::open(path)?;
    agenda::ensure_agenda_tables(&mut database)?;
    Ok(database)
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}
