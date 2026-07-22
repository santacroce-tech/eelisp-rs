//! Evaluator with tail-call optimization (ANALYSIS §4.3–4.5, fixing the "no TCO" bug).
//!
//! The outer `loop` is a trampoline: tail positions (the taken branch of `if`, the last form
//! of `do`/`let`/`cond`/`and`/`or`, and a function-application body's last form) reassign
//! `expr`/`env` and `continue` instead of recursing — so tail-recursive Lisp runs in O(1) stack.

use std::rc::Rc;

use crate::env::{self, Env};
use crate::value::*;

pub fn eval(mut expr: Value, mut env: Env) -> Result<Value, LispError> {
    loop {
        match expr {
            Value::Symbol(s) => return env::get(&env, &s),
            Value::Dict(d) => {
                // dict literals evaluate each value (keys unchanged)
                let mut nd = OrderedDict::default();
                for k in d.keys.iter() {
                    nd.insert(k.clone(), eval(d.map[k].clone(), env.clone())?);
                }
                return Ok(Value::Dict(Rc::new(nd)));
            }
            Value::List(items) => {
                if items.is_empty() {
                    return Ok(Value::List(items));
                }

                // ---- special forms (head is a bare symbol) ----
                if let Value::Symbol(head) = &items[0] {
                    match head.as_str() {
                        "quote" => return Ok(items.get(1).cloned().unwrap_or(Value::Null)),
                        "quasiquote" => {
                            return quasiquote(items.get(1).unwrap_or(&Value::Null), &env)
                        }
                        "if" => {
                            let c = eval(items[1].clone(), env.clone())?;
                            expr = if is_truthy(&c) {
                                items[2].clone()
                            } else {
                                items.get(3).cloned().unwrap_or(Value::Null)
                            };
                            continue;
                        }
                        "do" | "begin" => {
                            if items.len() == 1 {
                                return Ok(Value::Null);
                            }
                            for e in &items[1..items.len() - 1] {
                                eval(e.clone(), env.clone())?;
                            }
                            expr = items[items.len() - 1].clone();
                            continue;
                        }
                        "def" => return eval_def(&items, &env),
                        "defn" => return eval_defn(&items, &env),
                        "fn" | "lambda" => return eval_fn(&items, &env),
                        "defmacro" => return eval_defmacro(&items, &env),
                        "set!" => {
                            let v = eval(items[2].clone(), env.clone())?;
                            return match &items[1] {
                                Value::Symbol(name) => {
                                    env::set(&env, name, v.clone())?;
                                    Ok(v)
                                }
                                _ => Err(LispError::InvalidSyntax("set! expects a symbol".into())),
                            };
                        }
                        "let" => {
                            let scope = env::child(&env);
                            if let Value::List(binds) = &items[1] {
                                for b in binds.iter() {
                                    if let Value::List(pair) = b {
                                        if let Value::Symbol(name) = &pair[0] {
                                            let v = eval(pair[1].clone(), scope.clone())?;
                                            env::define(&scope, name, v);
                                        }
                                    }
                                }
                            }
                            if items.len() == 2 {
                                return Ok(Value::Null);
                            }
                            for e in &items[2..items.len() - 1] {
                                eval(e.clone(), scope.clone())?;
                            }
                            expr = items[items.len() - 1].clone();
                            env = scope;
                            continue;
                        }
                        "cond" => {
                            let mut chosen: Option<Value> = None;
                            for clause in &items[1..] {
                                if let Value::List(cl) = clause {
                                    if cl.is_empty() {
                                        continue;
                                    }
                                    let is_else =
                                        matches!(&cl[0], Value::Symbol(s) if s == "else");
                                    let test = if is_else {
                                        Value::Bool(true)
                                    } else {
                                        eval(cl[0].clone(), env.clone())?
                                    };
                                    if is_truthy(&test) {
                                        if cl.len() == 1 {
                                            return Ok(test);
                                        }
                                        for e in &cl[1..cl.len() - 1] {
                                            eval(e.clone(), env.clone())?;
                                        }
                                        chosen = Some(cl[cl.len() - 1].clone());
                                        break;
                                    }
                                }
                            }
                            match chosen {
                                Some(e) => {
                                    expr = e;
                                    continue;
                                }
                                None => return Ok(Value::Null),
                            }
                        }
                        "and" => {
                            if items.len() == 1 {
                                return Ok(Value::Bool(true));
                            }
                            for e in &items[1..items.len() - 1] {
                                let v = eval(e.clone(), env.clone())?;
                                if !is_truthy(&v) {
                                    return Ok(v);
                                }
                            }
                            expr = items[items.len() - 1].clone();
                            continue;
                        }
                        "or" => {
                            if items.len() == 1 {
                                return Ok(Value::Bool(false));
                            }
                            for e in &items[1..items.len() - 1] {
                                let v = eval(e.clone(), env.clone())?;
                                if is_truthy(&v) {
                                    return Ok(v);
                                }
                            }
                            expr = items[items.len() - 1].clone();
                            continue;
                        }
                        "for-each" => {
                            if let Value::Symbol(var) = &items[1] {
                                let lst = eval(items[2].clone(), env.clone())?;
                                if let Value::List(l) = lst {
                                    for item in l.iter() {
                                        let scope = env::child(&env);
                                        env::define(&scope, var, item.clone());
                                        for e in &items[3..] {
                                            eval(e.clone(), scope.clone())?;
                                        }
                                    }
                                }
                            }
                            return Ok(Value::Null);
                        }
                        _ => {}
                    }
                }

                // ---- application ----
                let head_val = eval(items[0].clone(), env.clone())?;
                if let Value::Macro(m) = &head_val {
                    expr = expand_macro(m, &items[1..])?;
                    continue;
                }
                match head_val {
                    Value::Builtin(b) => {
                        let args = eval_builtin_args(&b.arg_mode, &items, &env)?;
                        return (b.func)(&args, &env);
                    }
                    Value::Function(f) => {
                        let mut args = Vec::with_capacity(items.len().saturating_sub(1));
                        for a in &items[1..] {
                            args.push(eval(a.clone(), env.clone())?);
                        }
                        let scope = bind_params(&f, &args)?;
                        if f.body.is_empty() {
                            return Ok(Value::Null);
                        }
                        for e in &f.body[..f.body.len() - 1] {
                            eval(e.clone(), scope.clone())?;
                        }
                        expr = f.body[f.body.len() - 1].clone();
                        env = scope;
                        continue;
                    }
                    other => {
                        return Err(LispError::TypeMismatch {
                            expected: "function".into(),
                            got: type_name(&other),
                        })
                    }
                }
            }
            other => return Ok(other), // self-evaluating
        }
    }
}

