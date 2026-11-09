//! High-level API — the surface a host (REPL, native binding, WASM) calls (ANALYSIS §5).

use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "sqlite")]
use crate::agenda::{self, Agendas};
#[cfg(feature = "sqlite")]
use crate::database::Database;
use crate::editor::EditorHost;
use crate::env::{self, Env};
use crate::output::OutputState;
use crate::value::{LispError, Value};
use crate::docs::{self, SourceIndex};
use crate::{builtins, editor, eval, output, parser, prelude};
#[cfg(feature = "sqlite")]
use crate::{agenda_builtins, db_builtins, sheet_builtins};

pub struct Interpreter {
    pub global: Env,
    #[cfg(feature = "sqlite")]
    pub database: Rc<RefCell<Database>>,
    #[cfg(feature = "sqlite")]
    pub agendas: Rc<RefCell<Agendas>>,
    pub output: Rc<RefCell<OutputState>>,
    /// Host-installed editor callbacks (buffer-text / insert-at / …). Empty ⇒ headless.
    pub editor: Rc<RefCell<EditorHost>>,
    /// How each top-level definition was written — what `(source f)` reads back.
    pub sources: Rc<RefCell<SourceIndex>>,
    /// Why the database in use isn't the one the host asked for, when it isn't. `(database-info)`
    /// reports it, so a host can tell the user their data is not being saved.
    #[cfg(feature = "sqlite")]
    pub database_error: Rc<RefCell<Option<String>>>,
    /// The open sheets — shared with the `sheet-*` builtins.
    #[cfg(feature = "sqlite")]
    pub sheets: Rc<RefCell<crate::sheet::Sheets>>,
    /// The compiler's `compile` function once compiled mode has loaded it (`set_compiled`), held
    /// here so a program defining its own `compile` can't break the engine.
    compiler: RefCell<Option<Value>>,
    /// Whether top-level forms are compiled and run on the VM instead of interpreted.
    compiled: std::cell::Cell<bool>,
}

impl Interpreter {
    /// In-memory database (the default).
    #[cfg(feature = "sqlite")]
    pub fn new() -> Self {
        Self::with_database(":memory:")
    }

    /// The language alone: built without the `sqlite` feature there's no database, agenda or
    /// sheets.
    #[cfg(not(feature = "sqlite"))]
    pub fn new() -> Self {
        Self::build()
    }

    /// Open (or create) a database at `path` — `:memory:` for in-memory. Panics if it can't be
    /// opened; a host that must keep running uses [`Interpreter::with_database_or_memory`].
    #[cfg(feature = "sqlite")]
    pub fn with_database(path: &str) -> Self {
        Self::try_with_database(path).unwrap_or_else(|e| panic!("failed to open database {}: {}", path, e))
    }

    /// Open (or create) a database at `path`, creating its folder if needed.
    #[cfg(feature = "sqlite")]
    pub fn try_with_database(path: &str) -> Result<Self, LispError> {
        Ok(Self::build(open_database_file(path)?))
    }

    /// Open `path`, or — if that fails — run on an in-memory database and remember why, so the
    /// engine still answers and `(database-info)` can say the data isn't being kept.
    #[cfg(feature = "sqlite")]
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

