//! Core builtins (ANALYSIS §4.6). This is a representative subset that exercises every value
//! type and the eval callback path (map/filter/reduce/apply/eval). The full ~80-builtin
//! catalog + date/db/agenda/http builtins are TODO — register them the same way.

use std::rc::Rc;

use crate::env::{self, Env};
use crate::eval::{apply_value, eval};
use crate::value::*;

pub fn register(env: &Env) {
    macro_rules! b {
        ($name:expr, $f:expr) => {
            env::define(
                env,
                $name,
                Value::Builtin(Rc::new(Builtin {
                    name: $name.to_string(),
                    arg_mode: ArgMode::Eval,
                    func: Box::new($f),
                })),
            );
        };
    }

    // arithmetic
    b!("+", add);
    b!("-", sub);
    b!("*", mul);
    b!("/", div);
    b!("mod", modulo);
    b!("abs", absf);
    b!("min", minf);
    b!("max", maxf);
    b!("floor", floorf);
    b!("ceil", ceilf);
    b!("round", roundf);
    b!("pow", powf);
    // comparison / logic
    b!("=", eqf);
    b!("!=", nef);
    b!("<", ltf);
    b!(">", gtf);
    b!("<=", lef);
    b!(">=", gef);
    b!("not", notf);
    // strings
    b!("str", strf);
    b!("str-len", str_len);
    b!("str-upper", str_upper);
    b!("str-lower", str_lower);
    b!("str-split", str_split);
    b!("str-join", str_join);
    b!("str-contains", str_contains);
    b!("str-matches", str_matches);
    b!("str-trim", str_trim);
    // lists
    b!("list", listf);
    b!("cons", cons);
    b!("car", car);
    b!("head", car);
    b!("cdr", cdr);
    b!("tail", cdr);
    b!("nth", nth);
    b!("length", length);
    b!("append", append);
    b!("reverse", reversef);
    b!("range", rangef);
    b!("map", mapf);
    b!("filter", filterf);
    b!("reduce", reducef);
    b!("empty?", emptyf);
    // dicts
    b!("dict", dictf);
    b!("dict-get", dict_get);
    b!("dict-set", dict_set);
    b!("dict-keys", dict_keys);
    b!("dict-values", dict_values);
    b!("dict-has", dict_has);
    b!("dict-merge", dict_merge);
    // types
    b!("type", typef);
    b!("number?", is_number);
    b!("string?", is_string);
    b!("bool?", is_bool);
    b!("list?", is_list);
    b!("nil?", is_nil);
    b!("symbol?", is_symbol);
    b!("keyword?", is_keyword);
    b!("fn?", is_fn);
    // io / meta
    b!("print", printf);
    b!("println", printlnf);
    b!("eval", evalf);
    b!("apply", applyf);
    b!("json-parse", json_parse);
    b!("json-stringify", json_stringify);
    b!("http-get", http_get);
    b!("http-post", http_post);
}

// ---- helpers ----

fn as_num(v: &Value) -> Result<f64, LispError> {
    match v {
        Value::Number(n) => Ok(*n),
        _ => Err(LispError::TypeMismatch {
            expected: "number".into(),
            got: type_name(v),
        }),
    }
}
fn as_list(v: &Value) -> Result<Rc<Vec<Value>>, LispError> {
    match v {
        Value::List(l) => Ok(l.clone()),
        _ => Err(LispError::TypeMismatch {
            expected: "list".into(),
            got: type_name(v),
        }),
    }
}
fn list_val(v: Vec<Value>) -> Value {
    Value::List(Rc::new(v))
}
fn display(v: &Value) -> String {
    crate::printer::print_value(v, false)
}

// ---- arithmetic ----