/// Evaluate a builtin call's arguments according to its `ArgMode` (dBASE selective evaluation).
fn eval_builtin_args(mode: &ArgMode, items: &[Value], env: &Env) -> Result<Vec<Value>, LispError> {
    match mode {
        ArgMode::Eval => {
            let mut a = Vec::with_capacity(items.len().saturating_sub(1));
            for x in &items[1..] {
                a.push(eval(x.clone(), env.clone())?);
            }
            Ok(a)
        }
        ArgMode::TableFirst => {
            // arg 0 (table name) is passed raw; the rest evaluate normally.
            let mut a = Vec::with_capacity(items.len().saturating_sub(1));
            if items.len() > 1 {
                a.push(items[1].clone());
            }
            for x in items.iter().skip(2) {
                a.push(eval(x.clone(), env.clone())?);
            }
            Ok(a)
        }
        ArgMode::AllRaw => Ok(items[1..].to_vec()),
    }
}

/// Non-tail application, used by builtins (map/filter/reduce/apply).
pub fn apply_value(f: &Value, args: &[Value], env: &Env) -> Result<Value, LispError> {
    match f {
        Value::Builtin(b) => (b.func)(args, env),
        Value::Function(func) => {
            let scope = bind_params(func, args)?;
            let mut result = Value::Null;
            for e in &func.body {
                result = eval(e.clone(), scope.clone())?;
            }
            Ok(result)
        }
        other => Err(LispError::TypeMismatch {
            expected: "function".into(),
            got: type_name(other),
        }),
    }
}

fn bind_params(f: &Function, args: &[Value]) -> Result<Env, LispError> {
    let scope = env::child(&f.closure);
    for (i, p) in f.params.iter().enumerate() {
        env::define(&scope, p, args.get(i).cloned().unwrap_or(Value::Null));
    }
    if let Some(rest) = &f.rest {
        let extra = if args.len() > f.params.len() {
            args[f.params.len()..].to_vec()
        } else {
            vec![]
        };
        env::define(&scope, rest, Value::List(Rc::new(extra)));
    }
    Ok(scope)
}

fn expand_macro(m: &Macro, args: &[Value]) -> Result<Value, LispError> {
    let scope = env::child(&m.closure);
    for (i, p) in m.params.iter().enumerate() {
        env::define(&scope, p, args.get(i).cloned().unwrap_or(Value::Null));
    }
    if let Some(rest) = &m.rest {
        let extra = if args.len() > m.params.len() {
            args[m.params.len()..].to_vec()
        } else {
            vec![]
        };
        env::define(&scope, rest, Value::List(Rc::new(extra)));
    }
    let mut result = Value::Null;
    for e in &m.body {
        result = eval(e.clone(), scope.clone())?;
    }
    Ok(result)
}

