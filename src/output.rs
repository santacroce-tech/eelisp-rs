//! Output capture (ANALYSIS §5): `print`/`println` route through an `OutputState` so a host can
//! collect what a script printed. `echo` also mirrors to stdout (on for the CLI, off for a host).

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use crate::env::{self, Env};
use crate::printer::print_value;
use crate::value::*;

#[derive(Default)]
pub struct OutputState {
    pub buffer: String,
    pub echo: bool,
}

impl OutputState {
    fn emit(&mut self, s: &str) {
        if self.echo {
            print!("{}", s);
            std::io::stdout().flush().ok();
        }
        self.buffer.push_str(s);
    }
}

/// Register capturing `print`/`println`, overriding the stdout versions from `builtins`.
pub fn register(env: &Env, out: Rc<RefCell<OutputState>>) {
    let joined = |args: &[Value]| args.iter().map(|a| print_value(a, false)).collect::<Vec<_>>().join(" ");

    {
        let out = out.clone();
        let f = move |args: &[Value], _: &Env| -> Result<Value, LispError> {
            out.borrow_mut().emit(&joined(args));
            Ok(Value::Null)
        };
        env::define(env, "print", Value::Builtin(Rc::new(Builtin { name: "print".into(), arg_mode: ArgMode::Eval, func: Box::new(f) })));
    }
    {
        let out = out.clone();
        let f = move |args: &[Value], _: &Env| -> Result<Value, LispError> {
            let mut line = joined(args);
            line.push('\n');
            out.borrow_mut().emit(&line);
            Ok(Value::Null)
        };
        env::define(env, "println", Value::Builtin(Rc::new(Builtin { name: "println".into(), arg_mode: ArgMode::Eval, func: Box::new(f) })));
    }
}
