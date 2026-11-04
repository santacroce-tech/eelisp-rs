//! The bytecode virtual machine (roadmap step 1, docs/plans/roadmap.md).
//!
//! The compiler — to be written in EELisp — turns forms into **protos**: a list of instructions
//! for a stack machine plus the constants they use. A proto closed over an environment is an
//! ordinary `Function` whose `code` is set; everything that calls functions (the tree-walker,
//! `apply_value`, map/filter/reduce, `Interpreter::call`) runs it here. Compiled and interpreted
//! functions call each other freely.
//!
//! Variables live in the same environments the tree-walker uses (`env::Scope` chains), so
//! compiled code means exactly what interpreted code means: `def` defines in the current scope,
//! builtins get the caller's environment, closures share scopes. Lexical addressing can come
//! later as an optimization the compiler opts into.
//!
//! Tail calls replace the frame, so tail recursion runs in constant space. Non-tail calls push a
//! frame on a heap-allocated stack, not the Rust stack, and stop at `MAX_FRAMES`.
//!
//! The textual form a compiler emits, and `vm-assemble` reads:
//!
//! ```text
//! (proto :name "f" :params (n . rest) :source (body forms…)
//!        :code ((load n) (const 0) (load =) …))
//! ```
//!
//! Instructions (stack effect in brackets):
//!
//! | form | effect |
//! |---|---|
//! | `(const v)` | push `v` |
//! | `(load s)` / `(def s)` / `(set s)` | push the value of `s` / bind or assign `s` to the top (kept) |
//! | `(pop)` | drop the top |
//! | `(jump L)` `(jump-if-false L)` | jump; the conditional one pops its test |
//! | `(jump-if-false-or-pop L)` `(jump-if-true-or-pop L)` | `and`/`or`: keep the top and jump, or pop it |
//! | `(push-scope)` `(pop-scope)` | enter / leave a child environment |
//! | `(closure (proto …))` | push a function closing over the current environment |
//! | `(call n)` `(tail-call n)` | call the function below `n` arguments |
//! | `(return)` | return the top |
//! | `(dict k…)` | pop one value per key, push a dict |
//! | `(loop s…)` `(recur n)` `(end-loop)` | `loop`: mark a restart point that rebinds `s…`; `recur` pops `n` values and restarts |
//! | `(try L)` `(end-try)` | on an error until `end-try`, unwind and jump to `L` with the error dict pushed |
//! | `(label L)` | a jump target (no instruction) |

use std::collections::HashMap;
use std::rc::Rc;

use crate::env::{self, Env};
use crate::eval::{apply_value, error_dict};
use crate::value::*;

/// Deeper than this many non-tail calls is an error, not an exhausted machine.
pub const MAX_FRAMES: usize = 100_000;

#[derive(Clone, Debug)]
pub enum Op {
    Const(u32),
    Load(Sym),
    Def(Sym),
    Set(Sym),
    Pop,
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfFalseOrPop(u32),
    JumpIfTrueOrPop(u32),
    PushScope,
    PopScope,
    Closure(u32),
    Call(u32),
    TailCall(u32),
    Return,
    Dict(u32),
    Loop(u32),
    Recur(u32),
    EndLoop,
    Try(u32),
    EndTry,
}

/// A compiled function body, before it closes over an environment.
pub struct Proto {
    pub name: Option<String>,
    pub params: Vec<Sym>,
    pub rest: Option<Sym>,
    /// The forms it was compiled from — what `(source f)` and the printer show.
    pub source: Vec<Value>,
    pub code: Vec<Op>,
    pub consts: Vec<Value>,
    pub protos: Vec<Rc<Proto>>,
    /// Key lists for `Dict`, name lists for `Loop`.
    pub dict_keys: Vec<Vec<Symbol>>,
    pub loop_names: Vec<Vec<Sym>>,
}

/// The function a proto makes when it closes over `env`.
pub fn close(proto: &Rc<Proto>, env: &Env) -> Value {
    Value::Function(Rc::new(Function {
        name: proto.name.clone(),
        params: proto.params.clone(),
        rest: proto.rest.clone(),
        body: proto.source.clone(),
        closure: env.clone(),
        code: Some(proto.clone()),
    }))
}

