//! Word-sized values, prototyped: what would the same machine gain if a value were 8 bytes
//! instead of 32? (docs/self-hosting/23-word-sized-values.md)
//!
//!     cargo run --release --example nanbox_proto
//!
//! One small register machine, generic over its value type, runs the same programs twice: with
//! the engine's own `Value` (32 bytes: a tag and a payload, reference-counted kinds behind an
//! `Rc`) and with `Boxed`, 8 bytes, NaN-boxed. A number is its own `f64`; nil, the booleans and
//! pointers to strings, lists and dicts live in the bit patterns no arithmetic produces (quiet NaNs
//! with a tag in the top bits). Everything else is the same: the registers, the frames, the
//! arithmetic, `nth`, `dict-get`, the reference counts. So the ratio is the representation's.
//!
//! What it leaves out: the real machine's globals, cells, marks, handlers and errors, and every
//! other kind of value (functions, symbols, keywords, the database's). It measures the
//! representation's effect on registers and copies, not a finished design.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use eelisp::value::OrderedDict;
use eelisp::{Interpreter, Value};

/// What the machine needs of a value.
trait Val: Clone {
    fn nil() -> Self;
    fn num(n: f64) -> Self;
    /// A number that arithmetic on numbers made: never one of the tagged NaNs (the hardware's
    /// NaN is the canonical one, and a NaN with its sign bit set is a number here).
    fn arith(n: f64) -> Self {
        Self::num(n)
    }
    fn boolean(b: bool) -> Self;
    fn as_num(&self) -> Option<f64>;
    fn truthy(&self) -> bool;
    fn string(s: &str) -> Self;
    fn list(items: Vec<Self>) -> Self;
    fn dict(items: Vec<(&str, Self)>) -> Self;
    /// `(nth self i)`
    fn nth(&self, i: &Self) -> Self;
    /// `(dict-get self key)`
    fn dict_get(&self, key: &Self) -> Self;
    fn same(&self, other: &Self) -> bool;
    fn show(&self) -> String;
}

// ── the engine's own Value, 32 bytes ──

