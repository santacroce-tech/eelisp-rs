//! A register machine, prototyped by hand: is it worth a new back end for the compiler?
//!
//!     cargo run --release --example regvm_proto
//!
//! Three programs are written directly in register code and timed against the same programs
//! compiled for the current stack VM. The prototype keeps every cost the real machine would have
//! to keep: globals are read through the same cache (`env::global_slot`, checked against
//! `env::layout()`), every call of `+`, `<`, `nth`… looks the global up and calls whatever is
//! there, with the same fast path for the language's own arithmetic. What it leaves out is noted
//! where it matters; see docs/plans/register-vm.md.
//!
//! The idea, as in Lua 5: instructions name the registers they read and write. `(< n 2)` is one
//! instruction, `(call2 r1 < r0 #2)`, instead of four (load `<`, push n, push 2, call). And a
//! compiled function's arguments are already in the registers its frame starts at: nothing is
//! copied into a new frame.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use eelisp::env::{self, Env};
use eelisp::value::Sym;
use eelisp::{Interpreter, LispError, Value};

/// An operand: a register of the current frame, or a constant of the proto.
#[derive(Clone, Copy)]
enum Rk {
    R(u16),
    K(u16),
}

#[derive(Clone, Copy)]
enum Op {
    /// dst ← constant
    LoadK(u16, u16),
    /// dst ← register
    Move(u16, u16),
    /// dst ← global
    Global(u16, u16),
    /// dst ← (global args…), the arguments in registers argbase.. argbase+n
    CallG(u16, u16, u16, u16),
    /// dst ← (global a b), each operand a register or a constant
    CallG2(u16, u16, Rk, Rk),
    Jump(u32),
    JumpIfNot(u16, u32),
    Return(Rk),
}

#[derive(Clone, Copy)]
struct GlobalCache {
    scope: *const (),
    layout: u64,
    value: *const Value,
}

struct Proto {
    code: Vec<Op>,
    consts: Vec<Value>,
    syms: Vec<Sym>,
    /// The register-code function each name stands for, if it is one: the prototype's stand-in
    /// for a `Value::Function` carrying register code. The global is still read, as a real
    /// machine would have to.
    callees: Vec<std::cell::OnceCell<Rc<Proto>>>,
    nregs: usize,
    cache: Vec<Cell<GlobalCache>>,
}

impl Proto {
    fn new(code: Vec<Op>, consts: Vec<Value>, names: &[&str], nregs: usize) -> Proto {
        let empty = GlobalCache { scope: std::ptr::null(), layout: u64::MAX, value: std::ptr::null() };
        Proto {
            code,
            consts,
            syms: names.iter().map(|n| Sym::from(*n)).collect(),
            callees: names.iter().map(|_| std::cell::OnceCell::new()).collect(),
            nregs,
            cache: names.iter().map(|_| Cell::new(empty)).collect(),
        }
    }
}

struct Frame {
    proto: Rc<Proto>,
    pc: usize,
    base: usize,
    /// Where the result goes, as an absolute register index; unused for the entry frame.
    ret: usize,
}

fn global(env: &Env, p: &Proto, s: u16) -> Result<Value, LispError> {
    let c = p.cache[s as usize].get();
    if c.scope == Rc::as_ptr(env) as *const () && c.layout == env::layout() {
        return Ok(unsafe { &*c.value }.clone());
    }
    let name = &p.syms[s as usize];
    match env::global_slot(env, name) {
        Some(value) => {
            p.cache[s as usize].set(GlobalCache { scope: Rc::as_ptr(env) as *const (), layout: env::layout(), value });
            Ok(unsafe { &*value }.clone())
        }
        None => env::get(env, name.as_ref()),
    }
}