/// Call a compiled function with evaluated arguments. `f.code` must be set.
pub fn call(f: &Function, args: Vec<Value>) -> Result<Value, LispError> {
    let proto = f.code.clone().expect("vm::call on a function with no code");
    let env = bind(&proto, &f.closure, args);
    let mut vm = Vm { stack: Vec::new(), frames: Vec::new() };
    vm.frames.push(Frame::new(proto, env, 0));
    vm.run()
}

/// The parameters' scope — missing arguments are nil, extras go to the rest parameter or are
/// dropped, exactly as the tree-walker binds them.
fn bind(proto: &Proto, closure: &Env, args: Vec<Value>) -> Env {
    let mut locals = Vec::with_capacity(proto.params.len() + proto.rest.is_some() as usize);
    let mut args = args.into_iter();
    for p in &proto.params {
        locals.push((p.clone(), args.next().unwrap_or(Value::Null)));
    }
    if let Some(rest) = &proto.rest {
        locals.push((rest.clone(), Value::List(Rc::new(args.collect()))));
    }
    env::child_from(closure, locals)
}

struct Mark {
    start: usize,
    env: Env,
    sp: usize,
    handlers: usize,
    names: u32,
}

struct Handler {
    catch: usize,
    env: Env,
    sp: usize,
    marks: usize,
}

struct Frame {
    proto: Rc<Proto>,
    pc: usize,
    env: Env,
    base: usize,
    marks: Vec<Mark>,
    handlers: Vec<Handler>,
}

impl Frame {
    fn new(proto: Rc<Proto>, env: Env, base: usize) -> Frame {
        Frame { proto, pc: 0, env, base, marks: Vec::new(), handlers: Vec::new() }
    }
}

struct Vm {
    stack: Vec<Value>,
    frames: Vec<Frame>,
}

fn pop(stack: &mut Vec<Value>) -> Value {
    stack.pop().expect("vm stack underflow (malformed bytecode)")
}

fn top(stack: &[Value]) -> Value {
    stack.last().cloned().expect("vm stack underflow (malformed bytecode)")
}

impl Vm {
    fn run(&mut self) -> Result<Value, LispError> {
        loop {
            match self.step() {
                Ok(v) => return Ok(v),
                Err(e) => self.unwind(e)?,
            }
        }
    }