impl Val for Value {
    fn nil() -> Self {
        Value::Null
    }
    fn num(n: f64) -> Self {
        Value::Number(n)
    }
    fn boolean(b: bool) -> Self {
        Value::Bool(b)
    }
    #[inline(always)]
    fn as_num(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
    #[inline(always)]
    fn truthy(&self) -> bool {
        !matches!(self, Value::Null | Value::Bool(false))
    }
    fn string(s: &str) -> Self {
        Value::Str(s.into())
    }
    fn list(items: Vec<Self>) -> Self {
        Value::List(Rc::new(items))
    }
    fn dict(items: Vec<(&str, Self)>) -> Self {
        let mut d = OrderedDict::default();
        for (k, v) in items {
            d.insert(k.to_string(), v);
        }
        Value::Dict(Rc::new(d))
    }
    #[inline(always)]
    fn nth(&self, i: &Self) -> Self {
        match (self, i) {
            (Value::List(l), Value::Number(i)) => l.get(*i as usize).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }
    #[inline(always)]
    fn dict_get(&self, key: &Self) -> Self {
        match (self, key) {
            (Value::Dict(d), Value::Str(k)) => d.get(k).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }
    fn same(&self, other: &Self) -> bool {
        self == other
    }
    fn show(&self) -> String {
        eelisp::printer::print_value(self, false)
    }
}

// ── NaN-boxed, 8 bytes ──

/// A value in one word. Not a quiet NaN with one of the tags below: a number. Pointers are the
/// low 48 bits (what an address uses on the machines this runs on).
#[repr(transparent)]
struct Boxed(u64);

const TAG_MASK: u64 = 0xffff_0000_0000_0000;
const PTR_MASK: u64 = 0x0000_ffff_ffff_ffff;
const CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;
const TAG_NIL: u64 = 0x7ff9_0000_0000_0000;
const TAG_FALSE: u64 = 0x7ffa_0000_0000_0000;
const TAG_TRUE: u64 = 0x7ffb_0000_0000_0000;
const TAG_STR: u64 = 0x7ffc_0000_0000_0000;
const TAG_LIST: u64 = 0x7ffd_0000_0000_0000;
const TAG_DICT: u64 = 0x7ffe_0000_0000_0000;

type BStr = String;
type BList = Vec<Boxed>;
type BDict = HashMap<String, Boxed>;

impl Boxed {
    #[inline(always)]
    fn tag(&self) -> u64 {
        self.0 & TAG_MASK
    }
    #[inline(always)]
    fn ptr<T>(&self) -> *const T {
        (self.0 & PTR_MASK) as *const T
    }
    fn from_rc<T>(tag: u64, rc: Rc<T>) -> Boxed {
        let p = Rc::into_raw(rc) as u64;
        assert_eq!(p & TAG_MASK, 0, "a pointer wider than 48 bits");
        Boxed(tag | p)
    }
    /// The pointer tags are the three highest, so one comparison of the whole word decides it.
    #[inline(always)]
    fn is_ptr(&self) -> bool {
        self.0 >= TAG_STR && self.0 < 0x7fff_0000_0000_0000
    }
}

impl Clone for Boxed {
    #[inline(always)]
    fn clone(&self) -> Boxed {
        if self.is_ptr() {
            // SAFETY: a pointer tag always holds a pointer from Rc::into_raw of that type
            unsafe {
                match self.tag() {
                    TAG_STR => Rc::increment_strong_count(self.ptr::<BStr>()),
                    TAG_LIST => Rc::increment_strong_count(self.ptr::<BList>()),
                    _ => Rc::increment_strong_count(self.ptr::<BDict>()),
                }
            }
        }
        Boxed(self.0)
    }
}

impl Drop for Boxed {
    #[inline(always)]
    fn drop(&mut self) {
        if self.is_ptr() {
            // SAFETY: as in clone; this value's own reference is given back
            unsafe {
                match self.tag() {
                    TAG_STR => Rc::decrement_strong_count(self.ptr::<BStr>()),
                    TAG_LIST => Rc::decrement_strong_count(self.ptr::<BList>()),
                    _ => Rc::decrement_strong_count(self.ptr::<BDict>()),
                }
            }
        }
    }
}

impl Val for Boxed {
    fn nil() -> Self {
        Boxed(TAG_NIL)
    }
    #[inline(always)]
    fn num(n: f64) -> Self {
        Boxed(if n.is_nan() { CANONICAL_NAN } else { n.to_bits() })
    }
    #[inline(always)]
    fn arith(n: f64) -> Self {
        Boxed(n.to_bits())
    }
    #[inline(always)]
    fn boolean(b: bool) -> Self {
        Boxed(if b { TAG_TRUE } else { TAG_FALSE })
    }
    #[inline(always)]
    fn as_num(&self) -> Option<f64> {
        // the tags are 0x7ff9… to 0x7ffe…: one unsigned comparison after a subtraction
        if self.0.wrapping_sub(TAG_NIL) < (0x7fff_0000_0000_0000 - TAG_NIL) {
            None
        } else {
            Some(f64::from_bits(self.0))
        }
    }
    #[inline(always)]
    fn truthy(&self) -> bool {
        self.0 != TAG_NIL && self.0 != TAG_FALSE
    }
    fn string(s: &str) -> Self {
        Boxed::from_rc(TAG_STR, Rc::new(s.to_string()))
    }
    fn list(items: Vec<Self>) -> Self {
        Boxed::from_rc(TAG_LIST, Rc::new(items))
    }
    fn dict(items: Vec<(&str, Self)>) -> Self {
        Boxed::from_rc(TAG_DICT, Rc::new(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<BDict>()))
    }
    #[inline(always)]
    fn nth(&self, i: &Self) -> Self {
        match (self.tag(), i.as_num()) {
            // SAFETY: tag says list
            (TAG_LIST, Some(i)) => unsafe { &*self.ptr::<BList>() }.get(i as usize).cloned().unwrap_or(Boxed(TAG_NIL)),
            _ => Boxed(TAG_NIL),
        }
    }
    #[inline(always)]
    fn dict_get(&self, key: &Self) -> Self {
        match (self.tag(), key.tag()) {
            // SAFETY: tags say dict and string
            (TAG_DICT, TAG_STR) => unsafe { &*self.ptr::<BDict>() }
                .get(unsafe { &*key.ptr::<BStr>() }.as_str())
                .cloned()
                .unwrap_or(Boxed(TAG_NIL)),
            _ => Boxed(TAG_NIL),
        }
    }
    fn same(&self, other: &Self) -> bool {
        match (self.as_num(), other.as_num()) {
            (Some(a), Some(b)) => a == b,
            _ => self.0 == other.0,
        }
    }
    fn show(&self) -> String {
        match self.as_num() {
            Some(n) => eelisp::printer::print_value(&Value::Number(n), false),
            None => format!("#<{:x}>", self.0),
        }
    }
}

// ── the machine ──

#[derive(Clone, Copy)]
enum Rk {
    R(u16),
    K(u16),
}

#[derive(Clone, Copy)]
enum Op {
    Add(u16, Rk, Rk),
    Sub(u16, Rk, Rk),
    Lt(u16, Rk, Rk),
    Eq(u16, Rk, Rk),
    Nth(u16, Rk, Rk),
    DictGet(u16, Rk, Rk),
    Move(u16, Rk),
    JumpIfNot(u16, u32),
    Jump(u32),
    /// dst ← (protos[f] args in registers argbase…argbase+n): the callee's frame starts there
    Call(u16, u16, u16, u16),
    Return(Rk),
}

struct Proto<V> {
    code: Vec<Op>,
    consts: Vec<V>,
    nregs: usize,
}

struct Frame {
    proto: usize,
    pc: usize,
    base: usize,
    ret: usize,
}

#[inline(always)]
fn rk<'a, V>(regs: &'a [V], base: usize, consts: &'a [V], x: Rk) -> &'a V {
    match x {
        Rk::R(r) => &regs[base + r as usize],
        Rk::K(k) => &consts[k as usize],
    }
}

fn run<V: Val>(protos: &[Proto<V>], entry: usize, args: Vec<V>) -> V {
    let mut regs: Vec<V> = args;
    regs.resize(protos[entry].nregs, V::nil());
    let mut frames = vec![Frame { proto: entry, pc: 0, base: 0, ret: 0 }];
    'frame: loop {
        let (pi, mut pc, base) = {
            let f = frames.last().unwrap();
            (f.proto, f.pc, f.base)
        };
        let p = &protos[pi];
        loop {
            let op = p.code[pc];
            pc += 1;
            macro_rules! arith {
                ($d:expr, $a:expr, $b:expr, $f:expr) => {{
                    let (x, y) = (rk(&regs, base, &p.consts, $a).as_num(), rk(&regs, base, &p.consts, $b).as_num());
                    let v = match (x, y) {
                        (Some(x), Some(y)) => $f(x, y),
                        _ => V::nil(),
                    };
                    regs[base + $d as usize] = v;
                }};
            }
            match op {
                Op::Add(d, a, b) => arith!(d, a, b, |x, y| V::arith(x + y)),
                Op::Sub(d, a, b) => arith!(d, a, b, |x, y| V::arith(x - y)),
                Op::Lt(d, a, b) => arith!(d, a, b, |x: f64, y: f64| V::boolean(x < y)),
                Op::Eq(d, a, b) => {
                    let v = V::boolean(rk(&regs, base, &p.consts, a).same(rk(&regs, base, &p.consts, b)));
                    regs[base + d as usize] = v;
                }
                Op::Nth(d, a, b) => {
                    let v = rk(&regs, base, &p.consts, a).nth(rk(&regs, base, &p.consts, b));
                    regs[base + d as usize] = v;
                }
                Op::DictGet(d, a, b) => {
                    let v = rk(&regs, base, &p.consts, a).dict_get(rk(&regs, base, &p.consts, b));
                    regs[base + d as usize] = v;
                }
                Op::Move(d, a) => {
                    let v = rk(&regs, base, &p.consts, a).clone();
                    regs[base + d as usize] = v;
                }
                Op::JumpIfNot(r, t) => {
                    if !regs[base + r as usize].truthy() {
                        pc = t as usize;
                    }
                }
                Op::Jump(t) => pc = t as usize,
                Op::Call(d, f, a, n) => {
                    frames.last_mut().unwrap().pc = pc;
                    let at = base + a as usize;
                    let need = at + protos[f as usize].nregs;
                    regs.truncate(at + n as usize);
                    regs.resize(need, V::nil());
                    frames.push(Frame { proto: f as usize, pc: 0, base: at, ret: base + d as usize });
                    continue 'frame;
                }
                Op::Return(x) => {
                    let v = rk(&regs, base, &p.consts, x).clone();
                    let done = frames.pop().unwrap();
                    match frames.last() {
                        None => return v,
                        Some(caller) => {
                            let end = caller.base + protos[caller.proto].nregs;
                            regs.truncate(end.max(done.base));
                            regs.resize(end, V::nil());
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

/// The three programs, as protos for value type V: fib (0), sum-to (1), skip (2).
fn programs<V: Val>() -> Vec<Proto<V>> {
    vec![
        // (defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
        Proto {
            code: vec![
                Lt(1, R(0), K(0)),
                JumpIfNot(1, 3),
                Return(R(0)),
                Sub(2, R(0), K(1)),
                Call(1, 0, 2, 1),
                Sub(3, R(0), K(0)),
                Call(2, 0, 3, 1),
                Add(1, R(1), R(2)),
                Return(R(1)),
            ],
            consts: vec![V::num(2.0), V::num(1.0)],
            nregs: 4,
        },
        // (loop (i 0 acc 0) (if (= i k) acc (recur (+ i 1) (+ acc i))))   k in r0
        Proto {
            code: vec![
                Move(1, K(0)),
                Move(2, K(0)),
                Eq(3, R(1), R(0)),
                JumpIfNot(3, 5),
                Return(R(2)),
                Add(3, R(1), K(1)),
                Add(2, R(2), R(1)),
                Move(1, R(3)),
                Jump(2),
            ],
            consts: vec![V::num(0.0), V::num(1.0)],
            nregs: 4,
        },
        // the reader's whitespace loop: cs r0, n r1, i r2, classes r3
        // (loop (i i) (if (and (< i n) (= (dict-get classes (nth cs i)) 0)) (recur (+ i 1)) i))
        Proto {
            code: vec![
                Lt(4, R(2), R(1)),
                JumpIfNot(4, 7),
                Nth(5, R(0), R(2)),
                DictGet(5, R(3), R(5)),
                Eq(5, R(5), K(0)),
                JumpIfNot(5, 7),
                Add(2, R(2), K(1)),
                // 7 is the jump target for both exits; the loop back is just before it
            ],
            consts: vec![V::num(0.0), V::num(1.0)],
            nregs: 6,
        },
    ]
}

fn skip_program<V: Val>(mut p: Vec<Proto<V>>) -> Vec<Proto<V>> {
    // finish the skip loop: jump back to the top, and the exit returns i
    let code = &mut p[2].code;
    code.push(Jump(0));
    code.push(Return(R(2)));
    // the exits jump to 8 (the Return), not 7 (the Jump back)
    for op in code.iter_mut() {
        if let JumpIfNot(r, 7) = *op {
            *op = JumpIfNot(r, 8);
        }
    }
    p
}

fn best<F: FnMut() -> R2, R2>(mut f: F) -> (f64, R2) {
    let mut out = None;
    let ms = (0..7)
        .map(|_| {
            let t = Instant::now();
            out = Some(f());
            t.elapsed().as_secs_f64() * 1e3
        })
        .fold(f64::MAX, f64::min);
    (ms, out.unwrap())
}

/// A program's name, which proto it starts in, and its arguments.
type Run<V> = (&'static str, usize, Vec<V>);

fn cases<V: Val>(spaces: usize) -> (Vec<Proto<V>>, Vec<Run<V>>) {
    let protos = skip_program(programs::<V>());
    let cs = V::list((0..spaces).map(|_| V::string(" ")).collect());
    let classes = V::dict(vec![(" ", V::num(0.0)), ("a", V::num(3.0))]);
    let runs = vec![
        ("fib 25", 0, vec![V::num(25.0)]),
        ("loop 1M", 1, vec![V::num(1e6)]),
        ("whitespace 300k", 2, vec![cs, V::num(spaces as f64), V::num(0.0), classes]),
    ];
    (protos, runs)
}

fn main() {
    println!("size of a value: Value {} bytes, Boxed {} bytes", std::mem::size_of::<Value>(), std::mem::size_of::<Boxed>());
    let (wp, wr) = cases::<Value>(300_000);
    let (bp, br) = cases::<Boxed>(300_000);
    // the real machine, for reference: the same programs compiled from EELisp
    let it = Interpreter::new();
    it.set_compiled(true).unwrap();
    it.eval_str(
        r#"(defn fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
           (defn sum-to (k) (loop (i 0 acc 0) (if (= i k) acc (recur (+ i 1) (+ acc i)))))
           (def classes {" " 0 "a" 3})
           (defn skip (cs n i) (loop (i i) (if (and (< i n) (= (dict-get classes (nth cs i)) 0)) (recur (+ i 1)) i)))
           (def spaces (str-chars (str-join "" (map (fn (_) " ") (range 0 300000)))))"#,
    )
    .unwrap();
    let real = ["(fib 25)", "(sum-to 1000000)", "(skip spaces 300000 0)"];
    println!("{:<16} {:>14} {:>14} {:>8} {:>16}", "", "32-byte Value", "8-byte boxed", "", "src/vm.rs today");
    for (k, ((name, e, wargs), (_, _, bargs))) in wr.into_iter().zip(br).enumerate() {
        let (a, va) = best(|| run(&wp, e, wargs.clone()));
        let (b, vb) = best(|| run(&bp, e, bargs.clone()));
        assert_eq!(va.show(), vb.show(), "{name}: the two representations disagree");
        let (c, vc) = best(|| it.eval_str(real[k]).unwrap());
        assert_eq!(va.show(), vc.show(), "{name}: the prototype and the real machine disagree");
        println!("{name:<16} {a:11.1} ms {b:11.1} ms {:7.2}x {c:13.1} ms", a / b);
    }
}