/// Parse a param spec into (positional, rest) — `(a b . rest)`. Shared by fn/defn/defmacro,
/// which is why macro rest params work here (the Swift version's bug is fixed).
fn parse_params(spec: &[Value]) -> Result<(Vec<Symbol>, Option<Symbol>), LispError> {
    let mut params = Vec::new();
    let mut rest = None;
    let mut i = 0;
    while i < spec.len() {
        match &spec[i] {
            Value::Symbol(s) if s == "." => {
                match spec.get(i + 1) {
                    Some(Value::Symbol(r)) => rest = Some(r.clone()),
                    _ => return Err(LispError::InvalidSyntax("expected symbol after . ".into())),
                }
                break;
            }
            Value::Symbol(s) => params.push(s.clone()),
            other => {
                return Err(LispError::InvalidSyntax(format!(
                    "bad parameter: {}",
                    type_name(other)
                )))
            }
        }
        i += 1;
    }
    Ok((params, rest))
}

fn eval_def(items: &[Value], env: &Env) -> Result<Value, LispError> {
    match &items[1] {
        Value::Symbol(name) => {
            let v = eval(items[2].clone(), env.clone())?;
            env::define(env, name, v.clone());
            Ok(v)
        }
        Value::List(sig) => {
            // (def (name p...) body...) shorthand
            if let Some(Value::Symbol(name)) = sig.first() {
                let (params, rest) = parse_params(&sig[1..])?;
                let f = Function {
                    name: Some(name.clone()),
                    params,
                    rest,
                    body: items[2..].to_vec(),
                    closure: env.clone(),
                };
                let val = Value::Function(Rc::new(f));
                env::define(env, name, val.clone());
                Ok(val)
            } else {
                Err(LispError::InvalidSyntax("bad def".into()))
            }
        }
        _ => Err(LispError::InvalidSyntax("bad def".into())),
    }
}

fn eval_defn(items: &[Value], env: &Env) -> Result<Value, LispError> {
    if let (Value::Symbol(name), Value::List(spec)) = (&items[1], &items[2]) {
        let (params, rest) = parse_params(spec)?;
        let f = Function {
            name: Some(name.clone()),
            params,
            rest,
            body: items[3..].to_vec(),
            closure: env.clone(),
        };
        let val = Value::Function(Rc::new(f));
        env::define(env, name, val.clone());
        Ok(val)
    } else {
        Err(LispError::InvalidSyntax("bad defn".into()))
    }
}

fn eval_fn(items: &[Value], env: &Env) -> Result<Value, LispError> {
    if let Value::List(spec) = &items[1] {
        let (params, rest) = parse_params(spec)?;
        Ok(Value::Function(Rc::new(Function {
            name: None,
            params,
            rest,
            body: items[2..].to_vec(),
            closure: env.clone(),
        })))
    } else {
        Err(LispError::InvalidSyntax("bad fn".into()))
    }
}

fn eval_defmacro(items: &[Value], env: &Env) -> Result<Value, LispError> {
    if let (Value::Symbol(name), Value::List(spec)) = (&items[1], &items[2]) {
        let (params, rest) = parse_params(spec)?;
        let m = Macro {
            name: Some(name.clone()),
            params,
            rest,
            body: items[3..].to_vec(),
            closure: env.clone(),
        };
        let val = Value::Macro(Rc::new(m));
        env::define(env, name, val.clone());
        Ok(val)
    } else {
        Err(LispError::InvalidSyntax("bad defmacro".into()))
    }
}

/// Quasiquote with unquote / unquote-splicing (depth 1). Implemented, unlike the Swift version.
fn quasiquote(expr: &Value, env: &Env) -> Result<Value, LispError> {
    match expr {
        Value::List(items) => {
            // (unquote x)
            if items.len() == 2 {
                if let Value::Symbol(s) = &items[0] {
                    if s == "unquote" {
                        return eval(items[1].clone(), env.clone());
                    }
                }
            }
            let mut out: Vec<Value> = Vec::new();
            for it in items.iter() {
                if let Value::List(inner) = it {
                    if inner.len() == 2 {
                        if let Value::Symbol(s) = &inner[0] {
                            if s == "unquote-splicing" {
                                let spliced = eval(inner[1].clone(), env.clone())?;
                                match spliced {
                                    Value::List(l) => {
                                        out.extend(l.iter().cloned());
                                        continue;
                                    }
                                    other => {
                                        return Err(LispError::TypeMismatch {
                                            expected: "list".into(),
                                            got: type_name(&other),
                                        })
                                    }
                                }
                            }
                        }
                    }
                }
                out.push(quasiquote(it, env)?);
            }
            Ok(Value::List(Rc::new(out)))
        }
        other => Ok(other.clone()),
    }
}
