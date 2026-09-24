//! The EELisp surface of sheets, and the recalculation loop.
//!
//! A sheet is named the way a person would name it — `"Budget"`, `"Budget.eesheet"`,
//! `"money/Budget"`, or an absolute path. Relative names resolve against `(current-dir)`, the
//! workspace, and `.eesheet` is added when missing. Cells are named `"C3"`, areas `"A1:C9"`.
//!
//! Recalculation never holds the open-sheets registry while a formula runs: a formula may call
//! `(sheet-get …)`, which needs it. It may not *change* a sheet — the write builtins refuse while a
//! recalculation is in progress.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{Map, Value as J};

use crate::editor::EditorHost;
use crate::env::{self, Env};
use crate::eval;
use crate::host::to_json;
use crate::printer::print_value;
use crate::sheet::*;
use crate::sheet_ref::*;
use crate::value::*;

type Registry = Rc<RefCell<Sheets>>;
type Host = Rc<RefCell<EditorHost>>;

fn fail(msg: impl Into<String>) -> LispError {
    LispError::Runtime(msg.into())
}

fn define(env: &Env, name: &str, f: impl Fn(&[Value], &Env) -> Result<Value, LispError> + 'static) {
    env::define(
        env,
        name,
        Value::Builtin(Rc::new(Builtin { name: name.to_string(), arg_mode: ArgMode::Eval, func: Box::new(f) })),
    );
}

