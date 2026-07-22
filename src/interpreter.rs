//! High-level API — the surface a host (REPL, native binding, WASM) calls (ANALYSIS §5).

use std::cell::RefCell;
use std::rc::Rc;

use crate::agenda::{self, Agendas};
use crate::database::Database;
use crate::editor::EditorHost;
use crate::env::{self, Env};
use crate::output::OutputState;
use crate::value::{LispError, Value};
use crate::{agenda_builtins, builtins, db_builtins, editor, eval, output, parser, prelude};

pub struct Interpreter {
    pub global: Env,
    pub database: Rc<RefCell<Database>>,
    pub agendas: Rc<RefCell<Agendas>>,
    pub output: Rc<RefCell<OutputState>>,
    /// Host-installed editor callbacks (buffer-text / insert-at / …). Empty ⇒ headless.
    pub editor: Rc<RefCell<EditorHost>>,
}

impl Interpreter {
    /// In-memory database (the default).
    pub fn new() -> Self {
        Self::with_database(":memory:")
    }

    /// Open (or create) a database at `path` — `:memory:` for in-memory.
    pub fn with_database(path: &str) -> Self {
        let global = env::root();
        builtins::register(&global);

        let mut database = Database::open(path).expect("failed to open database");
        agenda::ensure_agenda_tables(&mut database).expect("failed to create agenda tables");
        let db = Rc::new(RefCell::new(database));
        let reg = Rc::new(RefCell::new(Agendas::new(agenda::agenda_name_from_path(path))));

        db_builtins::register(&global, db.clone());
        agenda_builtins::register(&global, db.clone(), reg.clone());

        // output capture (echo to stdout by default — CLI/REPL) overrides the stdout print/println
        let out = Rc::new(RefCell::new(OutputState { buffer: String::new(), echo: true }));
        output::register(&global, out.clone());

        // editor RPC bridge
        let ed = Rc::new(RefCell::new(EditorHost::default()));
        editor::register(&global, ed.clone());

        let it = Interpreter { global, database: db, agendas: reg, output: out, editor: ed };
        if let Err(e) = it.eval_str(prelude::PRELUDE) {
            panic!("prelude failed to load: {}", e);
        }
        it
    }

    /// Evaluate all top-level forms, return the last result.
    pub fn eval_str(&self, src: &str) -> Result<Value, LispError> {
        let forms = parser::parse(src)?;
        let mut result = Value::Null;
        for f in forms {
            result = eval::eval(f, self.global.clone())?;
        }
        Ok(result)
    }

    /// Evaluate all top-level forms, return each result.
    pub fn eval_all(&self, src: &str) -> Result<Vec<Value>, LispError> {
        let forms = parser::parse(src)?;
        let mut out = Vec::with_capacity(forms.len());
        for f in forms {
            out.push(eval::eval(f, self.global.clone())?);
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

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}