#[inline(always)]
fn rk<'a>(regs: &'a [Value], base: usize, consts: &'a [Value], x: Rk) -> &'a Value {
    match x {
        Rk::R(r) => &regs[base + r as usize],
        Rk::K(k) => &consts[k as usize],
    }
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

fn run(entry: Rc<Proto>, args: Vec<Value>, env: &Env) -> Result<Value, LispError> {
    let mut regs: Vec<Value> = args;
    regs.resize(entry.nregs, Value::Null);
    let mut frames = vec![Frame { proto: entry, pc: 0, base: 0, ret: 0 }];
    'frame: loop {
        let (proto, mut pc, base) = {
            let f = frames.last().unwrap();
            (f.proto.clone(), f.pc, f.base)
        };
        let code = &proto.code;
        loop {
            let op = code[pc];
            pc += 1;
            match op {
                Op::LoadK(d, k) => regs[base + d as usize] = proto.consts[k as usize].clone(),
                Op::Move(d, s) => regs[base + d as usize] = regs[base + s as usize].clone(),
                Op::Global(d, s) => regs[base + d as usize] = global(env, &proto, s)?,
                Op::Jump(t) => pc = t as usize,
                Op::JumpIfNot(r, t) => {
                    if !truthy(&regs[base + r as usize]) {
                        pc = t as usize
                    }
                }
                Op::CallG2(d, s, a, b) => {
                    let f = global(env, &proto, s)?;
                    let (x, y) = (rk(&regs, base, &proto.consts, a), rk(&regs, base, &proto.consts, b));
                    let v = match &f {
                        Value::Builtin(bi) => match (bi.fast, x, y) {
                            (Some(op), Value::Number(x), Value::Number(y)) => op.apply(*x, *y),
                            _ => (bi.func)(&[x.clone(), y.clone()], env)?,
                        },
                        _ => unimplemented!("the prototype calls builtins with call2"),
                    };
                    regs[base + d as usize] = v;
                }
                Op::CallG(d, s, a, n) => {
                    let f = global(env, &proto, s)?;
                    let at = base + a as usize;
                    if let Some(callee) = proto.callees[s as usize].get() {
                        // the arguments are already where the callee's registers begin
                        frames.last_mut().unwrap().pc = pc;
                        let need = at + callee.nregs;
                        if regs.len() < need {
                            regs.resize(need, Value::Null);
                        }
                        for r in &mut regs[at + n as usize..need] {
                            *r = Value::Null;
                        }
                        frames.push(Frame { proto: callee.clone(), pc: 0, base: at, ret: base + d as usize });
                        continue 'frame;
                    }
                    let v = match &f {
                        Value::Builtin(bi) => (bi.func)(&regs[at..at + n as usize], env)?,
                        _ => unimplemented!("the prototype calls builtins and register code"),
                    };
                    regs[base + d as usize] = v;
                }
                Op::Return(x) => {
                    let v = rk(&regs, base, &proto.consts, x).clone();
                    let done = frames.pop().unwrap();
                    match frames.last() {
                        None => return Ok(v),
                        Some(caller) => {
                            regs.truncate(caller.base + caller.proto.nregs);
                            regs[done.ret] = v;
                            continue 'frame;
                        }
                    }
                }
            }
        }
    }
}

use Op::*;
use Rk::{K, R};

/// (defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))   — n in r0
fn fib() -> Rc<Proto> {
    let p = Rc::new(Proto::new(
        vec![
            CallG2(1, 0, R(0), K(0)), // r1 ← (< n 2)
            JumpIfNot(1, 3),
            Return(R(0)),
            CallG2(2, 1, R(0), K(1)), // r2 ← (- n 1)
            CallG(1, 2, 2, 1),        // r1 ← (fib r2)
            CallG2(3, 1, R(0), K(0)), // r3 ← (- n 2)
            CallG(2, 2, 3, 1),        // r2 ← (fib r3)
            CallG2(1, 3, R(1), R(2)), // r1 ← (+ r1 r2)
            Return(R(1)),
        ],
        vec![Value::Number(2.0), Value::Number(1.0)],
        &["<", "-", "fib", "+"],
        4,
    ));
    // fib calls itself: the knot is tied once (a cycle, never freed, which a prototype can afford)
    p.callees[2].set(p.clone()).ok();
    p
}

