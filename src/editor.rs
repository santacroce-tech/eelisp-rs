//! Editor RPC (ANALYSIS §5): builtins that let a script read/mutate the host's editor buffer.
//! The host installs callbacks on `EditorHost`; with none installed they are inert (read → nil/"" ,
//! mutate → nil). This is how snippets like `word-count.eelisp` reach the current document.

use std::cell::RefCell;
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

fn b(env: &Env, name: &str, f: impl Fn(&[Value], &Env) -> Result<Value, LispError> + 'static) {
    env::define(env, name, Value::Builtin(Rc::new(Builtin { name: name.to_string(), arg_mode: ArgMode::Eval, func: Box::new(f) })));
}

pub fn register(env: &Env, host: Host) {
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
