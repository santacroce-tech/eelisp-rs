//! Sheets — a grid of cells whose formulas are EELisp, each sheet a SQLite file of its own
//! (`Name.eesheet`).
//!
//! This module is the store and the dependency graph. It never evaluates anything: running a
//! formula means calling back into the interpreter, and a formula may itself read a sheet, so
//! `sheet_builtins` drives recalculation and only borrows a sheet between formulas.
//!
//! What a cell holds is decided by its **input** — exactly what was typed:
//! `""` is empty · `1200` a number · `'1200` the text "1200" · `=(sum B1:B2)` a formula · anything
//! else text. A formula's last result is stored with it, so opening a sheet shows values without
//! running a line of code.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rusqlite::{params, Connection, OpenFlags, Transaction};
use serde_json::{Map, Value as J};

use crate::host::{from_json, from_tagged_json, to_json};
use crate::parser;
use crate::printer::print_value;
use crate::sheet_ref::*;
use crate::value::*;

pub const EXT: &str = "eesheet";
/// `PRAGMA application_id` — "EESH". A sheet is recognised by its bytes, not its name.
const APPLICATION_ID: i64 = 0x4545_5348;
const SCHEMA_VERSION: i64 = 1;
/// A range this big is a typo, and reading it would build a list that size.
pub const MAX_RANGE_CELLS: u64 = 1_000_000;

const SCHEMA: &str = "
CREATE TABLE cells (
  row   INTEGER NOT NULL,            -- 0-based
  col   INTEGER NOT NULL,            -- 0-based
  input TEXT    NOT NULL DEFAULT '', -- exactly what was typed
  value TEXT,                        -- the last result, as tagged JSON
  error TEXT,                        -- why the formula failed, if it did
  fmt   TEXT,                        -- a JSON object: {\"num\":\"currency\",\"dp\":2,\"bold\":true}
  PRIMARY KEY (row, col)
) WITHOUT ROWID;
CREATE TABLE widths (col INTEGER PRIMARY KEY, width REAL NOT NULL);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

fn fail(msg: impl Into<String>) -> LispError {
    LispError::Runtime(msg.into())
}

fn sql(path: &Path) -> impl Fn(rusqlite::Error) -> LispError + '_ {
    move |e| fail(format!("{}: {}", path.display(), e))
}

// ── what a cell holds ────────────────────────────────────────────────

pub struct Formula {
    /// The parsed expression, or why the text doesn't read as one.
    pub expr: Result<Value, String>,
    /// Every distinct reference, spelled as written (`$A$1` and `A1` are both bound), and what it
    /// points at.
    pub refs: Vec<(String, RefSym)>,
}

pub struct CellData {
    pub input: String,
    pub value: Value,
    pub error: Option<String>,
    pub fmt: Option<Map<String, J>>,
    pub formula: Option<Formula>,
}

impl CellData {
    fn empty() -> Self {
        CellData { input: String::new(), value: Value::Null, error: None, fmt: None, formula: None }
    }
    /// Nothing typed and nothing formatted: no reason to keep a row for it.
    fn is_blank(&self) -> bool {
        self.input.is_empty() && self.fmt.is_none()
    }
}

pub enum Input {
    Empty,
    Literal(Value),
    Formula(Formula),
}

pub fn read_input(input: &str) -> Input {
    if input.is_empty() {
        return Input::Empty;
    }
    if let Some(body) = input.strip_prefix('=') {
        return Input::Formula(parse_formula(body));
    }
    if let Some(text) = input.strip_prefix('\'') {
        return Input::Literal(Value::Str(text.to_string()));
    }
    match looks_numeric(input.trim()).then(|| input.trim().parse::<f64>()) {
        Some(Ok(n)) if n.is_finite() => Input::Literal(Value::Number(n)),
        _ => Input::Literal(Value::Str(input.to_string())),
    }
}

/// `12`, `-3.5`, `.5`, `1e3` — but not `inf`, `NaN` or `0x1F`, which Rust would happily parse.
pub fn looks_numeric(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let (mut digits, mut dot) = (0, false);
    while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b'.' && !dot)) {
        if b[i] == b'.' {
            dot = true;
        } else {
            digits += 1;
        }
        i += 1;
    }
    if digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == b.len()
}