/// (loop (i 0 acc 0) (if (= i k) acc (recur (+ i 1) (+ acc i))))   — k in r0, i r1, acc r2
fn sum_to() -> Rc<Proto> {
    Rc::new(Proto::new(
        vec![
            LoadK(1, 0),
            LoadK(2, 0),
            CallG2(3, 0, R(1), R(0)), // r3 ← (= i k)
            JumpIfNot(3, 5),
            Return(R(2)),
            CallG2(3, 1, R(1), K(1)), // r3 ← (+ i 1)
            CallG2(2, 1, R(2), R(1)), // acc ← (+ acc i)
            Move(1, 3),
            Jump(2),
        ],
        vec![Value::Number(0.0), Value::Number(1.0)],
        &["=", "+"],
        4,
    ))
}

/// The reader's whitespace loop:
/// (loop (i i) (if (and (< i n) (= (dict-get classes (nth cs i)) 0)) (recur (+ i 1)) i))
/// — cs r0, n r1, i r2
fn skip() -> Rc<Proto> {
    Rc::new(Proto::new(
        vec![
            CallG2(3, 0, R(2), R(1)), // 0: r3 ← (< i n)
            JumpIfNot(3, 9),
            CallG2(4, 1, R(0), R(2)), // 2: r4 ← (nth cs i)
            Global(3, 2),             // 3: r3 ← classes
            CallG2(3, 3, R(3), R(4)), // 4: r3 ← (dict-get classes r4)
            CallG2(3, 4, R(3), K(0)), // 5: r3 ← (= r3 0)
            JumpIfNot(3, 9),
            CallG2(2, 5, R(2), K(1)), // 7: i ← (+ i 1)
            Jump(0),
            Return(R(2)), // 9
        ],
        vec![Value::Number(0.0), Value::Number(1.0)],
        &["<", "nth", "classes", "dict-get", "=", "+"],
        5,
    ))
}

// ─────────── the same machine, carrying the real machine's costs ───────────
//
// `run` above is a ceiling. `run_real` adds what the stack VM's frames and slots carry, to see
// how much of the gain survives: registers that can be unbound (their name then means the
// global) or shared with a closure through a cell, a frame holding its environment, its
// captured cells and where its loop marks and handlers start, and a callee entered through its
// function value (its closure becomes the frame's environment).

type Shared = Rc<std::cell::RefCell<Option<Value>>>;

#[derive(Clone)]
enum Reg {
    Unbound,
    Val(Value),
    #[allow(dead_code)]
    Cell(Shared),
}

struct RealFrame {
    proto: Rc<Proto>,
    pc: usize,
    base: usize,
    ret: usize,
    env: Env,
    #[allow(dead_code)]
    upvals: Option<Rc<[eelisp::vm::Cell]>>,
    #[allow(dead_code)]
    mark0: u32,
    #[allow(dead_code)]
    handler0: u32,
}

#[inline(always)]
fn copy(v: &Value) -> Value {
    match v {
        Value::Number(n) => Value::Number(*n),
        Value::Bool(b) => Value::Bool(*b),
        Value::Null => Value::Null,
        other => other.clone(),
    }
}

/// A register's value. An unbound one would mean the global of the slot's name; the hand-written
/// programs never read one, but the check is paid.
#[inline(always)]
fn reg(r: &Reg) -> Value {
    match r {
        Reg::Val(v) => copy(v),
        Reg::Cell(c) => c.borrow().clone().expect("unbound cell"),
        Reg::Unbound => panic!("the prototype reads no unbound register"),
    }
}

#[inline(always)]
fn rk_real(regs: &[Reg], base: usize, consts: &[Value], x: Rk) -> Value {
    match x {
        Rk::R(r) => reg(&regs[base + r as usize]),
        Rk::K(k) => copy(&consts[k as usize]),
    }
}

#[inline(always)]
fn set_reg(r: &mut Reg, v: Value) {
    match r {
        Reg::Cell(c) => *c.borrow_mut() = Some(v),
        r => *r = Reg::Val(v),
    }
}

