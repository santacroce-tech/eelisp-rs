//! Lexical environment — a chain of scopes (ANALYSIS §4.3). Reference-typed so closures share.
//!
//! A lookup happens for every symbol evaluated and a binding for every argument passed, so this
//! is the interpreter's hottest code. Names are `Rc<str>` — binding a parameter the evaluator
//! already holds as a `Sym` is a reference-count bump, not an allocation — and hashed with
//! FxHash rather than SipHash, whose resistance to hash flooding buys nothing here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use crate::value::{LispError, Sym, Value};

/// The Fx hash (rustc's): a rotate, a xor and a multiply per word.
#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type Vars = HashMap<Sym, Value, BuildHasherDefault<FxHasher>>;

pub struct Scope {
    pub vars: Vars,
    pub parent: Option<Env>,
}

pub type Env = Rc<RefCell<Scope>>;

pub fn root() -> Env {
    Rc::new(RefCell::new(Scope { vars: Vars::default(), parent: None }))
}

pub fn child(parent: &Env) -> Env {
    Rc::new(RefCell::new(Scope { vars: Vars::default(), parent: Some(parent.clone()) }))
}

pub fn define(env: &Env, name: &str, val: Value) {
    env.borrow_mut().vars.insert(Sym::from(name), val);
}

/// `define` for a name the caller already holds as a `Sym` — no allocation.
pub fn define_sym(env: &Env, name: &Sym, val: Value) {
    env.borrow_mut().vars.insert(name.clone(), val);
}

pub fn get(env: &Env, name: &str) -> Result<Value, LispError> {
    let mut cur = env.clone();
    loop {
        let next = {
            let scope = cur.borrow();
            if let Some(v) = scope.vars.get(name) {
                return Ok(v.clone());
            }
            scope.parent.clone()
        };
        match next {
            Some(p) => cur = p,
            None => return Err(LispError::UndefinedSymbol(name.to_string())),
        }
    }
}

pub fn set(env: &Env, name: &str, val: Value) -> Result<(), LispError> {
    let mut cur = env.clone();
    loop {
        let next = {
            let mut scope = cur.borrow_mut();
            if let Some(slot) = scope.vars.get_mut(name) {
                *slot = val;
                return Ok(());
            }
            scope.parent.clone()
        };
        match next {
            Some(p) => cur = p,
            None => return Err(LispError::UndefinedSymbol(name.to_string())),
        }
    }
}