    fn build(#[cfg(feature = "sqlite")] database: Database) -> Self {
        let global = env::root();
        builtins::register(&global);
        crate::bytes::register(&global);

        #[cfg(feature = "sqlite")]
        let (reg, db, database_error) = {
            let reg = Rc::new(RefCell::new(Agendas::new(agenda::agenda_name_from_path(database.path()))));
            let db = Rc::new(RefCell::new(database));
            let database_error = Rc::new(RefCell::new(None));
            db_builtins::register(&global, db.clone());
            db_builtins::register_info(&global, db.clone(), database_error.clone());
            agenda_builtins::register(&global, db.clone(), reg.clone());
            (reg, db, database_error)
        };

        // output capture (echo to stdout by default — CLI/REPL) overrides the stdout print/println
        let out = Rc::new(RefCell::new(OutputState { buffer: String::new(), echo: true }));
        output::register(&global, out.clone());

        // editor RPC bridge
        let ed = Rc::new(RefCell::new(EditorHost::default()));
        editor::register(&global, ed.clone());

        // sheets — their names resolve against the host's (current-dir)
        #[cfg(feature = "sqlite")]
        let sheets = Rc::new(RefCell::new(crate::sheet::Sheets::default()));
        // formulas note the tables they read, so a write to one can recalculate them
        #[cfg(feature = "sqlite")]
        {
            sheets.borrow_mut().database = Some(db.clone());
        }
        #[cfg(feature = "sqlite")]
        sheet_builtins::register(&global, ed.clone(), sheets.clone());

        // (functions …) / (source …) — needs the output channel and the definition index
        let sources = Rc::new(RefCell::new(SourceIndex::default()));
        docs::register(&global, out.clone(), sources.clone());

        let it = Interpreter {
            global,
            #[cfg(feature = "sqlite")]
            database: db,
            #[cfg(feature = "sqlite")]
            agendas: reg,
            output: out,
            editor: ed,
            sources,
            #[cfg(feature = "sqlite")]
            database_error,
            #[cfg(feature = "sqlite")]
            sheets,
            compiler: RefCell::new(None),
            compiled: std::cell::Cell::new(false),
        };
        if let Err(e) = it.eval_str(prelude::PRELUDE) {
            panic!("prelude failed to load: {}", e);
        }
        // EELISP_COMPILED=1 starts every engine in compiled mode — how the whole test suite is run
        // on the VM, and a way to try it in any host without changing the host.
        if std::env::var("EELISP_COMPILED").is_ok_and(|v| v == "1" || v == "true") {
            if let Err(e) = it.set_compiled(true) {
                panic!("compiled mode failed to start: {}", e);
            }
        }
        it
    }

    // ── compiled mode ────────────────────────────────────────────────

    /// Compile each top-level form and run it on the bytecode VM, instead of interpreting it
    /// (docs/plans/roadmap.md, step 4). The first time it's turned on, the compiler is loaded from
    /// its bootstrap image — compiled already, its source never interpreted — and the prelude is
    /// recompiled, so its functions and macros run on the VM too.
    ///
    /// Results are the same either way: the differential tests hold both engines to that. What
    /// still runs on the interpreter: `(eval …)`, agenda rules and sheet formulas.
    pub fn set_compiled(&self, on: bool) -> Result<(), LispError> {
        if on && self.compiler.borrow().is_none() {
            crate::boot::load_image(self, crate::boot::IMAGE)?;
            let compile = env::get(&self.global, "compile")?;
            *self.compiler.borrow_mut() = Some(compile);
            for form in parser::parse(prelude::PRELUDE)? {
                self.compile_and_run(&form)?;
            }
        }
        self.compiled.set(on);
        Ok(())
    }

    /// Whether top-level forms are compiled (`set_compiled`).
    pub fn is_compiled(&self) -> bool {
        self.compiled.get()
    }

    /// Evaluate one top-level form, the way the engine is set to.
    fn eval_form(&self, form: &Value) -> Result<Value, LispError> {
        if self.compiled.get() {
            self.compile_and_run(form)
        } else {
            eval::eval(form.clone(), self.global.clone())
        }
    }

    fn compile_and_run(&self, form: &Value) -> Result<Value, LispError> {
        let compile = self.compiler.borrow().clone().expect("compiled mode without a compiler");
        let proto = eval::apply_value(&compile, std::slice::from_ref(form), &self.global)?;
        crate::vm::run_in(crate::vm::assemble(&proto)?, self.global.clone())
    }

    /// Point the engine at another database file — a host whose workspace moved calls this. The
    /// definitions in the environment stay; the tables, items and agenda files are the new
    /// file's. Agendas opened with `open-agenda` are closed.
    ///
    /// If the file can't be opened the engine moves to an in-memory database rather than staying
    /// on the old one: writing into the previous workspace's data would be the worse surprise.
    #[cfg(feature = "sqlite")]
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
    #[cfg(feature = "sqlite")]
    pub fn export_database(&self) -> Result<Vec<u8>, LispError> {
        self.database.borrow().to_bytes()
    }

    /// Replace the engine's database with one given as bytes (see [`Interpreter::export_database`]).
    /// Like [`Interpreter::open_database`], definitions in the environment stay; the data is the
    /// new one's. Bytes that aren't a SQLite database are refused and the current data kept.
    #[cfg(feature = "sqlite")]
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

