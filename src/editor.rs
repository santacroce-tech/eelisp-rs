//! Editor RPC (ANALYSIS §5): builtins that let a script read/mutate the host's editor buffer.
//! The host installs callbacks on `EditorHost`; with none installed they are inert (read → nil/"" ,
//! mutate → nil). This is how snippets like `word-count.eelisp` reach the current document.
//!
//! `notes` / `read-note` read the workspace's notes — the files on disk under `(current-dir)`, not
//! the editor's buffers — so one command can look at every note, not just the open one. They are
//! read-only on purpose: writing goes through the host, which knows which files are open in tabs.
//!
//! `dir-list` / `file-info` read the same workspace as a file manager sees it: every entry of one
//! folder, with its size and when it last changed — what a two-panel browser draws.

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::env::{self, Env};
use crate::value::*;

/// Callbacks the UI provides. All optional so the engine runs headless.
#[derive(Default)]
pub struct EditorHost {
    pub buffer_text: Option<Box<dyn Fn() -> String>>,
    pub current_file: Option<Box<dyn Fn() -> String>>,
    /// The workspace root — the folder the host is editing, not the process's cwd. The engine
    /// has no filesystem of its own, so "where am I" is a question only the host can answer.
    pub current_dir: Option<Box<dyn Fn() -> String>>,
    pub cursor_pos: Option<Box<dyn Fn() -> i64>>,
    pub selection: Option<Box<dyn Fn() -> (i64, i64)>>,
    pub set_cursor: Option<Box<dyn Fn(i64)>>,
    pub insert_at: Option<Box<dyn Fn(i64, String)>>,
    pub replace_range: Option<Box<dyn Fn(i64, i64, String)>>,
}

type Host = Rc<RefCell<EditorHost>>;

/// Extensions `notes` lists. Everything else in a workspace (sheets, forms, images) isn't a note.
const NOTE_EXT: &[&str] = &["md", "markdown"];
/// A workspace bigger than this is not a notes folder (a home directory picked by mistake).
const MAX_ENTRIES: usize = 20_000;

fn fail(msg: impl Into<String>) -> LispError {
    LispError::Runtime(msg.into())
}

/// The workspace root: the host's answer, else the process's folder (the bare command line).
fn root(host: &Host) -> PathBuf {
    let dir = host.borrow().current_dir.as_ref().map(|f| f()).unwrap_or_default();
    if dir.is_empty() { std::env::current_dir().unwrap_or_default() } else { PathBuf::from(dir) }
}

/// A workspace-relative path → the file, refusing anything that would leave the workspace
/// (`/etc/passwd`, `../secrets`). `notes/../a.md` is fine: it never steps outside.
fn inside(root: &Path, rel: &str) -> Result<PathBuf, LispError> {
    let mut depth = 0i32;
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(fail(format!("{rel}: outside the workspace")));
                }
            }
            Component::RootDir | Component::Prefix(_) => return Err(fail(format!("{rel}: a path inside the workspace, please"))),
        }
    }
    Ok(root.join(rel))
}

fn is_note(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| NOTE_EXT.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Every note under `dir`, as paths relative to `root` with `/` separators. Hidden entries
/// (`.eeditor`, `.git`) are skipped, symlinked folders aren't followed (no loops), unreadable
/// folders are passed over.
fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, seen: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        *seen += 1;
        if *seen > MAX_ENTRIES {
            return;
        }
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(kind) = e.file_type() else { continue };
        let path = e.path();
        if kind.is_dir() {
            walk(root, &path, out, seen);
        } else if is_note(&path) {
            if let Ok(rel) = path.strip_prefix(root) {
                let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                out.push(parts.join("/"));
            }
        }
    }
}

/// A path relative to the workspace, `/`-separated whatever the platform.
fn rel_path(root: &Path, p: &Path) -> String {
    let rel = p.strip_prefix(root).unwrap_or(p);
    rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// One entry as `dir-list` and `file-info` give it: name, path, whether it is a folder, size in
/// bytes (0 for a folder), when it last changed (epoch seconds, as `now` counts them) and its
/// extension, lower-case, without the dot ("" for a folder or a file with none).
fn entry(root: &Path, p: &Path, meta: &std::fs::Metadata) -> Value {
    let is_dir = meta.is_dir();
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = if is_dir { String::new() } else { p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default() };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as f64)
        .unwrap_or(0.0);
    let mut d = OrderedDict::default();
    d.insert("name".into(), Value::Str(name));
    d.insert("path".into(), Value::Str(rel_path(root, p)));
    d.insert("dir".into(), Value::Bool(is_dir));
    d.insert("size".into(), Value::Number(if is_dir { 0.0 } else { meta.len() as f64 }));
    d.insert("modified".into(), Value::Number(modified));
    d.insert("ext".into(), Value::Str(ext));
    Value::Dict(Rc::new(d))
}

/// `(f "path")` → the path, or the workspace root for no argument / nil.
fn folder_arg(fname: &str, args: &[Value]) -> Result<String, LispError> {
    match args.first() {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(v) => Err(fail(format!("{fname}: expected a folder name, got {}", type_name(v)))),
    }
}

fn b(env: &Env, name: &str, f: impl Fn(&[Value], &Env) -> Result<Value, LispError> + 'static) {
    env::define(env, name, Value::Builtin(Rc::new(Builtin { name: name.to_string(), arg_mode: ArgMode::Eval, func: Box::new(f) })));
}