/// Define the `sheet-*` builtins over `reg` — the open sheets, which the interpreter also holds, so
/// a host can hand it sheets as bytes and take them back (`Interpreter::import_sheet`).
pub fn register(env: &Env, host: Host, reg: Registry) {

    // Every builtin gets the registry and the host; this keeps each definition to what it does.
    let def = |name: &str, f: fn(&Registry, &Host, &[Value], &Env) -> Result<Value, LispError>| {
        let (reg, host) = (reg.clone(), host.clone());
        define(env, name, move |args, env| f(&reg, &host, args, env));
    };

    def("sheet-new", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-new", "a sheet name")?)?;
        writable(reg, "sheet-new")?;
        reg.borrow_mut().create(&path)?;
        Ok(Value::Str(path.display().to_string()))
    });

    def("sheet-open", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-open", "a sheet name")?)?;
        Ok(reg.borrow_mut().sheet(&path)?.payload())
    });

    def("sheet-close", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-close", "a sheet name")?)?;
        writable(reg, "sheet-close")?;
        reg.borrow_mut().close(&path);
        Ok(Value::Null)
    });

    def("sheet-version", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-version", "a sheet name")?)?;
        Ok(Value::Number(reg.borrow_mut().sheet(&path)?.version() as f64))
    });

    def("sheet-get", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-get", "a sheet name")?)?;
        let area = area_arg(args, 1, "sheet-get")?;
        if area.start != area.end {
            return Err(fail(format!("sheet-get reads one cell — use (sheet-rows … \"{}\") for a range", area.a1())));
        }
        let mut sheets = reg.borrow_mut();
        let sheet = sheets.sheet(&path)?;
        match sheet.error_at(area.start) {
            Some(e) => Err(fail(format!("{}: {}", area.start.a1(), e))),
            None => Ok(sheet.value_at(area.start)),
        }
    });

    def("sheet-rows", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-rows", "a sheet name")?)?;
        let area = area_arg(args, 1, "sheet-rows")?;
        if area.len() > MAX_RANGE_CELLS {
            return Err(fail(format!("{} is {} cells — too many to read at once", area.a1(), area.len())));
        }
        let mut sheets = reg.borrow_mut();
        let sheet = sheets.sheet(&path)?;
        if let Some((at, e)) = sheet.first_error_in(area) {
            return Err(fail(format!("{}: {}", at.a1(), e)));
        }
        let rows = (area.start.row..=area.end.row)
            .map(|r| {
                let row = (area.start.col..=area.end.col).map(|c| sheet.value_at(CellRef::new(r, c))).collect();
                Value::List(Rc::new(row))
            })
            .collect();
        Ok(Value::List(Rc::new(rows)))
    });

    // Typed input: one cell, or a block of rows — what a paste is — in one recalculation.
    def("sheet-set", |reg, host, args, env| {
        let path = resolve(host, str_arg(args, 0, "sheet-set", "a sheet name")?)?;
        let area = area_arg(args, 1, "sheet-set")?;
        if area.start != area.end {
            return Err(fail("sheet-set takes the top-left cell of where to write, like \"C3\""));
        }
        let block = block_of(args.get(2).unwrap_or(&Value::Null), typed);
        write_block(reg, &path, env, area.start, block, "sheet-set")
    });

    // Values: whatever is given stays what it is — text that looks like a number stays text.
    def("sheet-put", |reg, host, args, env| {
        let path = resolve(host, str_arg(args, 0, "sheet-put", "a sheet name")?)?;
        let origin = area_arg(args, 1, "sheet-put")?.start;
        let block = block_of(args.get(2).unwrap_or(&Value::Null), input_arg);
        write_block(reg, &path, env, origin, block, "sheet-put")
    });

    // ── copying cells around ──

    def("sheet-copy", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-copy", "a sheet name")?)?;
        let area = area_arg(args, 1, "sheet-copy")?;
        if area.len() > MAX_RANGE_CELLS {
            return Err(fail(format!("{} is {} cells — too many to copy at once", area.a1(), area.len())));
        }
        let mut sheets = reg.borrow_mut();
        let sheet = sheets.sheet(&path)?;
        let rows = (area.start.row..=area.end.row)
            .map(|r| {
                let row = (area.start.col..=area.end.col)
                    .map(|c| Value::Str(sheet.input_at(CellRef::new(r, c)).to_string()))
                    .collect();
                Value::List(Rc::new(row))
            })
            .collect();
        Ok(Value::List(Rc::new(rows)))
    });

    def("sheet-paste", |reg, host, args, env| {
        let path = resolve(host, str_arg(args, 0, "sheet-paste", "a sheet name")?)?;
        let at = area_arg(args, 1, "sheet-paste")?.start;
        let rows = block_of(args.get(2).unwrap_or(&Value::Null), typed);
        // Where the block came from: given, its formulas move by the distance travelled.
        let shift = match args.get(3) {
            None | Some(Value::Null) => None,
            Some(_) => {
                let from = area_arg(args, 3, "sheet-paste")?.start;
                Some((at.row as i64 - from.row as i64, at.col as i64 - from.col as i64))
            }
        };
        let height = rows.len() as u64;
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0) as u64;
        if at.row as u64 + height > MAX_ROWS as u64 || at.col as u64 + width > MAX_COLS as u64 {
            return Err(fail(format!("sheet-paste: a {height}×{width} block doesn't fit at {}", at.a1())));
        }
        writable(reg, "sheet-paste")?;
        change(reg, &path, env, |sheet| {
            let touched = sheet.stage_paste(at, rows, shift);
            Ok((touched.clone(), touched))
        })
    });

    def("sheet-fill", |reg, host, args, env| {
        let path = resolve(host, str_arg(args, 0, "sheet-fill", "a sheet name")?)?;
        let source = area_arg(args, 1, "sheet-fill")?;
        let target = area_arg(args, 2, "sheet-fill")?;
        writable(reg, "sheet-fill")?;
        change(reg, &path, env, |sheet| {
            let touched = sheet.stage_fill(source, target)?;
            Ok((touched.clone(), touched))
        })
    });

    def("sheet-recalc", |reg, host, args, env| {
        let path = resolve(host, str_arg(args, 0, "sheet-recalc", "a sheet name")?)?;
        writable(reg, "sheet-recalc")?;
        change(reg, &path, env, |sheet| Ok((sheet.formula_cells(), vec![])))
    });

    def("sheet-format", |reg, host, args, env| {
        let path = resolve(host, str_arg(args, 0, "sheet-format", "a sheet name")?)?;
        let area = area_arg(args, 1, "sheet-format")?;
        let changes = match args.get(2) {
            None | Some(Value::Null) => None,
            Some(Value::Dict(d)) => Some(format_changes(d)?),
            // a block of rows, one format (or nil) per cell, set exactly rather than merged
            Some(Value::List(rows)) => {
                let block = format_block(rows)?;
                let height = block.len() as u64;
                let width = block.iter().map(|r| r.len()).max().unwrap_or(0) as u64;
                if area.start.row as u64 + height > MAX_ROWS as u64 || area.start.col as u64 + width > MAX_COLS as u64 {
                    return Err(fail(format!("sheet-format: a {height}×{width} block doesn't fit at {}", area.start.a1())));
                }
                writable(reg, "sheet-format")?;
                return change(reg, &path, env, |sheet| Ok((vec![], sheet.stage_formats(area.start, block))));
            }
            Some(other) => {
                return Err(fail(format!(
                    "sheet-format takes a dict like {{:num \"currency\" :dp 2}}, nil to clear, or rows of formats — got {}",
                    type_name(other)
                )))
            }
        };
        writable(reg, "sheet-format")?;
        change(reg, &path, env, |sheet| Ok((vec![], sheet.stage_format(area, changes.as_ref())?)))
    });

    def("sheet-col-width", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-col-width", "a sheet name")?)?;
        let col = col_arg(args, 1, "sheet-col-width")?;
        let width = size_arg(args, 2, "a width")?;
        writable(reg, "sheet-col-width")?;
        reg.borrow_mut().sheet(&path)?.set_width(col, width)?;
        Ok(Value::Null)
    });

    def("sheet-row-height", |reg, host, args, _| {
        let path = resolve(host, str_arg(args, 0, "sheet-row-height", "a sheet name")?)?;
        let row = row_arg(args, 1, "sheet-row-height")?;
        let height = size_arg(args, 2, "a height")?;
        writable(reg, "sheet-row-height")?;
        reg.borrow_mut().sheet(&path)?.set_height(row, height)?;
        Ok(Value::Null)
    });

    for (name, insert, axis) in [
        ("sheet-insert-rows", true, Axis::Rows),
        ("sheet-delete-rows", false, Axis::Rows),
        ("sheet-insert-cols", true, Axis::Cols),
        ("sheet-delete-cols", false, Axis::Cols),
    ] {
        let (reg, host) = (reg.clone(), host.clone());
        define(env, name, move |args, env| {
            let path = resolve(&host, str_arg(args, 0, name, "a sheet name")?)?;
            let at = match axis {
                Axis::Rows => row_arg(args, 1, name)?,
                Axis::Cols => col_arg(args, 1, name)?,
            };
            let n = match args.get(2) {
                None | Some(Value::Null) => 1,
                Some(Value::Number(n)) if *n >= 1.0 && n.fract() == 0.0 && *n <= MAX_ROWS as f64 => *n as u32,
                Some(other) => return Err(fail(format!("{name}: how many is a whole number from 1 — got {}", print_value(other, true)))),
            };
            let edit = if insert { Edit::Insert { axis, at, n } } else { Edit::Delete { axis, at, n } };
            writable(&reg, name)?;
            change(&reg, &path, env, |sheet| Ok((sheet.apply_edit(edit)?, vec![])))?;
            Ok(reg.borrow_mut().sheet(&path)?.payload())
        });
    }
}

