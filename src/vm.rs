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
//!        :slots 3 :captures ((local 0) (cap 1))
//!        :code ((load-local 0 n) (const 0) (load =) …))
//! ```
//!
//! With `:slots n` a proto is in **slot mode**: its frame has `n` local slots, the parameters in
//! the first ones, and a call makes no scope. A variable an inner closure uses is moved into a
//! shared cell when the closure is made (`:captures` says which: the maker's slot, or one of its
//! own captures). Without `:slots`, parameters are bound by name in a new scope and `load`/`def`
//! work on scopes — the mode hand-written bytecode uses.
//!
//! Instructions (stack effect in brackets):
//!
//! | form | effect |
//! |---|---|
//! | `(const v)` | push `v` |
//! | `(load s)` / `(def s)` / `(set s)` | push the value of `s` / bind or assign `s` to the top (kept) |
//! | `(pop)` | drop the top |
//! | `(def-pop s)` | bind `s` to the top and drop it (the assembler makes it from `(def s) (pop)`) |
//! | `(jump L)` `(jump-if-false L)` | jump; the conditional one pops its test |
//! | `(jump-if-false-or-pop L)` `(jump-if-true-or-pop L)` | `and`/`or`: keep the top and jump, or pop it |
//! | `(push-scope)` `(pop-scope)` | enter / leave a child environment |
//! | `(closure (proto …))` | push a function closing over the current environment |
//! | `(call n)` `(tail-call n)` | call the function below `n` arguments |
//! | `(return)` | return the top |
//! | `(dict k…)` | pop one value per key, push a dict |
//! | `(loop s…)` `(recur n)` `(end-loop)` | `loop`: mark a restart point that rebinds `s…`; `recur` pops `n` values and restarts |
//! | `(try L)` `(end-try)` | on an error until `end-try`, unwind and jump to `L` with the error dict pushed |
//! | `(make-list item splice …)` | pop one value per shape and push a list; a `splice` value is a list whose elements go in (quasiquote) |
//! | `(next-item L)` | with a list and an index on the stack: push the next element and advance, or pop both and jump to `L` when done (for-each) |
//! | `(macro (proto …))` | push a macro whose expander is that proto, closing over the current environment |
//! | `(load-local i name)` `(set-local i name)` | read / assign slot `i`; if the slot isn't bound yet, the global `name` is meant |
//! | `(def-local i)` `(init-local i)` `(clear-local i)` | bind slot `i` to the top (kept / popped as a fresh binding) / unbind it |
//! | `(load-cap i name)` `(set-cap i name)` | read / assign the closure's `i`-th captured variable |
//! | `(loop-slots i…)` | `loop` over slot variables: `recur` rebinds those slots |
//! | `(interp form)` | push what the interpreter makes of `form` in the current scope — the compiler's fallback for anything it doesn't compile yet |
//! | `(label L)` | a jump target (no instruction); `L` is a symbol or a number |

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::env::{self, Env};
use crate::eval::{apply_value, error_dict};
use crate::value::*;

/// Deeper than this many non-tail calls is an error, not an exhausted machine.
pub const MAX_FRAMES: usize = 100_000;

/// One instruction. Every operand is an index (into the proto's constants, symbols, protos…) or
/// a jump target, so an instruction is a copy, not a clone, when the machine fetches it.
#[derive(Clone, Copy, Debug)]
pub enum Op {
    Const(u32),
    Load(u32),
    Def(u32),
    /// `def` then `pop`, fused by the assembler: the value moves into the binding.
    DefPop(u32),
    Set(u32),
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
    Interp(u32),
    MakeList(u32),
    NextItem(u32),
    Macro(u32),
    // Slot-mode locals (a proto with `:slots`). The second operand of a load or set is the
    // variable's name, for when its slot isn't bound yet: then the global of that name is meant.
    LoadLocal(u32, u32),
    DefLocal(u32),
    DefLocalPop(u32),
    InitLocal(u32),
    SetLocal(u32, u32),
    ClearLocal(u32),
    LoadCap(u32, u32),
    SetCap(u32, u32),
    LoopSlots(u32),
}

/// A variable a closure shares with the function that made it (or with that function's own
/// closure). `None` until it's defined.
pub type Cell = Rc<RefCell<Option<Value>>>;

/// One local variable of a slot-mode frame.
#[derive(Clone)]
enum Slot {
    /// Not defined yet in this run of its scope: reading it means the global of the same name.
    Unbound,
    Val(Value),
    /// Captured by a closure: the frame and its closures share the cell.
    Cell(Cell),
}

/// Where a closure's captured variable comes from, in the function creating it.
#[derive(Clone, Copy, Debug)]
pub enum CapSource {
    /// That function's own slot.
    Local(u32),
    /// One of that function's own captures.
    Cap(u32),
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
    /// The names `Load`, `Def` and `Set` refer to.
    pub syms: Vec<Sym>,
    pub protos: Vec<Rc<Proto>>,
    /// Key lists for `Dict`, name lists for `Loop`.
    pub dict_keys: Vec<Vec<Symbol>>,
    pub loop_names: Vec<Vec<Sym>>,
    /// For `MakeList`: per element, whether it is spliced.
    pub list_shapes: Vec<Vec<bool>>,
    /// `Some(n)`: a slot-mode proto with `n` local slots — parameters first, in order, then the
    /// rest parameter. `None`: the parameters are bound by name in a new scope.
    pub nslots: Option<u32>,
    /// What a closure made from this proto captures, in order: `LoadCap(i)` reads the i-th.
    pub captures: Vec<CapSource>,
    /// Slot lists for `LoopSlots`.
    pub loop_slots: Vec<Vec<u32>>,
    /// Per name in `syms`: where `Load` last found it as a global (see `GlobalCache`).
    globals: Vec<std::cell::Cell<GlobalCache>>,
}