pub fn register(env: &Env, host: Host) {
    {
        let h = host.clone();
        b(env, "notes", move |args, _| {
            let root = root(&h);
            let under = match args.first() {
                None | Some(Value::Null) => root.clone(),
                Some(Value::Str(s)) => inside(&root, s)?,
                Some(v) => return Err(fail(format!("notes: expected a folder name, got {}", type_name(v)))),
            };
            let mut out = Vec::new();
            walk(&root, &under, &mut out, &mut 0);
            out.sort();
            Ok(Value::List(Rc::new(out.into_iter().map(Value::Str).collect())))
        });
    }
    {
        let h = host.clone();
        b(env, "read-note", move |args, _| {
            let rel = match args.first() {
                Some(Value::Str(s)) if !s.trim().is_empty() => s.clone(),
                _ => return Err(fail("read-note: expected a path, like \"inbox.md\"")),
            };
            let file = inside(&root(&h), &rel)?;
            match std::fs::read(&file) {
                Ok(bytes) => String::from_utf8(bytes).map(Value::Str).map_err(|_| fail(format!("{rel}: not a text file"))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
                Err(e) => Err(fail(format!("{rel}: {e}"))),
            }
        });
    }
    {
        let h = host.clone();
        b(env, "dir-list", move |args, _| {
            let rel = folder_arg("dir-list", args)?;
            let hidden = match (args.get(1), args.get(2)) {
                (None, _) => false,
                (Some(Value::Keyword(k)), v) if k == "hidden" => v.is_some_and(is_truthy),
                (Some(v), _) => return Err(fail(format!("dir-list: expected :hidden true after the folder, got {}", type_name(v)))),
            };
            let root = root(&h);
            let dir = inside(&root, &rel)?;
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
                Err(e) if dir.is_file() => return Err(fail(format!("{rel}: not a folder ({e})"))),
                Err(e) => return Err(fail(format!("{rel}: {e}"))),
            };
            let mut out: Vec<(bool, String, Value)> = Vec::new();
            for e in entries.flatten().take(MAX_ENTRIES) {
                let name = e.file_name().to_string_lossy().into_owned();
                if !hidden && name.starts_with('.') {
                    continue;
                }
                let p = e.path();
                // follow a symlink to what it points at; a broken one is still listed, as itself
                let Ok(meta) = std::fs::metadata(&p).or_else(|_| e.metadata()) else { continue };
                out.push((meta.is_dir(), name.to_lowercase(), entry(&root, &p, &meta)));
            }
            // folders first, then files, each by name whatever its case
            out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            Ok(Value::List(Rc::new(out.into_iter().map(|(_, _, v)| v).collect())))
        });
    }
    {
        let h = host.clone();
        b(env, "file-info", move |args, _| {
            let rel = match args.first() {
                Some(Value::Str(s)) => s.clone(),
                _ => return Err(fail("file-info: expected a path, like \"inbox.md\"")),
            };
            let root = root(&h);
            let p = inside(&root, &rel)?;
            match std::fs::metadata(&p) {
                Ok(meta) => Ok(entry(&root, &p, &meta)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
                Err(e) => Err(fail(format!("{rel}: {e}"))),
            }
        });
    }
    {
        let h = host.clone();
        b(env, "buffer-text", move |_, _| {
            Ok(Value::Str(h.borrow().buffer_text.as_ref().map(|f| f()).unwrap_or_default()))
        });
    }
    {
        let h = host.clone();
        b(env, "current-file", move |_, _| {
            Ok(Value::Str(h.borrow().current_file.as_ref().map(|f| f()).unwrap_or_default()))
        });
    }
    {
        let h = host.clone();
        b(env, "current-dir", move |_, _| {
            Ok(Value::Str(h.borrow().current_dir.as_ref().map(|f| f()).unwrap_or_default()))
        });
    }
    {
        let h = host.clone();
        b(env, "cursor-pos", move |_, _| {
            Ok(Value::Number(h.borrow().cursor_pos.as_ref().map(|f| f()).unwrap_or(0) as f64))
        });
    }
    {
        let h = host.clone();
        b(env, "selection", move |_, _| {
            let (s, e) = h.borrow().selection.as_ref().map(|f| f()).unwrap_or((0, 0));
            Ok(Value::List(Rc::new(vec![Value::Number(s as f64), Value::Number(e as f64)])))
        });
    }
    {
        let h = host.clone();
        b(env, "set-cursor", move |args, _| {
            if let (Some(f), Some(Value::Number(p))) = (h.borrow().set_cursor.as_ref(), args.first()) {
                f(*p as i64);
            }
            Ok(Value::Null)
        });
    }
    {
        let h = host.clone();
        b(env, "insert-at", move |args, _| {
            if let (Some(f), Some(Value::Number(p)), Some(Value::Str(t))) =
                (h.borrow().insert_at.as_ref(), args.first(), args.get(1))
            {
                f(*p as i64, t.clone());
            }
            Ok(Value::Null)
        });
    }
    {
        let h = host.clone();
        b(env, "replace-range", move |args, _| {
            if let (Some(f), Some(Value::Number(s)), Some(Value::Number(e)), Some(Value::Str(t))) =
                (h.borrow().replace_range.as_ref(), args.first(), args.get(1), args.get(2))
            {
                f(*s as i64, *e as i64, t.clone());
            }
            Ok(Value::Null)
        });
    }
}