// ── changing a sheet ─────────────────────────────────────────────────

/// Type a block of inputs with its top-left corner at `origin`, then recalculate once.
fn write_block(
    reg: &Registry,
    path: &Path,
    env: &Env,
    origin: CellRef,
    block: Vec<Vec<String>>,
    name: &str,
) -> Result<Value, LispError> {
    let height = block.len() as u64;
    let width = block.iter().map(|r| r.len()).max().unwrap_or(0) as u64;
    if origin.row as u64 + height > MAX_ROWS as u64 || origin.col as u64 + width > MAX_COLS as u64 {
        return Err(fail(format!("{name}: a {height}×{width} block doesn't fit at {}", origin.a1())));
    }
    writable(reg, name)?;
    change(reg, path, env, |sheet| {
        let mut at = Vec::new();
        for (r, row) in block.into_iter().enumerate() {
            for (c, input) in row.into_iter().enumerate() {
                let cell = CellRef::new(origin.row + r as u32, origin.col + c as u32);
                sheet.stage_input(cell, input);
                at.push(cell);
            }
        }
        Ok((at.clone(), at))
    })
}

/// A write builtin called from inside a formula would change a sheet mid-recalculation.
fn writable(reg: &Registry, name: &str) -> Result<(), LispError> {
    if reg.borrow().recalculating > 0 {
        return Err(fail(format!("{name}: a formula can read sheets but not change them")));
    }
    Ok(())
}