fn add(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut s = 0.0;
    for a in args {
        s += as_num(a)?;
    }
    Ok(Value::Number(s))
}
fn sub(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if args.is_empty() {
        return Err(LispError::Arity {
            func: "-".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    let first = as_num(&args[0])?;
    if args.len() == 1 {
        return Ok(Value::Number(-first));
    }
    let mut s = 0.0;
    for a in &args[1..] {
        s += as_num(a)?;
    }
    Ok(Value::Number(first - s))
}
fn mul(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut p = 1.0;
    for a in args {
        p *= as_num(a)?;
    }
    Ok(Value::Number(p))
}
fn div(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if args.is_empty() {
        return Err(LispError::Arity {
            func: "/".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    let mut acc = as_num(&args[0])?;
    for a in &args[1..] {
        let d = as_num(a)?;
        if d == 0.0 {
            return Err(LispError::DivisionByZero);
        }
        acc /= d;
    }
    Ok(Value::Number(acc))
}
fn modulo(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let a = as_num(&args[0])?;
    let b = as_num(&args[1])?;
    if b == 0.0 {
        return Err(LispError::DivisionByZero);
    }
    Ok(Value::Number(a % b)) // truncated remainder, sign follows dividend
}
fn absf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Number(as_num(&args[0])?.abs()))
}
fn minf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut m = as_num(&args[0])?;
    for a in &args[1..] {
        m = m.min(as_num(a)?);
    }
    Ok(Value::Number(m))
}
fn maxf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut m = as_num(&args[0])?;
    for a in &args[1..] {
        m = m.max(as_num(a)?);
    }
    Ok(Value::Number(m))
}
fn floorf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Number(as_num(&args[0])?.floor()))
}
fn ceilf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Number(as_num(&args[0])?.ceil()))
}
fn roundf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let n = as_num(&args[0])?;
    let places = args.get(1).map(as_num).transpose()?.unwrap_or(0.0);
    let f = 10f64.powf(places);
    Ok(Value::Number((n * f).round() / f))
}
fn powf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Number(as_num(&args[0])?.powf(as_num(&args[1])?)))
}

// ---- comparison / logic ----

fn eqf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if args.len() < 2 {
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(args[1..].iter().all(|a| a == &args[0])))
}
fn nef(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(!(args.len() >= 2 && args[0] == args[1])))
}
fn ltf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    for w in args.windows(2) {
        if !(as_num(&w[0])? < as_num(&w[1])?) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}
fn gtf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    for w in args.windows(2) {
        if !(as_num(&w[0])? > as_num(&w[1])?) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}
fn lef(args: &[Value], _: &Env) -> Result<Value, LispError> {
    for w in args.windows(2) {
        if !(as_num(&w[0])? <= as_num(&w[1])?) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}
fn gef(args: &[Value], _: &Env) -> Result<Value, LispError> {
    for w in args.windows(2) {
        if !(as_num(&w[0])? >= as_num(&w[1])?) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}
fn notf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(!is_truthy(&args[0])))
}

// ---- strings ----