pub fn parse_formula(body: &str) -> Formula {
    let expr = match parser::parse(body) {
        Ok(mut forms) if forms.len() == 1 => Ok(forms.remove(0)),
        Ok(forms) if forms.is_empty() => Err("an empty formula — write an expression after =".to_string()),
        Ok(_) => Err("a formula is one expression — wrap several in (do …)".to_string()),
        Err(e) => Err(e.to_string()),
    };
    let mut refs = Vec::new();
    if let Ok(e) = &expr {
        collect_refs(e, &mut refs);
    }
    Formula { expr, refs }
}

fn collect_refs(v: &Value, out: &mut Vec<(String, RefSym)>) {
    match v {
        Value::Symbol(s) => {
            if let Some(r) = classify(s) {
                if !out.iter().any(|(name, _)| name == s) {
                    out.push((s.clone(), r));
                }
            }
        }
        Value::List(items) => items.iter().for_each(|i| collect_refs(i, out)),
        Value::Dict(d) => d.keys.iter().for_each(|k| collect_refs(&d.map[k], out)),
        _ => {}
    }
}

/// What a computed value is kept as. Data stays data; a function, record or view becomes the text
/// it prints as, because a file can't hold one — and the value in memory is made the same, so a
/// sheet reads identically before and after it is reopened.
pub fn storable(v: Value) -> Result<Value, String> {
    Ok(match v {
        Value::Number(n) if !n.is_finite() => return Err("the result is not a finite number".into()),
        Value::Number(_) | Value::Str(_) | Value::Bool(_) | Value::Null | Value::Keyword(_) | Value::Symbol(_) => v,
        Value::List(items) => {
            Value::List(Rc::new(items.iter().cloned().map(storable).collect::<Result<Vec<_>, _>>()?))
        }
        Value::Dict(d) => {
            let mut out = OrderedDict::default();
            for k in &d.keys {
                out.insert(k.clone(), storable(d.map[k].clone())?);
            }
            Value::Dict(Rc::new(out))
        }
        other => Value::Str(print_value(&other, false)),
    })
}

/// A formula that reads a failed cell fails too, naming where the trouble started — once, not as a
/// chain through every cell in between.
fn propagated(origin: CellRef, message: &str) -> String {
    const MARK: &str = " has an error — ";
    let already_points_back = message.split_once(MARK).is_some_and(|(cell, _)| parse_cell(cell).is_some());
    if already_points_back {
        message.to_string()
    } else {
        format!("{}{}{}", origin.a1(), MARK, message)
    }
}

// ── recalculation plan ───────────────────────────────────────────────

pub enum Step {
    /// Recompute this formula; everything it reads is already up to date.
    Compute(CellRef),
    /// These formulas read each other in a circle.
    Cycle(Vec<CellRef>),
}

pub enum Prepared {
    /// Evaluate `expr` with each reference bound to its value.
    Eval { expr: Value, bindings: Vec<(String, Value)> },
    /// The formula can't run: its text doesn't parse, or it reads a failed or deleted cell.
    Fail(String),
}

// ── a sheet ──────────────────────────────────────────────────────────

pub struct Sheet {
    conn: Connection,
    pub path: PathBuf,
    cells: HashMap<CellRef, CellData>,
    widths: BTreeMap<u32, f64>,
    version: i64,
    /// `PRAGMA data_version` when last loaded — it moves when *another* connection commits.
    seen_data_version: i64,
    /// cell → the formulas that name it directly.
    single_deps: HashMap<CellRef, HashSet<CellRef>>,
    /// (range, formula) for every range a formula names.
    range_deps: Vec<(Range, CellRef)>,
}

impl Sheet {
    /// A new, empty sheet file. Refuses to overwrite anything.
    pub fn create(path: &Path) -> Result<Sheet, LispError> {
        if path.exists() {
            return Err(fail(format!("{} already exists", path.display())));
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql(path))?;
        conn.execute_batch(&format!(
            "PRAGMA application_id = {APPLICATION_ID};
             PRAGMA user_version = {SCHEMA_VERSION};
             BEGIN; {SCHEMA} INSERT INTO meta (key, value) VALUES ('version', '0'); COMMIT;"
        ))
        .map_err(sql(path))?;
        Self::with_connection(conn, path)
    }