    /// Run instructions until the entry frame returns or one fails.
    fn step(&mut self) -> Result<Value, LispError> {
        loop {
            let fr = self.frames.last_mut().expect("vm has no frame");
            let op = match fr.proto.code.get(fr.pc) {
                Some(op) => op.clone(),
                None => Op::Return,
            };
            fr.pc += 1;
            match op {
                Op::Const(i) => {
                    let v = fr.proto.consts[i as usize].clone();
                    self.stack.push(v);
                }
                Op::Load(s) => {
                    let v = env::get_sym(&fr.env, &s)?;
                    self.stack.push(v);
                }
                Op::Def(s) => env::define_sym(&fr.env, &s, top(&self.stack)),
                Op::Set(s) => env::set(&fr.env, &s, top(&self.stack))?,
                Op::Pop => {
                    pop(&mut self.stack);
                }
                Op::Jump(t) => fr.pc = t as usize,
                Op::JumpIfFalse(t) => {
                    if !is_truthy(&pop(&mut self.stack)) {
                        fr.pc = t as usize;
                    }
                }
                Op::JumpIfFalseOrPop(t) => {
                    if is_truthy(&top(&self.stack)) {
                        pop(&mut self.stack);
                    } else {
                        fr.pc = t as usize;
                    }
                }
                Op::JumpIfTrueOrPop(t) => {
                    if is_truthy(&top(&self.stack)) {
                        fr.pc = t as usize;
                    } else {
                        pop(&mut self.stack);
                    }
                }
                Op::PushScope => fr.env = env::child(&fr.env),
                Op::PopScope => {
                    let parent = fr.env.borrow().parent.clone();
                    fr.env = parent.expect("pop-scope at the root environment");
                }
                Op::Closure(i) => {
                    let v = close(&fr.proto.protos[i as usize], &fr.env);
                    self.stack.push(v);
                }
                Op::Call(n) => {
                    self.call(n as usize, false)?;
                }
                Op::TailCall(n) => {
                    // A frame inside a `try` or a `loop` keeps its frame: the handler or the
                    // restart point must survive the call.
                    let tail = fr.handlers.is_empty() && fr.marks.is_empty();
                    if let Some(v) = self.call(n as usize, tail)? {
                        return Ok(v);
                    }
                }
                Op::Return => {
                    let v = pop(&mut self.stack);
                    if let Some(done) = self.ret(v) {
                        return Ok(done);
                    }
                }
                Op::Dict(i) => {
                    let keys = &fr.proto.dict_keys[i as usize];
                    let vals = self.stack.split_off(self.stack.len() - keys.len());
                    let mut d = OrderedDict::default();
                    for (k, v) in keys.iter().zip(vals) {
                        d.insert(k.clone(), v);
                    }
                    self.stack.push(Value::Dict(Rc::new(d)));
                }
                Op::Loop(names) => {
                    let mark = Mark {
                        start: fr.pc,
                        env: fr.env.clone(),
                        sp: self.stack.len(),
                        handlers: fr.handlers.len(),
                        names,
                    };
                    fr.marks.push(mark);
                }
                Op::Recur(n) => {
                    let vals = self.stack.split_off(self.stack.len() - n as usize);
                    return Err(LispError::Recur(vals));
                }
                Op::EndLoop => {
                    fr.marks.pop();
                }
                Op::Try(catch) => {
                    let h = Handler { catch: catch as usize, env: fr.env.clone(), sp: self.stack.len(), marks: fr.marks.len() };
                    fr.handlers.push(h);
                }
                Op::EndTry => {
                    fr.handlers.pop();
                }
            }
        }
    }

    /// Call the function below `n` arguments. A compiled function gets a frame (replacing this
    /// one when `tail`); anything else is applied here. Returns the value when a tail call
    /// finished the entry frame.
    fn call(&mut self, n: usize, tail: bool) -> Result<Option<Value>, LispError> {
        let args = self.stack.split_off(self.stack.len() - n);
        let f = pop(&mut self.stack);
        match &f {
            Value::Function(func) if func.code.is_some() => {
                let proto = func.code.clone().unwrap();
                let env = bind(&proto, &func.closure, args);
                if tail {
                    let fr = self.frames.last_mut().unwrap();
                    self.stack.truncate(fr.base);
                    *fr = Frame::new(proto, env, fr.base);
                } else {
                    if self.frames.len() >= MAX_FRAMES {
                        return Err(LispError::Runtime(format!("stack overflow: more than {MAX_FRAMES} nested calls")));
                    }
                    self.frames.push(Frame::new(proto, env, self.stack.len()));
                }
                Ok(None)
            }
            Value::Function(_) | Value::Builtin(_) => {
                let env = self.frames.last().unwrap().env.clone();
                let v = apply_value(&f, &args, &env)?;
                if tail {
                    Ok(self.ret(v))
                } else {
                    self.stack.push(v);
                    Ok(None)
                }
            }
            Value::Macro(m) => Err(LispError::Runtime(format!(
                "macro {} reached at run time: compiled code must expand macros when it is compiled",
                m.name.as_deref().unwrap_or("(anonymous)")
            ))),
            other => Err(LispError::TypeMismatch { expected: "function".into(), got: type_name(other) }),
        }
    }

    /// Leave the current frame with `v`. `Some` when that was the entry frame.
    fn ret(&mut self, v: Value) -> Option<Value> {
        let fr = self.frames.pop().unwrap();
        self.stack.truncate(fr.base);
        if self.frames.is_empty() {
            return Some(v);
        }
        self.stack.push(v);
        None
    }