fn run_real(entry: Rc<Proto>, args: Vec<Value>, env: &Env) -> Result<Value, LispError> {
    let mut regs: Vec<Reg> = args.into_iter().map(Reg::Val).collect();
    regs.resize(entry.nregs, Reg::Unbound);
    let mut frames =
        vec![RealFrame { proto: entry, pc: 0, base: 0, ret: 0, env: env.clone(), upvals: None, mark0: 0, handler0: 0 }];
    'frame: loop {
        let (proto, mut pc, base, fenv) = {
            let f = frames.last().unwrap();
            (f.proto.clone(), f.pc, f.base, f.env.clone())
        };
        let code = &proto.code;
        loop {
            let op = code[pc];
            pc += 1;
            match op {
                Op::LoadK(d, k) => set_reg(&mut regs[base + d as usize], copy(&proto.consts[k as usize])),
                Op::Move(d, s) => {
                    let v = reg(&regs[base + s as usize]);
                    set_reg(&mut regs[base + d as usize], v)
                }
                Op::Global(d, s) => {
                    let v = global(&fenv, &proto, s)?;
                    set_reg(&mut regs[base + d as usize], v)
                }
                Op::Jump(t) => pc = t as usize,
                Op::JumpIfNot(r, t) => {
                    let v = reg(&regs[base + r as usize]);
                    if !truthy(&v) {
                        pc = t as usize
                    }
                }
                Op::CallG2(d, s, a, b) => {
                    let f = global(&fenv, &proto, s)?;
                    let v = match &f {
                        Value::Builtin(bi) => {
                            let (x, y) = (&regs[base..], &proto.consts);
                            match (bi.fast, a, b) {
                                (Some(op), a, b) => {
                                    let (x, y) = (rk_real(x, 0, y, a), rk_real(x, 0, y, b));
                                    match (&x, &y) {
                                        (Value::Number(x), Value::Number(y)) => op.apply(*x, *y),
                                        _ => (bi.func)(&[x, y], &fenv)?,
                                    }
                                }
                                (None, a, b) => {
                                    let args = [rk_real(x, 0, y, a), rk_real(x, 0, y, b)];
                                    (bi.func)(&args, &fenv)?
                                }
                            }
                        }
                        _ => unimplemented!("the prototype calls builtins with call2"),
                    };
                    set_reg(&mut regs[base + d as usize], v);
                }
                Op::CallG(d, s, a, n) => {
                    let f = global(&fenv, &proto, s)?;
                    let at = base + a as usize;
                    if let (Some(callee), Value::Function(func)) = (proto.callees[s as usize].get(), &f) {
                        frames.last_mut().unwrap().pc = pc;
                        let need = at + callee.nregs;
                        regs.truncate(at + n as usize);
                        regs.resize(need, Reg::Unbound);
                        frames.push(RealFrame {
                            proto: callee.clone(),
                            pc: 0,
                            base: at,
                            ret: base + d as usize,
                            env: func.closure.clone(),
                            upvals: func.upvals.clone(),
                            mark0: 0,
                            handler0: 0,
                        });
                        continue 'frame;
                    }
                    let v = match &f {
                        Value::Builtin(bi) => {
                            let args: Vec<Value> = regs[at..at + n as usize].iter().map(reg).collect();
                            (bi.func)(&args, &fenv)?
                        }
                        _ => unimplemented!("the prototype calls builtins and register code"),
                    };
                    set_reg(&mut regs[base + d as usize], v);
                }
                Op::Return(x) => {
                    let v = rk_real(&regs, base, &proto.consts, x);
                    let done = frames.pop().unwrap();
                    match frames.last() {
                        None => {
                            regs.clear();
                            return Ok(v);
                        }
                        Some(caller) => {
                            regs.truncate(caller.base + caller.proto.nregs);
                            set_reg(&mut regs[done.ret], v);
                            continue 'frame;
                        }
                    }
                }
            }
        }
    }
}