/// Stage a change, recalculate what it affects, save, and return the changed cells as rows.
/// `stage` returns (the cells to recalculate from, cells to save whatever happens to them).
/// If anything fails on the way, the sheet is reread from disk so memory never runs ahead of it.
fn change(
    reg: &Registry,
    path: &Path,
    env: &Env,
    stage: impl FnOnce(&mut Sheet) -> Result<(Vec<CellRef>, Vec<CellRef>), LispError>,
) -> Result<Value, LispError> {
    let outcome = (|| {
        let (seeds, save) = stage(reg.borrow_mut().sheet(path)?)?;
        recalculate(reg, path, &seeds, save.into_iter().collect(), env)
    })();
    match outcome {
        Ok(changed) => {
            let rows: Vec<Value> = {
                let mut sheets = reg.borrow_mut();
                let sheet = sheets.sheet(path)?;
                changed.iter().map(|at| sheet.row_value(*at)).collect()
            };
            propagate(reg, path, env)?;
            Ok(Value::List(Rc::new(rows)))
        }
        Err(e) => {
            reg.borrow_mut().reload(path);
            Err(e)
        }
    }
}

/// Counts a recalculation in and — on every way out, errors included — back out.
struct Recalculating<'a>(&'a Registry);

impl<'a> Recalculating<'a> {
    fn enter(reg: &'a Registry) -> Self {
        reg.borrow_mut().recalculating += 1;
        Recalculating(reg)
    }
}

impl Drop for Recalculating<'_> {
    fn drop(&mut self) {
        self.0.borrow_mut().recalculating -= 1;
    }
}

/// Recompute everything downstream of `seeds`, dependencies first, then save the cells whose
/// value or error changed together with `save`. Returns what was saved.
fn recalculate(
    reg: &Registry,
    path: &Path,
    seeds: &[CellRef],
    mut save: BTreeSet<CellRef>,
    env: &Env,
) -> Result<BTreeSet<CellRef>, LispError> {
    {
        // Entered before planning: from here on the sheet is never reread from disk, which would
        // throw away the change staged a moment ago along with the results computed below.
        let _running = Recalculating::enter(reg);
        let steps = reg.borrow_mut().sheet(path)?.plan(seeds);
        let root = root_of(env);
        for step in steps {
            match step {
                Step::Cycle(cells) => {
                    let names: Vec<String> = cells.iter().map(|c| c.a1()).collect();
                    let message = format!("circular reference between {}", names.join(", "));
                    let mut sheets = reg.borrow_mut();
                    let sheet = sheets.sheet(path)?;
                    for at in cells {
                        if sheet.set_result(at, Err(message.clone())) {
                            save.insert(at);
                        }
                    }
                }
                Step::Compute(at) => {
                    let prepared = reg.borrow_mut().sheet(path)?.prepare(at);
                    // No borrow held from here to the result: the formula may read sheets itself.
                    let result = match prepared {
                        None => continue,
                        Some(Prepared::Fail(message)) => Err(message),
                        Some(Prepared::Eval { expr, bindings, externals }) => {
                            // Reading another sheet borrows the registry, which is why it happens
                            // here — between formulas — and not inside `prepare`.
                            match read_externals(reg, path, externals) {
                                Err(message) => Err(message),
                                Ok(others) => {
                                    let scope = env::child(&root);
                                    for (name, value) in bindings.into_iter().chain(others) {
                                        env::define(&scope, &name, value);
                                    }
                                    eval::eval(expr, scope).map_err(|e| e.to_string())
                                }
                            }
                        }
                    };
                    if reg.borrow_mut().sheet(path)?.set_result(at, result) {
                        save.insert(at);
                    }
                }
            }
        }
    }
    reg.borrow_mut().sheet(path)?.save(&save)?;
    Ok(save)
}

/// Read the cells a formula names in other sheets — `Budget!A1`, `Budget!A1:B3` — opening them as
/// needed. A sheet is named beside the one referring to it, so `Budget!A1` in `money/Q3.eesheet`
/// means `money/Budget.eesheet`.
fn read_externals(reg: &Registry, from: &Path, externals: Vec<External>) -> Result<Vec<(String, Value)>, String> {
    let base = from.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut out = Vec::with_capacity(externals.len());
    for (symbol, name, at) in externals {
        let target = resolve_in(&base, &name);
        let mut sheets = reg.borrow_mut();
        let sheet = sheets.sheet(&target).map_err(|e| format!("{symbol}: {e}"))?;
        let value = match at {
            RefSym::Cell(c) => match sheet.error_at(c) {
                Some(e) => return Err(format!("{name}!{} has an error — {e}", c.a1())),
                None => sheet.value_at(c),
            },
            RefSym::Range(range) => {
                if range.len() > MAX_RANGE_CELLS {
                    return Err(format!("{symbol} is {} cells — more than a formula can read", range.len()));
                }
                if let Some((c, e)) = sheet.first_error_in(range) {
                    return Err(format!("{name}!{} has an error — {e}", c.a1()));
                }
                Value::List(Rc::new(range.cells().map(|c| sheet.value_at(c)).collect()))
            }
            _ => Value::Null,
        };
        out.push((symbol, value));
    }
    Ok(out)
}