fn strf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Str(args.iter().map(display).collect()))
}
fn str_len(args: &[Value], _: &Env) -> Result<Value, LispError> {
    match &args[0] {
        Value::Str(s) => Ok(Value::Number(s.chars().count() as f64)),
        other => Err(LispError::TypeMismatch {
            expected: "string".into(),
            got: type_name(other),
        }),
    }
}
fn str_upper(args: &[Value], _: &Env) -> Result<Value, LispError> {
    match &args[0] {
        Value::Str(s) => Ok(Value::Str(s.to_uppercase())),
        other => Err(LispError::TypeMismatch {
            expected: "string".into(),
            got: type_name(other),
        }),
    }
}
fn str_lower(args: &[Value], _: &Env) -> Result<Value, LispError> {
    match &args[0] {
        Value::Str(s) => Ok(Value::Str(s.to_lowercase())),
        other => Err(LispError::TypeMismatch {
            expected: "string".into(),
            got: type_name(other),
        }),
    }
}
fn str_split(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if let (Value::Str(s), Value::Str(sep)) = (&args[0], &args[1]) {
        let parts = s.split(sep.as_str()).map(|p| Value::Str(p.to_string())).collect();
        Ok(list_val(parts))
    } else {
        Err(LispError::InvalidSyntax("str-split expects (string string)".into()))
    }
}
fn str_join(args: &[Value], _: &Env) -> Result<Value, LispError> {
    // (str-join sep list) — separator first, matching EELisp
    if let (Value::Str(sep), Value::List(l)) = (&args[0], &args[1]) {
        let parts: Vec<String> = l.iter().map(display).collect();
        Ok(Value::Str(parts.join(sep)))
    } else {
        Err(LispError::InvalidSyntax("str-join expects (string list)".into()))
    }
}
fn str_contains(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if let (Value::Str(s), Value::Str(sub)) = (&args[0], &args[1]) {
        Ok(Value::Bool(s.contains(sub.as_str())))
    } else {
        Err(LispError::InvalidSyntax("str-contains expects (string string)".into()))
    }
}
fn str_trim(args: &[Value], _: &Env) -> Result<Value, LispError> {
    match &args[0] {
        Value::Str(s) => Ok(Value::Str(s.trim().to_string())),
        other => Err(LispError::TypeMismatch {
            expected: "string".into(),
            got: type_name(other),
        }),
    }
}
/// (str-matches string pattern) → (true <group0> <group1> …) on a match, else false.
/// Unmatched optional groups are nil. `(match n)` in rule/view context reads group n from this.
fn str_matches(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let (s, pat) = match (args.first(), args.get(1)) {
        (Some(Value::Str(s)), Some(Value::Str(p))) => (s, p),
        _ => return Err(LispError::InvalidSyntax("str-matches expects (string string)".into())),
    };
    let re = regex::Regex::new(pat).map_err(|e| LispError::Runtime(format!("bad regex: {}", e)))?;
    match re.captures(s) {
        Some(caps) => {
            let mut out = vec![Value::Bool(true)];
            for i in 0..caps.len() {
                out.push(caps.get(i).map(|m| Value::Str(m.as_str().to_string())).unwrap_or(Value::Null));
            }
            Ok(list_val(out))
        }
        None => Ok(Value::Bool(false)),
    }
}

// ---- lists ----

fn listf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(list_val(args.to_vec()))
}
fn cons(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let head = args[0].clone();
    match &args[1] {
        Value::List(l) => {
            let mut v = vec![head];
            v.extend(l.iter().cloned());
            Ok(list_val(v))
        }
        other => Ok(list_val(vec![head, other.clone()])),
    }
}
fn car(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let l = as_list(&args[0])?;
    Ok(l.first().cloned().unwrap_or(Value::Null))
}
fn cdr(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let l = as_list(&args[0])?;
    Ok(list_val(l.iter().skip(1).cloned().collect()))
}
fn nth(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let l = as_list(&args[0])?;
    let i = as_num(&args[1])? as usize;
    Ok(l.get(i).cloned().unwrap_or(Value::Null))
}
fn length(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let n = match &args[0] {
        Value::List(l) => l.len(),
        Value::Str(s) => s.chars().count(),
        Value::Dict(d) => d.keys.len(),
        other => {
            return Err(LispError::TypeMismatch {
                expected: "list|string|dict".into(),
                got: type_name(other),
            })
        }
    };
    Ok(Value::Number(n as f64))
}
fn append(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut out = Vec::new();
    for a in args {
        out.extend(as_list(a)?.iter().cloned());
    }
    Ok(list_val(out))
}
fn reversef(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut v: Vec<Value> = as_list(&args[0])?.iter().cloned().collect();
    v.reverse();
    Ok(list_val(v))
}
fn rangef(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let (start, end, step) = match args.len() {
        1 => (0.0, as_num(&args[0])?, 1.0),
        2 => (as_num(&args[0])?, as_num(&args[1])?, 1.0),
        _ => (as_num(&args[0])?, as_num(&args[1])?, as_num(&args[2])?),
    };
    if step == 0.0 {
        return Err(LispError::InvalidSyntax("range step cannot be 0".into()));
    }
    let mut out = Vec::new();
    let mut x = start;
    if step > 0.0 {
        while x < end {
            out.push(Value::Number(x));
            x += step;
        }
    } else {
        while x > end {
            out.push(Value::Number(x));
            x += step;
        }
    }
    Ok(list_val(out))
}
fn mapf(args: &[Value], env: &Env) -> Result<Value, LispError> {
    let l = as_list(&args[1])?;
    let mut out = Vec::with_capacity(l.len());
    for item in l.iter() {
        out.push(apply_value(&args[0], &[item.clone()], env)?);
    }
    Ok(list_val(out))
}
fn filterf(args: &[Value], env: &Env) -> Result<Value, LispError> {
    let l = as_list(&args[1])?;
    let mut out = Vec::new();
    for item in l.iter() {
        if is_truthy(&apply_value(&args[0], &[item.clone()], env)?) {
            out.push(item.clone());
        }
    }
    Ok(list_val(out))
}
fn reducef(args: &[Value], env: &Env) -> Result<Value, LispError> {
    // (reduce fn init list)
    let mut acc = args[1].clone();
    let l = as_list(&args[2])?;
    for item in l.iter() {
        acc = apply_value(&args[0], &[acc, item.clone()], env)?;
    }
    Ok(acc)
}
fn emptyf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let e = match &args[0] {
        Value::List(l) => l.is_empty(),
        Value::Str(s) => s.is_empty(),
        Value::Dict(d) => d.keys.is_empty(),
        Value::Null => true,
        _ => false,
    };
    Ok(Value::Bool(e))
}