/// A program's name, its stack-VM call, its register-code call, the prototype's code and arguments.
type Case = (&'static str, &'static str, &'static str, Rc<Proto>, Vec<Value>);

fn best<F: FnMut() -> Value>(mut f: F) -> (f64, Value) {
    let mut v = Value::Null;
    let ms = (0..5)
        .map(|_| {
            let t = Instant::now();
            v = f();
            t.elapsed().as_secs_f64() * 1e3
        })
        .fold(f64::MAX, f64::min);
    (ms, v)
}

fn main() {
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    it.eval_str(
        r#"(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
           (defn sum-to (k) (loop (i 0 acc 0) (if (= i k) acc (recur (+ i 1) (+ acc i)))))
           (def classes {" " 0 "a" 3})
           (defn skip (cs n i)
             (loop (i i) (if (and (< i n) (= (dict-get classes (nth cs i)) 0)) (recur (+ i 1)) i)))
           (def spaces (str-chars (str-join "" (map (fn (_) " ") (range 0 300000)))))"#,
    )
    .unwrap();
    // The same three programs as register code for the real machine (src/vm.rs), assembled
    it.eval_str(
        r#"(def rfib (vm-assemble '(proto :name "rfib" :params (n) :slots 4 :code (
               (call2 1 < (local 0 n) (const 2))
               (jump-if-not 1 more)
               (return-value (local 0 n))
               (label more)
               (call2 2 - (local 0 n) (const 1))
               (call-global 1 rfib 2 1)
               (call2 3 - (local 0 n) (const 2))
               (call-global 2 rfib 3 1)
               (call2 1 + 1 2)
               (return-value 1)))))
           (def rsum-to (vm-assemble '(proto :params (k) :slots 4 :code (
               (move 1 (const 0)) (move 2 (const 0))
               (label top)
               (call2 3 = 1 (local 0 k))
               (jump-if-not 3 go)
               (return-value 2)
               (label go)
               (call2 2 + 2 1)
               (call2 1 + 1 (const 1))
               (jump top)))))
           (def rskip (vm-assemble '(proto :params (cs n i) :slots 5 :code (
               (label top)
               (call2 3 < (local 2 i) (local 1 n))
               (jump-if-not 3 done)
               (call2 4 nth (local 0 cs) (local 2 i))
               (global 3 classes)
               (call2 3 dict-get 3 4)
               (call2 3 = 3 (const 0))
               (jump-if-not 3 done)
               (call2 2 + (local 2 i) (const 1))
               (jump top)
               (label done)
               (return-value (local 2 i))))))"#,
    )
    .unwrap();
    let env = it.global.clone();
    let spaces = env::get(&env, "spaces").unwrap();
    let cases: Vec<Case> = vec![
        ("fib 25", "(fib 25)", "(rfib 25)", fib(), vec![Value::Number(25.0)]),
        ("loop 1M", "(sum-to 1000000)", "(rsum-to 1000000)", sum_to(), vec![Value::Number(1e6)]),
        (
            "whitespace 300k",
            "(skip spaces 300000 0)",
            "(rskip spaces 300000 0)",
            skip(),
            vec![spaces, Value::Number(300000.0), Value::Number(0.0)],
        ),
    ];
    println!(
        "{:<16} {:>10} {:>18} {:>18} {:>18}",
        "", "stack VM", "ceiling", "real costs", "src/vm.rs registers"
    );
    for (name, src, rsrc, proto, args) in cases {
        let (a, va) = best(|| it.eval_str(src).unwrap());
        let (b, vb) = best(|| run(proto.clone(), args.clone(), &env).unwrap());
        let (c, vc) = best(|| run_real(proto.clone(), args.clone(), &env).unwrap());
        let (d, vd) = best(|| it.eval_str(rsrc).unwrap());
        assert_eq!(va, vb, "{name}: the two machines disagree");
        assert_eq!(va, vc, "{name}: the stack VM and the realistic machine disagree");
        assert_eq!(va, vd, "{name}: the stack and register code of the real machine disagree");
        println!(
            "{name:<16} {a:7.1} ms {b:7.1} ms {:5.2}x {c:7.1} ms {:5.2}x {d:7.1} ms {:5.2}x",
            a / b,
            a / c,
            a / d
        );
    }
}