    /// A sheet given as the bytes of a `.eesheet` file, open from now on under `name` — resolved the
    /// way `(sheet-open name)` resolves it, so the formulas and forms that name it find it.
    #[cfg(feature = "sqlite")]
    pub fn import_sheet(&self, name: &str, bytes: &[u8]) -> Result<(), LispError> {
        let path = sheet_builtins::resolve(&self.editor, name)?;
        self.sheets.borrow_mut().import(&path, bytes)
    }

    /// An open sheet as the bytes of a `.eesheet` file.
    #[cfg(feature = "sqlite")]
    pub fn export_sheet(&self, name: &str) -> Result<Vec<u8>, LispError> {
        let path = sheet_builtins::resolve(&self.editor, name)?;
        let mut sheets = self.sheets.borrow_mut();
        let sheet = sheets
            .loaded(&path)
            .ok_or_else(|| LispError::Runtime(format!("{} isn't open", path.display())))?;
        sheet.to_bytes()
    }

    /// Every open sheet, as `(path, version)` — a version that moved is a sheet to keep again.
    #[cfg(feature = "sqlite")]
    pub fn sheet_versions(&self) -> Vec<(String, i64)> {
        self.sheets.borrow().versions().into_iter().map(|(p, v)| (p.display().to_string(), v)).collect()
    }

    /// Changes to the database's rows since it was opened or imported.
    #[cfg(feature = "sqlite")]
    pub fn database_changes(&self) -> u64 {
        self.database.borrow().total_changes()
    }

    /// Evaluate all top-level forms, return the last result.
    pub fn eval_str(&self, src: &str) -> Result<Value, LispError> {
        let forms = parser::top_forms(src)?;
        let mut result = Value::Null;
        for f in forms {
            let r = self.eval_form(&f.value);
            self.follow_tables();
            result = r?;
            self.sources.borrow_mut().record(&f);
        }
        Ok(result)
    }

    /// Evaluate all top-level forms, return each result.
    pub fn eval_all(&self, src: &str) -> Result<Vec<Value>, LispError> {
        let forms = parser::top_forms(src)?;
        let mut out = Vec::with_capacity(forms.len());
        for f in forms {
            let r = self.eval_form(&f.value);
            self.follow_tables();
            out.push(r?);
            self.sources.borrow_mut().record(&f);
        }
        Ok(out)
    }

    /// Tables written by what just ran recalculate the open sheets' formulas that read them —
    /// once per top-level form, so a loop of inserts recalculates once, at the end. Runs even when
    /// the form failed: what it wrote before failing is written.
    fn follow_tables(&self) {
        #[cfg(feature = "sqlite")]
        {
            let written = self.database.borrow().take_written();
            crate::sheet_builtins::follow_tables(&self.sheets, &self.global, &written);
        }
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

    // ── giving the language new powers ───────────────────────────────

    /// Add a function to the language, written in Rust: how a host lets scripts reach into it (a
    /// game's `set-block`, an app's `notify`). Its arguments arrive evaluated. It replaces any
    /// function of the same name.
    ///
    /// ```
    /// use eelisp::{Interpreter, Value};
    /// let it = Interpreter::new();
    /// it.define_fn("double", |args| match args.first() {
    ///     Some(Value::Number(n)) => Ok(Value::Number(n * 2.0)),
    ///     _ => Err(eelisp::LispError::Runtime("double wants a number".into())),
    /// });
    /// assert!(matches!(it.eval_str("(double 21)"), Ok(Value::Number(n)) if n == 42.0));
    /// ```
    pub fn define_fn(&self, name: &str, f: impl Fn(&[Value]) -> Result<Value, LispError> + 'static) {
        env::define(
            &self.global,
            name,
            Value::Builtin(Rc::new(crate::value::Builtin {
                name: name.into(),
                arg_mode: crate::value::ArgMode::Eval,
                func: Box::new(move |args, _| f(args)),
            })),
        );
    }

    /// Call a function the script defined, by name, with these arguments: how a host tells a
    /// script that something happened (`(on-join player name)`). `None` if there's no such
    /// function, so a script only defines the handlers it wants.
    pub fn call(&self, name: &str, args: &[Value]) -> Option<Result<Value, LispError>> {
        let f = env::get(&self.global, name).ok()?;
        if !matches!(f, Value::Function(_) | Value::Builtin(_)) {
            return None;
        }
        let r = eval::apply_value(&f, args, &self.global);
        self.follow_tables();
        Some(r)
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
#[cfg(feature = "sqlite")]
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