/// Where a global's value was found: in which global scope, at which layout of the global scopes
/// (`env::layout`), and at what address. While the frame's scope is that same global scope and
/// the layout hasn't changed, the value is still at that address, so `Load` reads it there
/// instead of hashing the name.
#[derive(Clone, Copy)]
struct GlobalCache {
    scope: *const RefCell<crate::env::Scope>,
    layout: u64,
    value: *const Value,
}

impl GlobalCache {
    const EMPTY: GlobalCache = GlobalCache { scope: std::ptr::null(), layout: u64::MAX, value: std::ptr::null() };
}

/// `Load`: a global, from the cache when it's still valid, otherwise by name (and cached).
#[inline(always)]
fn load_global(env: &Env, proto: &Proto, s: u32) -> Result<Value, LispError> {
    let cell = &proto.globals[s as usize];
    let c = cell.get();
    if c.scope == Rc::as_ptr(env) && c.layout == env::layout() {
        // SAFETY: same scope, same layout: the map hasn't grown since, so the entry hasn't moved
        // (`env::layout`). The borrow check refuses while anyone is changing the scope.
        if unsafe { env.try_borrow_unguarded() }.is_ok() {
            return Ok(copy(unsafe { &*c.value }));
        }
    }
    let sym = &proto.syms[s as usize];
    let v = env::get_sym(env, sym)?;
    if let Some(value) = env::global_slot(env, sym) {
        cell.set(GlobalCache { scope: Rc::as_ptr(env), layout: env::layout(), value });
    }
    Ok(v)
}

/// The function a proto makes when it closes over `env`.
pub fn close(proto: &Rc<Proto>, env: &Env) -> Value {
    close_with(proto, env, None)
}

fn close_with(proto: &Rc<Proto>, env: &Env, upvals: Option<Rc<[Cell]>>) -> Value {
    Value::Function(Rc::new(Function {
        name: proto.name.clone(),
        params: proto.params.clone(),
        rest: proto.rest.clone(),
        body: proto.source.clone(),
        closure: env.clone(),
        code: Some(proto.clone()),
        upvals,
    }))
}

/// Run a proto's code directly in `env` — no parameter scope — the way a top-level form runs:
/// its `def`s land in `env`.
pub fn run_in(proto: Rc<Proto>, env: Env) -> Result<Value, LispError> {
    run_frame(proto, env)
}

thread_local! {
    /// Machines that finished a run, kept for the next one: `map` calling a compiled function
    /// once per element would otherwise allocate a stack and a frame list each time. A run that
    /// starts while another is going (a builtin calling back into compiled code) takes another.
    static SPARE: std::cell::RefCell<Vec<Vm>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn run_frame(proto: Rc<Proto>, env: Env) -> Result<Value, LispError> {
    run_entry(proto, &env, None, std::iter::empty(), true)
}

/// Run `proto` as the entry frame of a machine: called with `args` under `closure`, or — `top` —
/// as a top-level form whose definitions land in `closure` itself.
fn run_entry(
    proto: Rc<Proto>,
    closure: &Env,
    upvals: Option<Rc<[Cell]>>,
    args: impl Iterator<Item = Value>,
    top: bool,
) -> Result<Value, LispError> {
    let mut vm = SPARE.with(|s| s.borrow_mut().pop()).unwrap_or_default();
    let frame = if top && proto.nslots.is_none() {
        Frame::new(proto, closure.clone(), 0, 0, None)
    } else if top {
        enter(&mut vm.slots, proto, closure, upvals, args, 0, false)
    } else {
        enter(&mut vm.slots, proto, closure, upvals, args, 0, true)
    };
    vm.frames.push(frame);
    let r = vm.run();
    vm.stack.clear();
    vm.frames.clear();
    vm.marks.clear();
    vm.handlers.clear();
    vm.slots.clear();
    SPARE.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() < 16 {
            s.push(vm);
        }
    });
    r
}

/// The global scope at the root of `env`'s chain.
pub fn root_of(env: &Env) -> Env {
    let mut cur = env.clone();
    loop {
        let parent = cur.borrow().parent.clone();
        match parent {
            Some(p) => cur = p,
            None => return cur,
        }
    }
}

/// The macro a proto makes when it closes over `env`.
fn close_macro(proto: &Rc<Proto>, env: &Env, upvals: Option<Rc<[Cell]>>) -> Value {
    Value::Macro(Rc::new(Macro {
        name: proto.name.clone(),
        params: proto.params.clone(),
        rest: proto.rest.clone(),
        body: proto.source.clone(),
        closure: env.clone(),
        code: Some(proto.clone()),
        upvals,
    }))
}

/// Call a compiled function with evaluated arguments. `f.code` must be set.
pub fn call(f: &Function, args: Vec<Value>) -> Result<Value, LispError> {
    call_proto(f.code.clone().expect("vm::call on a function with no code"), &f.closure, f.upvals.clone(), args)
}

