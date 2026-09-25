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

/// One scope. The global scope holds hundreds of names in `vars`, a hash map. Every other scope
/// — a call's parameters, a `let`, a `loop` — holds a handful, and is made and dropped on every
/// call: those live in `local`, a short vector searched front to back, which costs no hashing
/// and allocates nothing until the first binding.
pub struct Scope {
    pub vars: Vars,
    pub local: Vec<(Sym, Value)>,
    pub parent: Option<Env>,
}

impl Scope {
    #[inline]
    fn lookup(&self, name: &Sym) -> Option<&Value> {
        if self.parent.is_none() {
            return self.vars.get(name);
        }
        self.local.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    #[inline]
    fn lookup_mut(&mut self, name: &Sym) -> Option<&mut Value> {
        if self.parent.is_none() {
            return self.vars.get_mut(name);
        }
        self.local.iter_mut().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    fn bind(&mut self, name: Sym, val: Value) {
        if self.parent.is_none() {
            self.vars.insert(name, val);
        } else if let Some(slot) = self.lookup_mut(&name) {
            *slot = val;
        } else {
            self.local.push((name, val));
        }
    }

    /// Every binding in this one scope (not its parents).
    pub fn bindings(&self) -> Box<dyn Iterator<Item = (&Sym, &Value)> + '_> {
        Box::new(self.vars.iter().chain(self.local.iter().map(|(k, v)| (k, v))))
    }
}

pub type Env = Rc<RefCell<Scope>>;

pub fn root() -> Env {
    Rc::new(RefCell::new(Scope { vars: Vars::default(), local: Vec::new(), parent: None }))
}

pub fn child(parent: &Env) -> Env {
    Rc::new(RefCell::new(Scope { vars: Vars::default(), local: Vec::new(), parent: Some(parent.clone()) }))
}

/// A child scope with room for `n` bindings — a call knows how many parameters it binds.
pub fn child_with(parent: &Env, n: usize) -> Env {
    Rc::new(RefCell::new(Scope {
        vars: Vars::default(),
        local: Vec::with_capacity(n),
        parent: Some(parent.clone()),
    }))
}

/// A child scope holding these bindings already — how a call binds its parameters.
pub fn child_from(parent: &Env, local: Vec<(Sym, Value)>) -> Env {
    Rc::new(RefCell::new(Scope { vars: Vars::default(), local, parent: Some(parent.clone()) }))
}

pub fn define(env: &Env, name: &str, val: Value) {
    env.borrow_mut().bind(Sym::from(name), val);
}

/// `define` for a name the caller already holds as a `Sym` — no allocation.
pub fn define_sym(env: &Env, name: &Sym, val: Value) {
    env.borrow_mut().bind(name.clone(), val);
}

/// Look a name up by its text. A name that was never interned can't be bound.
pub fn get(env: &Env, name: &str) -> Result<Value, LispError> {
    match Sym::existing(name) {
        Some(sym) => get_sym(env, &sym),
        None => Err(LispError::UndefinedSymbol(name.to_string())),
    }
}

/// Walks the chain by reference — no `Rc` clone and no borrow guard per level, and each level
/// compares pointers (symbols are interned). This is the lookup every evaluated symbol makes.
pub fn get_sym(env: &Env, name: &Sym) -> Result<Value, LispError> {
    let mut cur: &RefCell<Scope> = env;
    loop {
        // SAFETY: the reference lives only for this iteration and no `borrow_mut` of any scope
        // can start while `get_sym` runs (it calls nothing that could); `try_borrow_unguarded`
        // refuses if one is already active.
        let scope = unsafe { cur.try_borrow_unguarded() }
            .map_err(|_| LispError::Runtime(format!("{name}: its scope is being changed")))?;
        if let Some(v) = scope.lookup(name) {
            return Ok(v.clone());
        }
        match &scope.parent {
            Some(p) => cur = p,
            None => return Err(LispError::UndefinedSymbol(name.to_string())),
        }
    }
}

pub fn set(env: &Env, name: &str, val: Value) -> Result<(), LispError> {
    let Some(name) = Sym::existing(name) else {
        return Err(LispError::UndefinedSymbol(name.to_string()));
    };
    let mut cur = env.clone();
    loop {
        let next = {
            let mut scope = cur.borrow_mut();
            if let Some(slot) = scope.lookup_mut(&name) {
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