    /// An existing sheet file. Never creates one: a mistyped name is an error, not a new file.
    pub fn open(path: &Path) -> Result<Sheet, LispError> {
        if !path.is_file() {
            return Err(fail(format!("no sheet at {}", path.display())));
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(sql(path))?;
        let not_a_sheet = || fail(format!("{} is not a sheet", path.display()));
        let app_id: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0)).map_err(|_| not_a_sheet())?;
        if app_id != APPLICATION_ID {
            return Err(not_a_sheet());
        }
        let schema: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(sql(path))?;
        if schema > SCHEMA_VERSION {
            return Err(fail(format!("{} was made by a newer EEditor", path.display())));
        }
        Self::with_connection(conn, path)
    }

    fn with_connection(conn: Connection, path: &Path) -> Result<Sheet, LispError> {
        // Same reasoning as the engine database: two hosts may share a workspace.
        conn.busy_timeout(std::time::Duration::from_secs(2)).map_err(sql(path))?;
        let mut sheet = Sheet {
            conn,
            path: path.to_path_buf(),
            cells: HashMap::new(),
            widths: BTreeMap::new(),
            version: 0,
            seen_data_version: 0,
            single_deps: HashMap::new(),
            range_deps: Vec::new(),
        };
        sheet.load()?;
        Ok(sheet)
    }

    /// Read everything from the file, replacing what is in memory.
    pub fn load(&mut self) -> Result<(), LispError> {
        let path = self.path.clone();
        type Row = (u32, u32, String, Option<String>, Option<String>, Option<String>);
        let rows: Vec<Row> = {
            let mut stmt =
                self.conn.prepare("SELECT row, col, input, value, error, fmt FROM cells").map_err(sql(&path))?;
            let mapped = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
                .map_err(sql(&path))?;
            mapped.collect::<Result<_, _>>().map_err(sql(&path))?
        };
        self.cells.clear();
        for (row, col, input, value, error, fmt) in rows {
            let formula = input.strip_prefix('=').map(parse_formula);
            let value = value
                .and_then(|v| serde_json::from_str::<J>(&v).ok())
                .map(|j| from_tagged_json(&j))
                .unwrap_or(Value::Null);
            let fmt = fmt.and_then(|f| match serde_json::from_str::<J>(&f) {
                Ok(J::Object(m)) => Some(m),
                _ => None,
            });
            self.cells.insert(CellRef::new(row, col), CellData { input, value, error, fmt, formula });
        }
        self.widths = {
            let mut stmt = self.conn.prepare("SELECT col, width FROM widths").map_err(sql(&path))?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql(&path))?;
            mapped.collect::<Result<_, _>>().map_err(sql(&path))?
        };
        self.version = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'version'", [], |r| r.get::<_, String>(0))
            .map_err(sql(&path))?
            .parse()
            .unwrap_or(0);
        self.seen_data_version = self.data_version()?;
        self.rebuild_index();
        Ok(())
    }

    fn data_version(&self) -> Result<i64, LispError> {
        self.conn.query_row("PRAGMA data_version", [], |r| r.get(0)).map_err(sql(&self.path))
    }

    /// Pick up a write another connection made — the desktop app and the dev bridge on the same
    /// workspace, or a second engine. Our own commits don't move `data_version`.
    pub fn refresh(&mut self) -> Result<(), LispError> {
        if self.data_version()? != self.seen_data_version {
            self.load()?;
        }
        Ok(())
    }

    pub fn version(&self) -> i64 {
        self.version
    }

    pub fn value_at(&self, at: CellRef) -> Value {
        self.cells.get(&at).map(|c| c.value.clone()).unwrap_or(Value::Null)
    }

    pub fn error_at(&self, at: CellRef) -> Option<&str> {
        self.cells.get(&at).and_then(|c| c.error.as_deref())
    }

    pub fn input_at(&self, at: CellRef) -> &str {
        self.cells.get(&at).map(|c| c.input.as_str()).unwrap_or("")
    }

    pub fn formula_cells(&self) -> Vec<CellRef> {
        let mut out: Vec<CellRef> =
            self.cells.iter().filter(|(_, c)| c.formula.is_some()).map(|(at, _)| *at).collect();
        out.sort();
        out
    }

    /// The first failed cell in `area`, row by row.
    pub fn first_error_in(&self, area: Range) -> Option<(CellRef, &str)> {
        self.cells
            .iter()
            .filter(|(at, c)| c.error.is_some() && area.contains(**at))
            .min_by_key(|(at, _)| **at)
            .map(|(at, c)| (*at, c.error.as_deref().unwrap_or("")))
    }

    // ── the dependency index ──

    fn rebuild_index(&mut self) {
        self.single_deps.clear();
        self.range_deps.clear();
        for (at, cell) in &self.cells {
            if let Some(f) = &cell.formula {
                index(&mut self.single_deps, &mut self.range_deps, *at, &f.refs);
            }
        }
    }

    fn direct_dependents(&self, at: CellRef) -> Vec<CellRef> {
        let mut out: Vec<CellRef> = self.single_deps.get(&at).into_iter().flatten().copied().collect();
        out.extend(self.range_deps.iter().filter(|(r, _)| r.contains(at)).map(|(_, d)| *d));
        out
    }

    /// Replace what a cell holds, in memory. A literal's value is known at once; a formula's waits
    /// for recalculation. Nothing is written — that happens once the recalculation is done.
    pub fn stage_input(&mut self, at: CellRef, input: String) {
        let cell = self.cells.entry(at).or_insert_with(CellData::empty);
        if let Some(old) = cell.formula.take() {
            unindex(&mut self.single_deps, &mut self.range_deps, at, &old.refs);
        }
        match read_input(&input) {
            Input::Empty => {
                cell.value = Value::Null;
                cell.error = None;
            }
            Input::Literal(v) => {
                cell.value = v;
                cell.error = None;
            }
            Input::Formula(f) => {
                index(&mut self.single_deps, &mut self.range_deps, at, &f.refs);
                cell.formula = Some(f);
            }
        }
        cell.input = input;
        if cell.is_blank() {
            self.cells.remove(&at);
        }
    }

    /// The order to recompute after `seeds` changed: the seeds and everything that reads them,
    /// transitively, each formula after the ones it reads. Formulas that read each other in a
    /// circle come out together as one `Cycle`.
    pub fn plan(&self, seeds: &[CellRef]) -> Vec<Step> {
        let mut affected: BTreeSet<CellRef> = seeds.iter().copied().collect();
        let mut todo: Vec<CellRef> = seeds.to_vec();
        while let Some(at) = todo.pop() {
            for d in self.direct_dependents(at) {
                if affected.insert(d) {
                    todo.push(d);
                }
            }
        }
        let nodes: Vec<CellRef> = affected.into_iter().collect();
        let position: HashMap<CellRef, usize> = nodes.iter().enumerate().map(|(i, c)| (*c, i)).collect();
        let succs: Vec<Vec<usize>> = nodes
            .iter()
            .map(|c| self.direct_dependents(*c).iter().filter_map(|d| position.get(d).copied()).collect())
            .collect();

        let mut steps = Vec::new();
        // Tarjan hands back components dependents-first; evaluation wants the reverse.
        for component in strongly_connected(&succs).into_iter().rev() {
            let cyclic = component.len() > 1 || succs[component[0]].contains(&component[0]);
            if cyclic {
                let mut cells: Vec<CellRef> = component.iter().map(|i| nodes[*i]).collect();
                cells.sort();
                steps.push(Step::Cycle(cells));
            } else if self.cells.get(&nodes[component[0]]).is_some_and(|c| c.formula.is_some()) {
                steps.push(Step::Compute(nodes[component[0]]));
            }
        }
        steps
    }

    /// Everything a formula needs in order to run, or why it can't.
    pub fn prepare(&self, at: CellRef) -> Option<Prepared> {
        let formula = self.cells.get(&at)?.formula.as_ref()?;
        let expr = match &formula.expr {
            Ok(e) => e.clone(),
            Err(m) => return Some(Prepared::Fail(m.clone())),
        };
        let mut bindings = Vec::with_capacity(formula.refs.len());
        for (name, r) in &formula.refs {
            let value = match r {
                RefSym::Cell(c) => {
                    if let Some(e) = self.error_at(*c) {
                        return Some(Prepared::Fail(propagated(*c, e)));
                    }
                    self.value_at(*c)
                }
                RefSym::Range(range) => {
                    if range.len() > MAX_RANGE_CELLS {
                        return Some(Prepared::Fail(format!(
                            "{} is {} cells — more than a formula can read",
                            range.a1(),
                            range.len()
                        )));
                    }
                    if let Some((c, e)) = self.first_error_in(*range) {
                        return Some(Prepared::Fail(propagated(c, e)));
                    }
                    Value::List(Rc::new(range.cells().map(|c| self.value_at(c)).collect()))
                }
                RefSym::OtherSheet => {
                    return Some(Prepared::Fail(format!(
                        "{name}: formulas can't reference another sheet yet — use (sheet-get \"Sheet\" \"A1\")"
                    )))
                }
                RefSym::Deleted => {
                    return Some(Prepared::Fail(format!("{DELETED} — this formula read a cell that was deleted")))
                }
            };
            bindings.push((name.clone(), value));
        }
        Some(Prepared::Eval { expr, bindings })
    }

    /// Record a formula's outcome. True when the value or the error actually changed.
    pub fn set_result(&mut self, at: CellRef, result: Result<Value, String>) -> bool {
        let Some(cell) = self.cells.get_mut(&at) else { return false };
        let (value, error) = match result.and_then(storable) {
            Ok(v) => (v, None),
            Err(e) => (Value::Null, Some(e)),
        };
        let changed = cell.value != value || cell.error != error;
        cell.value = value;
        cell.error = error;
        changed
    }

    /// Merge `changes` into the format of every cell in `area` — a key set to null is removed — or
    /// clear the formats outright with `None`. In memory; returns the cells to save.
    pub fn stage_format(&mut self, area: Range, changes: Option<&Map<String, J>>) -> Result<Vec<CellRef>, LispError> {
        if area.len() > MAX_RANGE_CELLS {
            return Err(fail(format!("{} is {} cells — too many to format at once", area.a1(), area.len())));
        }
        let mut touched = Vec::new();
        for at in area.cells() {
            let mut fmt = match changes {
                Some(_) => self.cells.get(&at).and_then(|c| c.fmt.clone()).unwrap_or_default(),
                None => Map::new(),
            };
            for (k, v) in changes.into_iter().flatten() {
                if v.is_null() {
                    fmt.remove(k);
                } else {
                    fmt.insert(k.clone(), v.clone());
                }
            }
            self.put_format(at, fmt);
            touched.push(at);
        }
        Ok(touched)
    }

    /// Set each cell's format exactly — a block of rows from `origin`, `None` for no format. What
    /// undo needs: putting back formats that differed from cell to cell.
    pub fn stage_formats(&mut self, origin: CellRef, rows: Vec<Vec<Option<Map<String, J>>>>) -> Vec<CellRef> {
        let mut touched = Vec::new();
        for (r, row) in rows.into_iter().enumerate() {
            for (c, fmt) in row.into_iter().enumerate() {
                let at = CellRef::new(origin.row + r as u32, origin.col + c as u32);
                self.put_format(at, fmt.unwrap_or_default());
                touched.push(at);
            }
        }
        touched
    }

    fn put_format(&mut self, at: CellRef, fmt: Map<String, J>) {
        let cell = self.cells.entry(at).or_insert_with(CellData::empty);
        cell.fmt = (!fmt.is_empty()).then_some(fmt);
        if cell.is_blank() {
            self.cells.remove(&at);
        }
    }

    /// Write these cells — a cell no longer in memory is deleted — and bump the version, in one
    /// transaction. If the write fails the file is reread, so memory never runs ahead of disk.
    pub fn save(&mut self, cells: &BTreeSet<CellRef>) -> Result<(), LispError> {
        if cells.is_empty() {
            return Ok(()); // nothing moved: the version stays, so an open grid doesn't reload for nothing
        }
        self.write(|tx, all, _| {
            for at in cells {
                match all.get(at) {
                    Some(c) => insert_cell(tx, *at, c)?,
                    None => {
                        tx.execute("DELETE FROM cells WHERE row = ?1 AND col = ?2", params![at.row, at.col])?;
                    }
                }
            }
            Ok(())
        })
    }

    /// A column's width in the host's units, or `None` to go back to the default.
    pub fn set_width(&mut self, col: u32, width: Option<f64>) -> Result<(), LispError> {
        match width {
            Some(w) => self.widths.insert(col, w),
            None => self.widths.remove(&col),
        };
        self.write(|tx, _, _| {
            match width {
                Some(w) => tx.execute("INSERT OR REPLACE INTO widths (col, width) VALUES (?1, ?2)", params![col, w])?,
                None => tx.execute("DELETE FROM widths WHERE col = ?1", params![col])?,
            };
            Ok(())
        })
    }

    /// Insert or delete rows or columns: move the cells, rewrite every formula's references, shift
    /// the column widths, and write the lot in one transaction. Returns the formulas that now read
    /// different cells — a range that grew or shrank, a deleted reference — and so need recomputing.
    pub fn apply_edit(&mut self, edit: Edit) -> Result<Vec<CellRef>, LispError> {
        if matches!(edit, Edit::Insert { .. }) {
            if let Some(stuck) = self.cells.keys().filter(|at| edit.map_cell(**at).is_none()).min() {
                return Err(fail(format!("no room: {} would be pushed off the end of the sheet", stuck.a1())));
            }
        }
        let mut recalc = Vec::new();
        for (at, mut cell) in std::mem::take(&mut self.cells) {
            let Some(to) = edit.map_cell(at) else { continue };
            if let Some(body) = cell.input.strip_prefix('=') {
                let rw = rewrite_formula(body, &edit);
                if rw.changed {
                    cell.formula = Some(parse_formula(&rw.text));
                    cell.input = format!("={}", rw.text);
                }
                if rw.resized {
                    recalc.push(to);
                }
            }
            self.cells.insert(to, cell);
        }
        if edit.axis() == Axis::Cols {
            self.widths = std::mem::take(&mut self.widths)
                .into_iter()
                .filter_map(|(c, w)| edit.map_index(c).map(|c| (c, w)))
                .collect();
        }
        self.rebuild_index();
        self.write(|tx, all, widths| {
            tx.execute("DELETE FROM cells", [])?;
            for (at, c) in all {
                insert_cell(tx, *at, c)?;
            }
            tx.execute("DELETE FROM widths", [])?;
            for (col, w) in widths {
                tx.execute("INSERT INTO widths (col, width) VALUES (?1, ?2)", params![col, w])?;
            }
            Ok(())
        })?;
        recalc.sort();
        Ok(recalc)
    }

    fn write(
        &mut self,
        f: impl FnOnce(&Transaction, &HashMap<CellRef, CellData>, &BTreeMap<u32, f64>) -> rusqlite::Result<()>,
    ) -> Result<(), LispError> {
        let next = self.version + 1;
        let Sheet { conn, cells, widths, .. } = &mut *self;
        let outcome = (|| {
            let tx = conn.transaction()?;
            f(&tx, cells, widths)?;
            tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('version', ?1)", params![next.to_string()])?;
            tx.commit()
        })();
        match outcome {
            Ok(()) => {
                self.version = next;
                Ok(())
            }
            Err(e) => {
                let _ = self.load();
                Err(sql(&self.path)(e))
            }
        }
    }

    // ── what a host sees ──

    /// `(row col input value error fmt)`. A cell that no longer exists comes back blank, so a host
    /// patching its grid clears it.
    pub fn row_value(&self, at: CellRef) -> Value {
        let n = |x: u32| Value::Number(x as f64);
        let items = match self.cells.get(&at) {
            Some(c) => vec![
                n(at.row),
                n(at.col),
                Value::Str(c.input.clone()),
                c.value.clone(),
                c.error.clone().map(Value::Str).unwrap_or(Value::Null),
                c.fmt.clone().map(|m| from_json(&J::Object(m))).unwrap_or(Value::Null),
            ],
            None => vec![n(at.row), n(at.col), Value::Str(String::new()), Value::Null, Value::Null, Value::Null],
        };
        Value::List(Rc::new(items))
    }

    /// `{:path … :version … :cells ((row col input value error fmt) …) :widths ((col width) …)}`
    pub fn payload(&self) -> Value {
        let mut at: Vec<CellRef> = self.cells.keys().copied().collect();
        at.sort();
        let mut d = OrderedDict::default();
        d.insert("path".into(), Value::Str(self.path.display().to_string()));
        d.insert("version".into(), Value::Number(self.version as f64));
        d.insert("cells".into(), Value::List(Rc::new(at.into_iter().map(|c| self.row_value(c)).collect())));
        d.insert(
            "widths".into(),
            Value::List(Rc::new(
                self.widths
                    .iter()
                    .map(|(c, w)| Value::List(Rc::new(vec![Value::Number(*c as f64), Value::Number(*w)])))
                    .collect(),
            )),
        );
        Value::Dict(Rc::new(d))
    }
}