/// Call a compiled function with arguments borrowed from a slice — how builtins such as `map`
/// call back into compiled code, without collecting each element's arguments into a vector.
pub fn call_with(f: &Function, args: &[Value]) -> Result<Value, LispError> {
    let proto = f.code.clone().expect("vm::call_with on a function with no code");
    run_entry(proto, &f.closure, f.upvals.clone(), args.iter().map(copy), false)
}

/// Run a proto as a call: its parameters bound to `args` in a scope under `closure`. How a
/// compiled macro expands, too — it is called with its arguments unevaluated.
pub fn call_proto(
    proto: Rc<Proto>,
    closure: &Env,
    upvals: Option<Rc<[Cell]>>,
    args: Vec<Value>,
) -> Result<Value, LispError> {
    run_entry(proto, closure, upvals, args.into_iter(), false)
}

/// A frame for calling `proto` with `args`. Slot mode: its slots go on top of `slots`, the
/// parameters in the first ones, and its environment is the closure's own — a call makes no
/// scope. Named mode (`bind`): a new scope holds the parameters. `as_call` false runs a slot-mode
/// top-level form: slots, but no parameters.
#[inline(never)]
fn enter(
    slots: &mut Vec<Slot>,
    proto: Rc<Proto>,
    closure: &Env,
    upvals: Option<Rc<[Cell]>>,
    args: impl IntoIterator<Item = Value>,
    base: usize,
    as_call: bool,
) -> Frame {
    let Some(n) = proto.nslots else {
        let env = bind(&proto, closure, args);
        return Frame::new(proto, env, base, slots.len(), upvals);
    };
    let sb = slots.len();
    slots.resize(sb + n as usize, Slot::Unbound);
    if as_call {
        let mut args = args.into_iter();
        let np = proto.params.len();
        for k in 0..np {
            slots[sb + k] = Slot::Val(args.next().unwrap_or(Value::Null));
        }
        if proto.rest.is_some() {
            slots[sb + np] = Slot::Val(Value::List(Rc::new(args.collect())));
        }
    }
    Frame::new(proto, closure.clone(), base, sb, upvals)
}

/// `def` in a slot: through the cell if a closure shares it, so the closure sees it too.
fn def_slot(slot: &mut Slot, v: Value) {
    match slot {
        Slot::Cell(c) => *c.borrow_mut() = Some(v),
        s => *s = Slot::Val(v),
    }
}

/// The cells a closure made from `proto` captures, from the frame creating it. A captured slot
/// becomes a cell, shared from then on by the frame and the closure.
fn capture(slots: &mut [Slot], sb: usize, upvals: &Option<Rc<[Cell]>>, proto: &Proto) -> Option<Rc<[Cell]>> {
    if proto.captures.is_empty() {
        return None;
    }
    let cells: Vec<Cell> = proto
        .captures
        .iter()
        .map(|c| match *c {
            CapSource::Local(j) => {
                let slot = &mut slots[sb + j as usize];
                match slot {
                    Slot::Cell(c) => c.clone(),
                    Slot::Val(v) => {
                        let c = Rc::new(RefCell::new(Some(v.clone())));
                        *slot = Slot::Cell(c.clone());
                        c
                    }
                    Slot::Unbound => {
                        let c = Rc::new(RefCell::new(None));
                        *slot = Slot::Cell(c.clone());
                        c
                    }
                }
            }
            CapSource::Cap(j) => upvals.as_ref().expect("a capture of a capture needs captures")[j as usize].clone(),
        })
        .collect();
    Some(Rc::from(cells))
}

/// The parameters' scope — missing arguments are nil, extras go to the rest parameter or are
/// dropped, exactly as the tree-walker binds them.
fn bind(proto: &Proto, closure: &Env, args: impl IntoIterator<Item = Value>) -> Env {
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
    /// What `recur` rebinds: names (`Loop`) or slots (`LoopSlots`), by list index.
    rebind: Rebind,
}

#[derive(Clone, Copy)]
enum Rebind {
    Names(u32),
    Slots(u32),
}

struct Handler {
    catch: usize,
    env: Env,
    sp: usize,
    marks: usize,
}

/// What a call did to the frames: nothing (a builtin ran), moved control to another frame (a
/// compiled function was entered, or a tail call returned), or finished the run.
enum Next {
    Stay,
    Switch,
    Done(Value),
}

/// `Next` after leaving a frame with `Vm::ret`.
fn left(r: Option<Value>) -> Next {
    match r {
        Some(v) => Next::Done(v),
        None => Next::Switch,
    }
}

struct Frame {
    proto: Rc<Proto>,
    pc: usize,
    env: Env,
    base: usize,
    /// Where this frame's slots start in `Vm::slots`.
    slot_base: usize,
    upvals: Option<Rc<[Cell]>>,
    /// Where this frame's loop marks and `try` handlers start in `Vm::marks` and `Vm::handlers`.
    /// Most frames have neither, so they live in the VM, not in a vector per frame.
    mark0: u32,
    handler0: u32,
}

impl Frame {
    fn new(proto: Rc<Proto>, env: Env, base: usize, slot_base: usize, upvals: Option<Rc<[Cell]>>) -> Frame {
        Frame { proto, pc: 0, env, base, slot_base, upvals, mark0: 0, handler0: 0 }
    }
}

#[derive(Default)]
struct Vm {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    /// Every frame's local slots, end to end.
    slots: Vec<Slot>,
    /// Every frame's active loops and `try` handlers, end to end.
    marks: Vec<Mark>,
    handlers: Vec<Handler>,
}