// ---- dicts ----

fn dictf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut d = OrderedDict::default();
    let mut i = 0;
    while i + 1 < args.len() {
        if let Value::Keyword(k) = &args[i] {
            d.insert(k.clone(), args[i + 1].clone());
        } else {
            return Err(LispError::InvalidSyntax("dict keys must be keywords".into()));
        }
        i += 2;
    }
    Ok(Value::Dict(Rc::new(d)))
}
fn dict_get(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if let (Value::Dict(d), Value::Keyword(k)) = (&args[0], &args[1]) {
        Ok(d.get(k).cloned().unwrap_or_else(|| args.get(2).cloned().unwrap_or(Value::Null)))
    } else {
        Err(LispError::InvalidSyntax("dict-get expects (dict keyword)".into()))
    }
}
fn dict_set(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if let (Value::Dict(d), Value::Keyword(k)) = (&args[0], &args[1]) {
        let mut nd = (**d).clone();
        nd.insert(k.clone(), args[2].clone());
        Ok(Value::Dict(Rc::new(nd)))
    } else {
        Err(LispError::InvalidSyntax("dict-set expects (dict keyword value)".into()))
    }
}
fn dict_keys(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if let Value::Dict(d) = &args[0] {
        Ok(list_val(d.keys.iter().map(|k| Value::Keyword(k.clone())).collect()))
    } else {
        Err(LispError::TypeMismatch {
            expected: "dict".into(),
            got: type_name(&args[0]),
        })
    }
}
fn dict_values(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if let Value::Dict(d) = &args[0] {
        Ok(list_val(d.keys.iter().map(|k| d.map[k].clone()).collect()))
    } else {
        Err(LispError::TypeMismatch {
            expected: "dict".into(),
            got: type_name(&args[0]),
        })
    }
}
fn dict_has(args: &[Value], _: &Env) -> Result<Value, LispError> {
    if let (Value::Dict(d), Value::Keyword(k)) = (&args[0], &args[1]) {
        Ok(Value::Bool(d.get(k).is_some()))
    } else {
        Err(LispError::InvalidSyntax("dict-has expects (dict keyword)".into()))
    }
}
fn dict_merge(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let mut out = OrderedDict::default();
    for a in args {
        match a {
            Value::Dict(d) => {
                for k in &d.keys {
                    out.insert(k.clone(), d.map[k].clone());
                }
            }
            other => {
                return Err(LispError::TypeMismatch {
                    expected: "dict".into(),
                    got: type_name(other),
                })
            }
        }
    }
    Ok(Value::Dict(Rc::new(out)))
}

// ---- types ----

fn typef(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Str(type_name(&args[0])))
}
fn is_number(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::Number(_))))
}
fn is_string(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::Str(_))))
}
fn is_bool(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::Bool(_))))
}
fn is_list(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::List(_))))
}
fn is_nil(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::Null)))
}
fn is_symbol(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::Symbol(_))))
}
fn is_keyword(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::Keyword(_))))
}
fn is_fn(a: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Bool(matches!(a[0], Value::Function(_) | Value::Builtin(_))))
}

// ---- io / meta ----