    /// Route an error to where it lands: a `recur` to the innermost loop, anything else to the
    /// innermost `try`. Frames with neither are left. `Err` means no frame of this run takes it.
    fn unwind(&mut self, e: LispError) -> Result<(), LispError> {
        let recur = matches!(e, LispError::Recur(_));
        while let Some(fr) = self.frames.last_mut() {
            if recur && !fr.marks.is_empty() {
                let LispError::Recur(vals) = e else { unreachable!() };
                let m = fr.marks.last().unwrap();
                let names = &fr.proto.loop_names[m.names as usize];
                for (name, v) in names.iter().zip(vals) {
                    env::define_sym(&m.env, name, v);
                }
                fr.env = m.env.clone();
                fr.pc = m.start;
                fr.handlers.truncate(m.handlers);
                self.stack.truncate(m.sp);
                return Ok(());
            }
            if !recur && !fr.handlers.is_empty() {
                let h = fr.handlers.pop().unwrap();
                fr.env = h.env;
                fr.pc = h.catch;
                fr.marks.truncate(h.marks);
                self.stack.truncate(h.sp);
                self.stack.push(error_dict(&e));
                return Ok(());
            }
            let fr = self.frames.pop().unwrap();
            self.stack.truncate(fr.base);
        }
        Err(e)
    }
}

// ───────────────────────────── assembler ─────────────────────────────

fn bad(msg: impl Into<String>) -> LispError {
    LispError::InvalidSyntax(format!("vm-assemble: {}", msg.into()))
}

fn sym_of(v: &Value, what: &str) -> Result<Sym, LispError> {
    match v {
        Value::Symbol(s) => Ok(s.clone()),
        other => Err(bad(format!("{what} must be a symbol, got {}", type_name(other)))),
    }
}

fn count_of(v: Option<&Value>, what: &str) -> Result<u32, LispError> {
    match v {
        Some(Value::Number(n)) if *n >= 0.0 && n.fract() == 0.0 => Ok(*n as u32),
        _ => Err(bad(format!("{what} expects a whole number"))),
    }
}

