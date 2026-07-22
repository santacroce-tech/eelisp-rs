//! Lexical environment — a chain of scopes (ANALYSIS §4.3). Reference-typed so closures share.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::value::{LispError, Symbol, Value};

pub struct Scope {
    pub vars: HashMap<Symbol, Value>,
    pub parent: Option<Env>,
}

pub type Env = Rc<RefCell<Scope>>;

pub fn root() -> Env {
    Rc::new(RefCell::new(Scope {
        vars: HashMap::new(),
        parent: None,
    }))
}

pub fn child(parent: &Env) -> Env {
    Rc::new(RefCell::new(Scope {
        vars: HashMap::new(),
        parent: Some(parent.clone()),
    }))
}

pub fn define(env: &Env, name: &str, val: Value) {
    env.borrow_mut().vars.insert(name.to_string(), val);
}

pub fn get(env: &Env, name: &str) -> Result<Value, LispError> {
    let mut cur = env.clone();
    loop {
        if let Some(v) = cur.borrow().vars.get(name) {
            return Ok(v.clone());
        }
        let parent = cur.borrow().parent.clone();
        match parent {
            Some(p) => cur = p,
            None => return Err(LispError::UndefinedSymbol(name.to_string())),
        }
    }
}

pub fn set(env: &Env, name: &str, val: Value) -> Result<(), LispError> {
    let mut cur = env.clone();
    loop {
        if cur.borrow().vars.contains_key(name) {
            cur.borrow_mut().vars.insert(name.to_string(), val);
            return Ok(());
        }
        let parent = cur.borrow().parent.clone();
        match parent {
            Some(p) => cur = p,
            None => return Err(LispError::UndefinedSymbol(name.to_string())),
        }
    }
}