/// `recur`'s new values into a loop's variables.
fn rebind(m: &Mark, proto: &Proto, slots: &mut [Slot], sb: usize, vals: impl Iterator<Item = Value>) {
    match m.rebind {
        Rebind::Names(i) => {
            for (name, v) in proto.loop_names[i as usize].iter().zip(vals) {
                env::define_sym(&m.env, name, v);
            }
        }
        Rebind::Slots(i) => {
            for (&slot, v) in proto.loop_slots[i as usize].iter().zip(vals) {
                def_slot(&mut slots[sb + slot as usize], v);
            }
        }
    }
}

/// A copy of `v`. Numbers, booleans and nil — most values in a hot loop — are copied directly
/// rather than through `Value::clone`, which is a large match the compiler doesn't inline.
#[inline(always)]
fn copy(v: &Value) -> Value {
    match v {
        Value::Number(n) => Value::Number(*n),
        Value::Bool(b) => Value::Bool(*b),
        Value::Null => Value::Null,
        other => other.clone(),
    }
}

fn pop(stack: &mut Vec<Value>) -> Value {
    stack.pop().expect("vm stack underflow (malformed bytecode)")
}

fn top(stack: &[Value]) -> Value {
    copy(stack.last().expect("vm stack underflow (malformed bytecode)"))
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
        // The current frame's code and position are held here, not re-read from the frame for
        // every instruction. They change only when control moves to another frame: a call that
        // enters a compiled function, a tail call, a return. Then they are reloaded.
        'frame: loop {
            let (proto, mut pc, sb) = {
                let fr = self.frames.last().expect("vm has no frame");
                (fr.proto.clone(), fr.pc, fr.slot_base)
            };
            let code: &[Op] = &proto.code;
            loop {
            let op = code.get(pc).copied().unwrap_or(Op::Return);
            pc += 1;
            match op {
                Op::Const(i) => {
                    let v = copy(&proto.consts[i as usize]);
                    self.stack.push(v);
                }
                Op::Load(s) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let v = load_global(&fr.env, &proto, s)?;
                    self.stack.push(v);
                }
                Op::Def(s) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    env::define_sym(&fr.env, &proto.syms[s as usize], top(&self.stack));
                }
                Op::DefPop(s) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    env::define_sym(&fr.env, &proto.syms[s as usize], pop(&mut self.stack));
                }
                Op::Set(s) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    env::set(&fr.env, &proto.syms[s as usize], top(&self.stack))?;
                }
                Op::Pop => {
                    pop(&mut self.stack);
                }
                Op::Jump(t) => pc = t as usize,
                Op::JumpIfFalse(t) => {
                    if !is_truthy(&pop(&mut self.stack)) {
                        pc = t as usize;
                    }
                }
                Op::JumpIfFalseOrPop(t) => {
                    if is_truthy(&top(&self.stack)) {
                        pop(&mut self.stack);
                    } else {
                        pc = t as usize;
                    }
                }
                Op::JumpIfTrueOrPop(t) => {
                    if is_truthy(&top(&self.stack)) {
                        pc = t as usize;
                    } else {
                        pop(&mut self.stack);
                    }
                }
                Op::PushScope => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    fr.env = env::child(&fr.env);
                }
                Op::PopScope => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let parent = fr.env.borrow().parent.clone();
                    fr.env = parent.expect("pop-scope at the root environment");
                }
                Op::Closure(i) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let proto = &proto.protos[i as usize];
                    let up = capture(&mut self.slots, fr.slot_base, &fr.upvals, proto);
                    self.stack.push(close_with(proto, &fr.env, up));
                }
                Op::Call(n) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    fr.pc = pc;
                    match self.call(n as usize, false)? {
                        Next::Stay => {}
                        Next::Switch => continue 'frame,
                        Next::Done(v) => return Ok(v),
                    }
                }
                Op::TailCall(n) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    // A frame inside a `try` or a `loop` keeps its frame: the handler or the
                    // restart point must survive the call.
                    let tail = self.handlers.len() == fr.handler0 as usize && self.marks.len() == fr.mark0 as usize;
                    fr.pc = pc;
                    match self.call(n as usize, tail)? {
                        Next::Stay => {}
                        Next::Switch => continue 'frame,
                        Next::Done(v) => return Ok(v),
                    }
                }
                Op::Return => {
                    let v = pop(&mut self.stack);
                    if let Some(done) = self.ret(v) {
                        return Ok(done);
                    }
                    continue 'frame;
                }
                Op::Dict(i) => {
                    let keys = &proto.dict_keys[i as usize];
                    let vals = self.stack.split_off(self.stack.len() - keys.len());
                    let mut d = OrderedDict::default();
                    for (k, v) in keys.iter().zip(vals) {
                        d.insert(k.clone(), v);
                    }
                    self.stack.push(Value::Dict(Rc::new(d)));
                }
                Op::Loop(i) | Op::LoopSlots(i) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let mark = Mark {
                        start: pc,
                        env: fr.env.clone(),
                        sp: self.stack.len(),
                        handlers: self.handlers.len(),
                        rebind: if matches!(op, Op::Loop(_)) { Rebind::Names(i) } else { Rebind::Slots(i) },
                    };
                    self.marks.push(mark);
                }
                Op::Recur(n) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let base = self.stack.len() - n as usize;
                    // The loop is this frame's own (the usual case): rebind and jump, here.
                    if self.marks.len() > fr.mark0 as usize {
                        let m = self.marks.last().unwrap();
                        rebind(m, &proto, &mut self.slots, fr.slot_base, self.stack.drain(base..));
                        let (start, sp, handlers, menv) = (m.start, m.sp, m.handlers, m.env.clone());
                        fr.env = menv;
                        pc = start;
                        self.handlers.truncate(handlers);
                        self.stack.truncate(sp);
                        continue;
                    }
                    // Otherwise it travels, as the interpreter's does, to a caller's loop.
                    let vals = self.stack.split_off(base);
                    return Err(LispError::Recur(vals));
                }
                Op::EndLoop => {
                    self.marks.pop();
                }
                Op::Try(catch) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let h = Handler { catch: catch as usize, env: fr.env.clone(), sp: self.stack.len(), marks: self.marks.len() };
                    self.handlers.push(h);
                }
                Op::EndTry => {
                    self.handlers.pop();
                }
                Op::Interp(i) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let form = proto.consts[i as usize].clone();
                    let v = crate::eval::eval(form, fr.env.clone())?;
                    self.stack.push(v);
                }
                Op::MakeList(i) => {
                    let shape = &proto.list_shapes[i as usize];
                    let vals = self.stack.split_off(self.stack.len() - shape.len());
                    let mut out = Vec::with_capacity(vals.len());
                    for (splice, v) in shape.iter().zip(vals) {
                        match (splice, v) {
                            (false, v) => out.push(v),
                            (true, Value::List(l)) => out.extend(l.iter().cloned()),
                            (true, other) => {
                                return Err(LispError::TypeMismatch { expected: "list".into(), got: type_name(&other) })
                            }
                        }
                    }
                    self.stack.push(Value::List(Rc::new(out)));
                }
                Op::NextItem(end) => {
                    let n = self.stack.len();
                    let i = match &self.stack[n - 1] {
                        Value::Number(i) => *i as usize,
                        _ => unreachable!("next-item needs an index on the stack"),
                    };
                    let next = match &self.stack[n - 2] {
                        Value::List(l) => l.get(i).cloned(),
                        _ => None, // for-each over something that isn't a list does nothing
                    };
                    match next {
                        Some(v) => {
                            self.stack[n - 1] = Value::Number((i + 1) as f64);
                            self.stack.push(v);
                        }
                        None => {
                            self.stack.truncate(n - 2);
                            pc = end as usize;
                        }
                    }
                }
                Op::Macro(i) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let proto = &proto.protos[i as usize];
                    let up = capture(&mut self.slots, fr.slot_base, &fr.upvals, proto);
                    self.stack.push(close_macro(proto, &fr.env, up));
                }
                Op::LoadLocal(i, s) => {
                    let v = match &self.slots[sb + i as usize] {
                        Slot::Val(v) => copy(v),
                        Slot::Cell(c) => match &*c.borrow() {
                            Some(v) => v.clone(),
                            None => env::get_sym(&self.frames.last().unwrap().env, &proto.syms[s as usize])?,
                        },
                        Slot::Unbound => env::get_sym(&self.frames.last().unwrap().env, &proto.syms[s as usize])?,
                    };
                    self.stack.push(v);
                }
                Op::DefLocal(i) => def_slot(&mut self.slots[sb + i as usize], top(&self.stack)),
                Op::DefLocalPop(i) => def_slot(&mut self.slots[sb + i as usize], pop(&mut self.stack)),
                Op::InitLocal(i) => self.slots[sb + i as usize] = Slot::Val(pop(&mut self.stack)),
                Op::ClearLocal(i) => self.slots[sb + i as usize] = Slot::Unbound,
                Op::SetLocal(i, s) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let v = top(&self.stack);
                    match &mut self.slots[sb + i as usize] {
                        Slot::Val(old) => *old = v,
                        Slot::Cell(c) if c.borrow().is_some() => *c.borrow_mut() = Some(v),
                        // not defined in this scope yet: set! looks further out, to the global
                        _ => env::set(&fr.env, &proto.syms[s as usize], v)?,
                    }
                }
                Op::LoadCap(i, s) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let up = fr.upvals.as_ref().expect("load-cap in a closure with no captures");
                    let v = match &*up[i as usize].borrow() {
                        Some(v) => copy(v),
                        None => env::get_sym(&fr.env, &proto.syms[s as usize])?,
                    };
                    self.stack.push(v);
                }
                Op::SetCap(i, s) => {
                    let fr = self.frames.last_mut().expect("vm has no frame");
                    let v = top(&self.stack);
                    let up = fr.upvals.as_ref().expect("set-cap in a closure with no captures");
                    let cell = &up[i as usize];
                    if cell.borrow().is_some() {
                        *cell.borrow_mut() = Some(v);
                    } else {
                        env::set(&fr.env, &proto.syms[s as usize], v)?;
                    }
                }
            }
        }
        }
    }

    /// Call the function below `n` arguments. A compiled function gets a frame (replacing this
    /// one when `tail`); anything else is applied here. Returns the value when a tail call
    /// finished the entry frame.
    fn call(&mut self, n: usize, tail: bool) -> Result<Next, LispError> {
        // The arguments are the top `n` values, the function just below them. They are passed
        // from where they lie: no vector is made for them.
        let base = self.stack.len() - n;
        // A builtin is called where it lies, with its arguments where they lie.
        if let Value::Builtin(b) = &self.stack[base - 1] {
            // `(+ a b)`, `(< a b)`… on two numbers: computed here, no call through the builtin
            if let (Some(op), 2) = (b.fast, n) {
                if let (Value::Number(x), Value::Number(y)) = (&self.stack[base], &self.stack[base + 1]) {
                    let v = op.apply(*x, *y);
                    // two numbers need no dropping; only the builtin's reference is released
                    self.stack.truncate(base - 1);
                    return Ok(if tail { left(self.ret(v)) } else {
                        self.stack.push(v);
                        Next::Stay
                    });
                }
            }
            let env = &self.frames.last().unwrap().env;
            let v = (b.func)(&self.stack[base..], env)?;
            self.stack.truncate(base - 1);
            return Ok(if tail { left(self.ret(v)) } else {
                self.stack.push(v);
                Next::Stay
            });
        }
        let f = self.stack[base - 1].clone();
        match &f {
            Value::Function(func) if func.code.is_some() => {
                let proto = func.code.clone().unwrap();
                if tail {
                    // the new frame takes this one's place: its slots, then its stack
                    let (fbase, fsb) = {
                        let fr = self.frames.last().unwrap();
                        (fr.base, fr.slot_base)
                    };
                    // the arguments move straight from the stack into the new frame's slots
                    self.slots.truncate(fsb);
                    let frame =
                        enter(&mut self.slots, proto, &func.closure, func.upvals.clone(), self.stack.drain(base..), fbase, true);
                    self.stack.truncate(fbase);
                    // a tail call happens only with no marks or handlers active in this frame, so
                    // the new frame's start where this one's did
                    let mut frame = frame;
                    frame.mark0 = self.marks.len() as u32;
                    frame.handler0 = self.handlers.len() as u32;
                    *self.frames.last_mut().unwrap() = frame;
                } else {
                    if self.frames.len() >= MAX_FRAMES {
                        return Err(LispError::Runtime(format!("stack overflow: more than {MAX_FRAMES} nested calls")));
                    }
                    let mut frame =
                        enter(&mut self.slots, proto, &func.closure, func.upvals.clone(), self.stack.drain(base..), base - 1, true);
                    frame.mark0 = self.marks.len() as u32;
                    frame.handler0 = self.handlers.len() as u32;
                    self.stack.pop();
                    self.frames.push(frame);
                }
                Ok(Next::Switch)
            }
            Value::Function(_) | Value::Builtin(_) => {
                let env = self.frames.last().unwrap().env.clone();
                let v = apply_value(&f, &self.stack[base..], &env)?;
                self.stack.truncate(base - 1);
                if tail {
                    Ok(left(self.ret(v)))
                } else {
                    self.stack.push(v);
                    Ok(Next::Stay)
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
        self.slots.truncate(fr.slot_base);
        // normally already gone (EndLoop, EndTry); this keeps the stacks right whatever happened
        if self.marks.len() > fr.mark0 as usize {
            self.marks.truncate(fr.mark0 as usize);
        }
        if self.handlers.len() > fr.handler0 as usize {
            self.handlers.truncate(fr.handler0 as usize);
        }
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
            if recur && self.marks.len() > fr.mark0 as usize {
                let LispError::Recur(vals) = e else { unreachable!() };
                let m = self.marks.last().unwrap();
                rebind(m, &fr.proto, &mut self.slots, fr.slot_base, vals.into_iter());
                fr.env = m.env.clone();
                fr.pc = m.start;
                self.handlers.truncate(m.handlers);
                self.stack.truncate(m.sp);
                return Ok(());
            }
            if !recur && self.handlers.len() > fr.handler0 as usize {
                let h = self.handlers.pop().unwrap();
                fr.env = h.env;
                fr.pc = h.catch;
                self.marks.truncate(h.marks);
                self.stack.truncate(h.sp);
                self.stack.push(error_dict(&e));
                return Ok(());
            }
            let fr = self.frames.pop().unwrap();
            self.stack.truncate(fr.base);
            self.slots.truncate(fr.slot_base);
            self.marks.truncate(fr.mark0 as usize);
            self.handlers.truncate(fr.handler0 as usize);
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
    let mut nslots = None;
    let mut captures = Vec::new();
    let mut i = 1;
    while i < items.len() {
        let key = match &items[i] {
            Value::Keyword(k) => k.as_str(),
            other => return Err(bad(format!("expected a keyword, got {}", type_name(other)))),
        };
        let val = items.get(i + 1).ok_or_else(|| bad(format!(":{key} has no value")))?;
        match (key, val) {
            ("name", Value::Str(s)) => name = Some(s.to_string()),
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
            ("slots", Value::Number(_)) => nslots = Some(count_of(Some(val), ":slots")?),
            ("captures", Value::List(caps)) => {
                for c in caps.iter() {
                    let (kind, args) = instr(c)?;
                    let i = count_of(args.first(), "a capture")?;
                    captures.push(match &*kind {
                        "local" => CapSource::Local(i),
                        "cap" => CapSource::Cap(i),
                        other => return Err(bad(format!("a capture is (local i) or (cap i), not ({other} …)"))),
                    });
                }
            }
            ("captures", Value::Null) => {}
            ("code", Value::List(forms)) => code_forms = forms,
            (k, _) => return Err(bad(format!("unknown or malformed field :{k}"))),
        }
        i += 2;
    }

    // Pass 0: fuse `(def s) (pop)` into `(def-pop s)`, before labels are counted.
    let fused = fuse_def_pop(code_forms);
    let code_forms: &[Value] = &fused;

    // Pass 1: where each label lands.
    let mut labels: HashMap<String, u32> = HashMap::new();
    let mut at = 0u32;
    for f in code_forms {
        let parts = instr(f)?;
        if &*parts.0 == "label" {
            let l = label_of(parts.1.first())?;
            if labels.insert(l.clone(), at).is_some() {
                return Err(bad(format!("label {l} defined twice")));
            }
        } else {
            at += 1;
        }
    }
    let target = |v: Option<&Value>| -> Result<u32, LispError> {
        let l = label_of(v)?;
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
        syms: Vec::new(),
        protos: Vec::new(),
        dict_keys: Vec::new(),
        loop_names: Vec::new(),
        list_shapes: Vec::new(),
        nslots,
        captures,
        loop_slots: Vec::new(),
        globals: Vec::new(),
    };
    if let Some(n) = nslots {
        let need = p.params.len() + p.rest.is_some() as usize;
        if (n as usize) < need {
            return Err(bad(format!(":slots {n} has no room for {need} parameter(s)")));
        }
    }
    let ncaps = p.captures.len() as u32;
    let slot = |v: Option<&Value>, nslots: Option<u32>| -> Result<u32, LispError> {
        let Some(n) = nslots else { return Err(bad("a slot instruction needs a proto with :slots")) };
        let i = count_of(v, "a slot")?;
        if i < n {
            Ok(i)
        } else {
            Err(bad(format!("slot {i} is out of range (:slots {n})")))
        }
    };
    let cap = |v: Option<&Value>| -> Result<u32, LispError> {
        let i = count_of(v, "a capture")?;
        if i < ncaps {
            Ok(i)
        } else {
            Err(bad(format!("capture {i} is out of range ({ncaps} captured)")))
        }
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
            "load" | "def" | "set" => {
                arity(1)?;
                let s = sym_of(&args[0], &format!("{op}'s name"))?;
                let i = intern(&mut p.syms, s);
                match &*op {
                    "load" => Op::Load(i),
                    "def" => Op::Def(i),
                    _ => Op::Set(i),
                }
            }
            "pop" => Op::Pop,
            "load-local" | "set-local" | "load-cap" | "set-cap" => {
                arity(2)?;
                let name = intern(&mut p.syms, sym_of(&args[1], "the variable's name")?);
                match &*op {
                    "load-local" => Op::LoadLocal(slot(args.first(), p.nslots)?, name),
                    "set-local" => Op::SetLocal(slot(args.first(), p.nslots)?, name),
                    "load-cap" => Op::LoadCap(cap(args.first())?, name),
                    _ => Op::SetCap(cap(args.first())?, name),
                }
            }
            "def-local" => Op::DefLocal(slot(args.first(), p.nslots)?),
            "def-local-pop" => Op::DefLocalPop(slot(args.first(), p.nslots)?),
            "init-local" => Op::InitLocal(slot(args.first(), p.nslots)?),
            "clear-local" => Op::ClearLocal(slot(args.first(), p.nslots)?),
            "loop-slots" => {
                let slots = args.iter().map(|a| slot(Some(a), p.nslots)).collect::<Result<Vec<_>, _>>()?;
                p.loop_slots.push(slots);
                Op::LoopSlots(p.loop_slots.len() as u32 - 1)
            }
            "def-pop" => {
                arity(1)?;
                let s = sym_of(&args[0], "def-pop's name")?;
                Op::DefPop(intern(&mut p.syms, s))
            }
            "jump" => Op::Jump(target(args.first())?),
            "jump-if-false" => Op::JumpIfFalse(target(args.first())?),
            "jump-if-false-or-pop" => Op::JumpIfFalseOrPop(target(args.first())?),
            "jump-if-true-or-pop" => Op::JumpIfTrueOrPop(target(args.first())?),
            "push-scope" => Op::PushScope,
            "pop-scope" => Op::PopScope,
            "closure" => {
                arity(1)?;
                let child = assemble(&args[0])?;
                check_captures(&child, p.nslots, ncaps)?;
                p.protos.push(child);
                Op::Closure(p.protos.len() as u32 - 1)
            }
            "call" => Op::Call(count_of(args.first(), "call")?),
            "tail-call" => Op::TailCall(count_of(args.first(), "tail-call")?),
            "return" => Op::Return,
            "dict" => {
                let keys = args
                    .iter()
                    .map(|k| match k {
                        Value::Keyword(s) => Ok(s.to_string()),
                        Value::Str(s) => Ok(s.to_string()),
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
            "make-list" => {
                let shape = args
                    .iter()
                    .map(|a| match a {
                        Value::Symbol(s) if &**s == "item" => Ok(false),
                        Value::Symbol(s) if &**s == "splice" => Ok(true),
                        _ => Err(bad("make-list takes item or splice for each element")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                p.list_shapes.push(shape);
                Op::MakeList(p.list_shapes.len() as u32 - 1)
            }
            "next-item" => Op::NextItem(target(args.first())?),
            "macro" => {
                arity(1)?;
                let child = assemble(&args[0])?;
                check_captures(&child, p.nslots, ncaps)?;
                p.protos.push(child);
                Op::Macro(p.protos.len() as u32 - 1)
            }
            "interp" => {
                arity(1)?;
                p.consts.push(args[0].clone());
                Op::Interp(p.consts.len() as u32 - 1)
            }
            other => return Err(bad(format!("unknown instruction {other}"))),
        };
        p.code.push(o);
    }
    p.globals = vec![std::cell::Cell::new(GlobalCache::EMPTY); p.syms.len()];
    Ok(Rc::new(p))
}

/// A closure's captures must name slots and captures the creating proto has.
fn check_captures(child: &Proto, nslots: Option<u32>, ncaps: u32) -> Result<(), LispError> {
    for c in &child.captures {
        match *c {
            CapSource::Local(j) if j >= nslots.unwrap_or(0) => {
                return Err(bad(format!("a closure captures slot {j}, which its maker doesn't have")))
            }
            CapSource::Cap(j) if j >= ncaps => {
                return Err(bad(format!("a closure captures capture {j}, which its maker doesn't have")))
            }
            _ => {}
        }
    }
    Ok(())
}

/// The index of `s` in a proto's symbol table, adding it if it's new.
fn intern(syms: &mut Vec<Sym>, s: Sym) -> u32 {
    match syms.iter().position(|x| *x == s) {
        Some(i) => i as u32,
        None => {
            syms.push(s);
            syms.len() as u32 - 1
        }
    }
}

/// `(def s) (pop)` → `(def-pop s)` and `(def-local i) (pop)` → `(def-local-pop i)`: a
/// definition whose value nobody uses moves it instead of copying it.
fn fuse_def_pop(forms: &[Value]) -> Vec<Value> {
    let is = |f: &Value, name: &str, n: usize| {
        matches!(f, Value::List(l) if l.len() == n && matches!(&l[0], Value::Symbol(s) if &**s == name))
    };
    let mut out = Vec::with_capacity(forms.len());
    let mut i = 0;
    while i < forms.len() {
        let fused = if is(&forms[i], "def", 2) { "def-pop" } else if is(&forms[i], "def-local", 2) { "def-local-pop" } else { "" };
        if i + 1 < forms.len() && !fused.is_empty() && is(&forms[i + 1], "pop", 1) {
            let Value::List(l) = &forms[i] else { unreachable!() };
            out.push(Value::List(Rc::new(vec![Value::Symbol(Sym::new(fused)), l[1].clone()])));
            i += 2;
        } else {
            out.push(forms[i].clone());
            i += 1;
        }
    }
    out
}

fn label_of(v: Option<&Value>) -> Result<String, LispError> {
    match v {
        Some(Value::Symbol(s)) => Ok(s.to_string()),
        Some(Value::Number(n)) => Ok(crate::printer::print_value(&Value::Number(*n), false)),
        Some(Value::Str(s)) => Ok(s.to_string()),
        _ => Err(bad("a label is a symbol, a number or a string")),
    }
}

fn instr(f: &Value) -> Result<(Sym, Vec<Value>), LispError> {
    match f {
        Value::List(l) if !l.is_empty() => Ok((sym_of(&l[0], "an instruction")?, l[1..].to_vec())),
        other => Err(bad(format!("an instruction is a list, got {}", type_name(other)))),
    }
}

fn builtin(env: &Env, name: &str, f: impl Fn(&[Value], &Env) -> Result<Value, LispError> + 'static) {
    env::define(env, name, Value::Builtin(Rc::new(Builtin { name: name.into(), arg_mode: ArgMode::Eval, func: Box::new(f), fast: None })));
}

fn one<'a>(name: &str, args: &'a [Value]) -> Result<&'a Value, LispError> {
    match args {
        [x] => Ok(x),
        _ => Err(LispError::Arity { func: name.into(), expected: "1".into(), got: args.len() }),
    }
}

/// What a global name means to the compiler: "macro" (expand it when compiling), "table-first"
/// or "raw" (a builtin whose arguments are not all evaluated), or nil (compile an ordinary call).
fn global_kind(name: &Value, env: &Env) -> Value {
    let Value::Symbol(s) = name else { return Value::Null };
    match env::get_sym(&root_of(env), s) {
        Ok(Value::Macro(_)) => Value::Str("macro".into()),
        Ok(Value::Builtin(b)) => match b.arg_mode {
            ArgMode::Eval => Value::Null,
            ArgMode::TableFirst => Value::Str("table-first".into()),
            ArgMode::AllRaw => Value::Str("raw".into()),
        },
        _ => Value::Null,
    }
}

pub fn register(env: &Env) {
    builtin(env, "vm-assemble", |args, env| {
        let proto = assemble(one("vm-assemble", args)?)?;
        if !proto.captures.is_empty() {
            return Err(bad("a proto with :captures is made by (closure …) inside another proto, not on its own"));
        }
        Ok(close(&proto, env))
    });
    builtin(env, "vm-run", |args, env| run_in(assemble(one("vm-run", args)?)?, root_of(env)));
    builtin(env, "vm-global-kind", |args, env| Ok(global_kind(one("vm-global-kind", args)?, env)));
    builtin(env, "vm-macroexpand", |args, env| {
        let form = one("vm-macroexpand", args)?;
        if let Value::List(items) = form {
            if let Some(Value::Symbol(h)) = items.first() {
                if let Ok(Value::Macro(m)) = env::get_sym(&root_of(env), h) {
                    return crate::eval::expand_macro(&m, &items[1..]);
                }
            }
        }
        Ok(form.clone())
    });
}