/// Read a `(proto :name … :params … :source … :code …)` form into a `Proto`.
pub fn assemble(form: &Value) -> Result<Rc<Proto>, LispError> {
    let items = match form {
        Value::List(l) if matches!(l.first(), Some(Value::Symbol(s)) if &**s == "proto") => l,
        _ => return Err(bad("expected a (proto …) form")),
    };
    let mut name = None;
    let mut params = Vec::new();
    let mut rest = None;
    let mut source = Vec::new();
    let mut code_forms: &[Value] = &[];
    let mut i = 1;
    while i < items.len() {
        let key = match &items[i] {
            Value::Keyword(k) => k.as_str(),
            other => return Err(bad(format!("expected a keyword, got {}", type_name(other)))),
        };
        let val = items.get(i + 1).ok_or_else(|| bad(format!(":{key} has no value")))?;
        match (key, val) {
            ("name", Value::Str(s)) => name = Some(s.clone()),
            ("name", Value::Symbol(s)) => name = Some(s.to_string()),
            ("name", Value::Null) => name = None,
            ("params", Value::List(spec)) => {
                let mut j = 0;
                while j < spec.len() {
                    let s = sym_of(&spec[j], "a parameter")?;
                    if &*s == "." {
                        rest = Some(sym_of(spec.get(j + 1).unwrap_or(&Value::Null), "the rest parameter")?);
                        break;
                    }
                    params.push(s);
                    j += 1;
                }
            }
            ("source", Value::List(forms)) => source = forms.to_vec(),
            ("code", Value::List(forms)) => code_forms = forms,
            (k, _) => return Err(bad(format!("unknown or malformed field :{k}"))),
        }
        i += 2;
    }

    // Pass 1: where each label lands.
    let mut labels: HashMap<Sym, u32> = HashMap::new();
    let mut at = 0u32;
    for f in code_forms {
        let parts = instr(f)?;
        if &*parts.0 == "label" {
            let l = sym_of(parts.1.first().unwrap_or(&Value::Null), "a label")?;
            if labels.insert(l.clone(), at).is_some() {
                return Err(bad(format!("label {l} defined twice")));
            }
        } else {
            at += 1;
        }
    }
    let target = |v: Option<&Value>| -> Result<u32, LispError> {
        let l = sym_of(v.unwrap_or(&Value::Null), "a jump target")?;
        labels.get(&l).copied().ok_or_else(|| bad(format!("no label {l}")))
    };

    // Pass 2: the instructions.
    let mut p = Proto {
        name,
        params,
        rest,
        source,
        code: Vec::new(),
        consts: Vec::new(),
        protos: Vec::new(),
        dict_keys: Vec::new(),
        loop_names: Vec::new(),
    };
    for f in code_forms {
        let (op, args) = instr(f)?;
        let arity = |n: usize| -> Result<(), LispError> {
            if args.len() == n {
                Ok(())
            } else {
                Err(bad(format!("({op} …) takes {n} argument(s), got {}", args.len())))
            }
        };
        let o = match &*op {
            "label" => continue,
            "const" => {
                arity(1)?;
                p.consts.push(args[0].clone());
                Op::Const(p.consts.len() as u32 - 1)
            }
            "load" => {
                arity(1)?;
                Op::Load(sym_of(&args[0], "load's name")?)
            }
            "def" => {
                arity(1)?;
                Op::Def(sym_of(&args[0], "def's name")?)
            }
            "set" => {
                arity(1)?;
                Op::Set(sym_of(&args[0], "set's name")?)
            }
            "pop" => Op::Pop,
            "jump" => Op::Jump(target(args.first())?),
            "jump-if-false" => Op::JumpIfFalse(target(args.first())?),
            "jump-if-false-or-pop" => Op::JumpIfFalseOrPop(target(args.first())?),
            "jump-if-true-or-pop" => Op::JumpIfTrueOrPop(target(args.first())?),
            "push-scope" => Op::PushScope,
            "pop-scope" => Op::PopScope,
            "closure" => {
                arity(1)?;
                p.protos.push(assemble(&args[0])?);
                Op::Closure(p.protos.len() as u32 - 1)
            }
            "call" => Op::Call(count_of(args.first(), "call")?),
            "tail-call" => Op::TailCall(count_of(args.first(), "tail-call")?),
            "return" => Op::Return,
            "dict" => {
                let keys = args
                    .iter()
                    .map(|k| match k {
                        Value::Keyword(s) | Value::Str(s) => Ok(s.clone()),
                        Value::Symbol(s) => Ok(s.to_string()),
                        other => Err(bad(format!("a dict key must be a keyword, got {}", type_name(other)))),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                p.dict_keys.push(keys);
                Op::Dict(p.dict_keys.len() as u32 - 1)
            }
            "loop" => {
                let names = args.iter().map(|n| sym_of(n, "a loop name")).collect::<Result<Vec<_>, _>>()?;
                p.loop_names.push(names);
                Op::Loop(p.loop_names.len() as u32 - 1)
            }
            "recur" => Op::Recur(count_of(args.first(), "recur")?),
            "end-loop" => Op::EndLoop,
            "try" => Op::Try(target(args.first())?),
            "end-try" => Op::EndTry,
            other => return Err(bad(format!("unknown instruction {other}"))),
        };
        p.code.push(o);
    }
    Ok(Rc::new(p))
}

fn instr(f: &Value) -> Result<(Sym, Vec<Value>), LispError> {
    match f {
        Value::List(l) if !l.is_empty() => Ok((sym_of(&l[0], "an instruction")?, l[1..].to_vec())),
        other => Err(bad(format!("an instruction is a list, got {}", type_name(other)))),
    }
}

pub fn register(env: &Env) {
    env::define(
        env,
        "vm-assemble",
        Value::Builtin(Rc::new(Builtin {
            name: "vm-assemble".into(),
            arg_mode: ArgMode::Eval,
            func: Box::new(|args: &[Value], env: &Env| match args {
                [form] => Ok(close(&assemble(form)?, env)),
                _ => Err(LispError::Arity { func: "vm-assemble".into(), expected: "1".into(), got: args.len() }),
            }),
        })),
    );
}