/// A write can change what another open sheet shows. Every open sheet that reads this one is
/// recomputed — once each, so two sheets reading each other settle instead of bouncing.
fn propagate(reg: &Registry, changed: &Path, env: &Env) -> Result<(), LispError> {
    let readers: Vec<PathBuf> = reg
        .borrow()
        .reading_map()
        .into_iter()
        .filter(|(path, names)| {
            path != changed
                && names.iter().any(|n| resolve_in(path.parent().unwrap_or(Path::new("")), n) == changed)
        })
        .map(|(path, _)| path)
        .collect();
    for reader in readers {
        let seeds = match reg.borrow_mut().loaded(&reader) {
            Some(sheet) => sheet.cells_reading_other_sheets(),
            None => continue,
        };
        if !seeds.is_empty() {
            recalculate(reg, &reader, &seeds, BTreeSet::new(), env)?;
        }
    }
    Ok(())
}

/// Formulas run in the global environment, whatever scope the builtin was called from.
fn root_of(env: &Env) -> Env {
    let mut cur = env.clone();
    loop {
        let parent = cur.borrow().parent.clone();
        match parent {
            Some(p) => cur = p,
            None => return cur,
        }
    }
}

// ── arguments ────────────────────────────────────────────────────────

/// A sheet name → the file. See the module comment.
/// The file a sheet name means — against `(current-dir)`, the workspace. With no workspace and no
/// process folder either (a browser), a name stays relative: `"examples/Budget"` is just that key.
pub fn resolve(host: &Host, name: &str) -> Result<PathBuf, LispError> {
    if name.trim().is_empty() {
        return Err(fail("a sheet needs a name"));
    }
    let dir = host.borrow().current_dir.as_ref().map(|f| f()).unwrap_or_default();
    let base = if dir.is_empty() { std::env::current_dir().unwrap_or_default() } else { PathBuf::from(dir) };
    Ok(resolve_in(&base, name))
}

/// The same, looking in `base` — the folder a sheet naming another one sits in.
fn resolve_in(base: &Path, name: &str) -> PathBuf {
    let named = Path::new(name);
    let file = if named.extension().is_some_and(|e| e == std::ffi::OsStr::new(EXT)) {
        named.to_path_buf()
    } else {
        PathBuf::from(format!("{name}.{EXT}"))
    };
    let absolute = if file.is_absolute() { file } else { base.join(file) };
    // One key per file however it was spelled: `notes/../Budget` and `Budget` are the same sheet.
    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }
    match (absolute.parent().and_then(|p| p.canonicalize().ok()), absolute.file_name()) {
        (Some(dir), Some(file)) => dir.join(file),
        _ => absolute,
    }
}

fn str_arg<'a>(args: &'a [Value], i: usize, name: &str, what: &str) -> Result<&'a str, LispError> {
    match args.get(i) {
        Some(Value::Str(s)) => Ok(s),
        Some(other) => Err(fail(format!("{name}: expected {what} as a string, got {}", print_value(other, true)))),
        None => Err(fail(format!("{name}: missing {what}"))),
    }
}

fn area_arg(args: &[Value], i: usize, name: &str) -> Result<Range, LispError> {
    let s = str_arg(args, i, name, "a cell like \"C3\"")?;
    parse_area(s).ok_or_else(|| fail(format!("{name}: {s:?} isn't a cell or a range — write \"C3\" or \"A1:C9\"")))
}

/// A row as people number them — 1 is the first — made 0-based.
fn row_arg(args: &[Value], i: usize, name: &str) -> Result<u32, LispError> {
    match args.get(i) {
        Some(Value::Number(n)) if *n >= 1.0 && n.fract() == 0.0 && *n <= MAX_ROWS as f64 => Ok(*n as u32 - 1),
        other => Err(fail(format!(
            "{name}: expected a row number from 1, got {}",
            other.map(|v| print_value(v, true)).unwrap_or_else(|| "nothing".into())
        ))),
    }
}