fn insert_cell(tx: &Transaction, at: CellRef, c: &CellData) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO cells (row, col, input, value, error, fmt) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            at.row,
            at.col,
            c.input,
            to_json(&c.value).to_string(),
            c.error,
            c.fmt.as_ref().map(|m| J::Object(m.clone()).to_string()),
        ],
    )?;
    Ok(())
}

fn index(
    single: &mut HashMap<CellRef, HashSet<CellRef>>,
    ranges: &mut Vec<(Range, CellRef)>,
    at: CellRef,
    refs: &[(String, RefSym)],
) {
    for (_, r) in refs {
        match r {
            RefSym::Cell(c) => {
                single.entry(*c).or_default().insert(at);
            }
            RefSym::Range(range) => ranges.push((*range, at)),
            RefSym::OtherSheet | RefSym::Deleted => {}
        }
    }
}

fn unindex(
    single: &mut HashMap<CellRef, HashSet<CellRef>>,
    ranges: &mut Vec<(Range, CellRef)>,
    at: CellRef,
    refs: &[(String, RefSym)],
) {
    for (_, r) in refs {
        if let RefSym::Cell(c) = r {
            if let Some(set) = single.get_mut(c) {
                set.remove(&at);
                if set.is_empty() {
                    single.remove(c);
                }
            }
        }
    }
    if refs.iter().any(|(_, r)| matches!(r, RefSym::Range(_))) {
        ranges.retain(|(_, d)| *d != at);
    }
}