fn printf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let s: Vec<String> = args.iter().map(display).collect();
    print!("{}", s.join(" "));
    Ok(Value::Null)
}
fn printlnf(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let s: Vec<String> = args.iter().map(display).collect();
    println!("{}", s.join(" "));
    Ok(Value::Null)
}
fn evalf(args: &[Value], env: &Env) -> Result<Value, LispError> {
    eval(args[0].clone(), env.clone())
}
fn applyf(args: &[Value], env: &Env) -> Result<Value, LispError> {
    let list = as_list(&args[1])?;
    apply_value(&args[0], &list, env)
}

// ── JSON (ANALYSIS §4.11) — plain JSON ⇄ Value; objects↔dicts, arrays↔lists ──

fn json_parse(args: &[Value], _: &Env) -> Result<Value, LispError> {
    match args.first() {
        Some(Value::Str(s)) => {
            let j: serde_json::Value =
                serde_json::from_str(s).map_err(|e| LispError::Runtime(format!("Invalid JSON: {}", e)))?;
            Ok(crate::host::from_json(&j))
        }
        _ => Err(LispError::InvalidSyntax("json-parse expects a string".into())),
    }
}

fn json_stringify(args: &[Value], _: &Env) -> Result<Value, LispError> {
    Ok(Value::Str(value_to_natural(args.first().unwrap_or(&Value::Null)).to_string()))
}

// ── HTTP (ANALYSIS §4.11) — synchronous, returns {:status N :body "…"} ──

fn http_dict(status: u16, body: String) -> Value {
    let mut d = OrderedDict::default();
    d.insert("status".into(), Value::Number(status as f64));
    d.insert("body".into(), Value::Str(body));
    Value::Dict(Rc::new(d))
}

fn http_result(r: Result<ureq::Response, ureq::Error>) -> Result<Value, LispError> {
    match r {
        Ok(resp) => {
            let status = resp.status();
            Ok(http_dict(status, resp.into_string().unwrap_or_default()))
        }
        // a 4xx/5xx is still a response — surface its status + body, matching EELisp
        Err(ureq::Error::Status(code, resp)) => {
            Ok(http_dict(code, resp.into_string().unwrap_or_default()))
        }
        Err(e) => Err(LispError::Runtime(format!("HTTP request failed: {}", e))),
    }
}

fn http_get(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let url = match args.first() {
        Some(Value::Str(u)) => u,
        _ => return Err(LispError::InvalidSyntax("http-get expects a URL string".into())),
    };
    http_result(ureq::get(url).timeout(std::time::Duration::from_secs(30)).call())
}

fn http_post(args: &[Value], _: &Env) -> Result<Value, LispError> {
    let url = match args.first() {
        Some(Value::Str(u)) => u,
        _ => return Err(LispError::InvalidSyntax("http-post expects a URL string".into())),
    };
    let body = match args.get(1) {
        Some(Value::Str(b)) => b.clone(),
        _ => String::new(),
    };
    // optional :content-type keyword
    let mut content_type = "application/json".to_string();
    let mut i = 2;
    while i + 1 < args.len() {
        if let Value::Keyword(k) = &args[i] {
            if k == "content-type" {
                if let Value::Str(ct) = &args[i + 1] {
                    content_type = ct.clone();
                }
            }
        }
        i += 1;
    }
    http_result(
        ureq::post(url)
            .timeout(std::time::Duration::from_secs(30))
            .set("Content-Type", &content_type)
            .send_string(&body),
    )
}

fn value_to_natural(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                serde_json::json!(*n as i64)
            } else {
                serde_json::json!(n)
            }
        }
        Value::Str(s) => J::String(s.clone()),
        Value::Bool(b) => J::Bool(*b),
        Value::Null => J::Null,
        Value::Keyword(k) | Value::Symbol(k) => J::String(k.clone()),
        Value::List(l) => J::Array(l.iter().map(value_to_natural).collect()),
        Value::Dict(d) => {
            let mut m = serde_json::Map::new(); // BTreeMap → sorted keys, matching EELisp
            for k in &d.keys {
                m.insert(k.clone(), value_to_natural(&d.map[k]));
            }
            J::Object(m)
        }
        other => J::String(crate::printer::print_value(other, false)),
    }
}