/// A width or a height: a positive number, or nil for the default.
fn size_arg(args: &[Value], i: usize, what: &str) -> Result<Option<f64>, LispError> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.is_finite() && *n > 0.0 => Ok(Some(*n)),
        Some(other) => Err(fail(format!("{what} is a positive number, or nil — got {}", print_value(other, true)))),
    }
}

/// A column by its letters, `"C"`, made 0-based.
fn col_arg(args: &[Value], i: usize, name: &str) -> Result<u32, LispError> {
    let s = str_arg(args, i, name, "a column like \"C\"")?;
    col_index(&s.trim().to_ascii_uppercase()).ok_or_else(|| fail(format!("{name}: {s:?} isn't a column — write \"C\"")))
}

/// `sheet-set`'s argument is what a person would type: a string goes in as it is, so `"=(sum B1:B2)"`
/// is a formula and `"1200"` a number. A number or nil is typed the obvious way.
fn typed(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        other => input_arg(other),
    }
}

/// What to type into a cell to hold `v`. Text that would read as something else — a number, a
/// formula — gets the leading `'` that keeps it text; a bool becomes the formula that returns it.
fn input_arg(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Str(s) => {
            let misreads = s.starts_with('=') || s.starts_with('\'') || looks_numeric(s.trim());
            if misreads {
                format!("'{s}")
            } else {
                s.clone()
            }
        }
        Value::Number(_) => print_value(v, false),
        Value::Bool(b) => format!("={b}"),
        other => input_arg(&Value::Str(print_value(other, false))),
    }
}

/// Data as rows of inputs, each value turned into one by `input`: a result set is a header row and
/// its records; a list of lists is rows; a flat list is one row; anything else is one cell.
fn block_of(v: &Value, input: fn(&Value) -> String) -> Vec<Vec<String>> {
    match v {
        Value::ResultSet(rs) => {
            let mut rows = vec![rs.columns.iter().map(|c| input(&Value::Str(c.clone()))).collect()];
            for rec in &rs.records {
                rows.push(rs.columns.iter().map(|c| input(rec.data.get(c).unwrap_or(&Value::Null))).collect());
            }
            rows
        }
        Value::List(items) if items.iter().all(|i| matches!(i, Value::List(_))) && !items.is_empty() => items
            .iter()
            .map(|row| match row {
                Value::List(cells) => cells.iter().map(input).collect(),
                _ => Vec::new(), // excluded by the guard above
            })
            .collect(),
        Value::List(items) => vec![items.iter().map(input).collect()],
        other => vec![vec![input(other)]],
    }
}

/// Each cell's own format, or none, in rows — `sheet-format`'s block form.
type FormatBlock = Vec<Vec<Option<Map<String, J>>>>;

/// Rows of formats for `sheet-format`'s block form: each a dict, or nil for none.
fn format_block(rows: &[Value]) -> Result<FormatBlock, LispError> {
    rows.iter()
        .map(|row| match row {
            Value::List(cells) => cells
                .iter()
                .map(|c| match c {
                    Value::Null => Ok(None),
                    Value::Dict(d) => {
                        let mut m = format_changes(d)?;
                        m.retain(|_, v| !v.is_null()); // an exact format has no keys to remove
                        Ok(Some(m))
                    }
                    other => Err(fail(format!("sheet-format: each cell's format is a dict or nil — got {}", type_name(other)))),
                })
                .collect(),
            other => Err(fail(format!("sheet-format: a block is a list of rows — got {}", type_name(other)))),
        })
        .collect()
}

/// `{:num "currency" :dp 2 :bold true}` → the JSON object stored with each cell. Nil removes a key.
fn format_changes(d: &OrderedDict) -> Result<Map<String, J>, LispError> {
    let mut out = Map::new();
    for k in &d.keys {
        let v = &d.map[k];
        match v {
            Value::Null | Value::Str(_) | Value::Number(_) | Value::Bool(_) => {
                out.insert(k.clone(), to_json(v));
            }
            other => {
                return Err(fail(format!(
                    "sheet-format: :{k} must be a string, number, bool or nil — got {}",
                    type_name(other)
                )))
            }
        }
    }
    Ok(out)
}