/// Tarjan's algorithm, without recursion — a column of ten thousand running totals is a chain ten
/// thousand deep, which a recursive version would take out of the engine thread's stack. Returns
/// the components with every component listed after all the ones it points to.
fn strongly_connected(succs: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNSEEN: usize = usize::MAX;
    let n = succs.len();
    let (mut index, mut low, mut on_stack) = (vec![UNSEEN; n], vec![0; n], vec![false; n]);
    let (mut stack, mut out, mut next) = (Vec::new(), Vec::new(), 0);
    for root in 0..n {
        if index[root] != UNSEEN {
            continue;
        }
        let mut calls: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&(v, i)) = calls.last() {
            if let Some(&w) = succs[v].get(i) {
                calls.last_mut().unwrap().1 += 1;
                if index[w] == UNSEEN {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    calls.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(component);
            }
        }
    }
    out
}

// ── the open sheets ──────────────────────────────────────────────────

#[derive(Default)]
pub struct Sheets {
    open: HashMap<PathBuf, Sheet>,
    /// Recalculations in progress. A formula may read a sheet while one runs, never change one.
    pub recalculating: u32,
}

impl Sheets {
    /// The sheet at `path`, opened on first use and reread if another connection has written it.
    pub fn sheet(&mut self, path: &Path) -> Result<&mut Sheet, LispError> {
        if !self.open.contains_key(path) {
            let sheet = Sheet::open(path)?;
            self.open.insert(path.to_path_buf(), sheet);
        } else if self.recalculating == 0 {
            // Mid-recalculation the memory holds results not yet saved; a reload would drop them.
            self.open.get_mut(path).unwrap().refresh()?;
        }
        Ok(self.open.get_mut(path).unwrap())
    }

    pub fn create(&mut self, path: &Path) -> Result<&mut Sheet, LispError> {
        let sheet = Sheet::create(path)?;
        self.open.insert(path.to_path_buf(), sheet);
        Ok(self.open.get_mut(path).unwrap())
    }

    /// Close the connection. True if it was open.
    pub fn close(&mut self, path: &Path) -> bool {
        self.open.remove(path).is_some()
    }

    /// Drop what's in memory and reread the file — after an operation failed halfway.
    pub fn reload(&mut self, path: &Path) {
        if let Some(s) = self.open.get_mut(path) {
            let _ = s.load();
        }
    }
}
