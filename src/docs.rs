//! `(functions …)` and `(source …)` — the language documenting itself.
//!
//! Two halves. **User code** carries its own documentation: every top-level `def`/`defn`/`defmacro`
//! read through the parser is remembered with the text it was written in, comment block and all
//! (`SourceIndex`), so `(source f)` can echo a definition back exactly as the author wrote it.
//! **Builtins** have no EELisp source to echo, so the tables at the bottom of this file are their
//! source: a signature, one line of prose, and a worked example for each.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::rc::Rc;

use crate::env::{self, Env};
use crate::output::OutputState;
use crate::parser::TopForm;
use crate::printer::print_value;
use crate::value::*;

// ── the manual ───────────────────────────────────────────────────────

/// One entry of the built-in manual.
pub struct Entry {
    pub name: &'static str,
    /// How it is called, return type included: `(str-len s) → number`.
    pub sig: &'static str,
    /// One line of prose. No trailing period needed.
    pub summary: &'static str,
    /// A worked example, or `""` where one would say nothing.
    pub example: &'static str,
}

/// The manual, in reading order: core language, database, agenda, sheets, editor RPC.
pub static TABLES: &[&[Entry]] = &[CORE, DATABASE, AGENDA, SHEETS, EDITOR];

pub fn builtins() -> impl Iterator<Item = &'static Entry> {
    TABLES.iter().flat_map(|t| t.iter())
}

pub fn builtin_entry(name: &str) -> Option<&'static Entry> {
    builtins().find(|e| e.name == name)
}

pub fn special_form_entry(name: &str) -> Option<&'static Entry> {
    SPECIAL_FORMS.iter().find(|e| e.name == name)
}

// ── remembering how user code was written ────────────────────────────

/// A definition as it appeared in the source text.
#[derive(Clone)]
pub struct Definition {
    /// The run of `;` comment lines directly above the form. Empty if it had none.
    pub comments: String,
    /// The form itself, verbatim.
    pub source: String,
}

/// Name → the text it was defined in. Later definitions of a name replace earlier ones, which is
/// what a REPL user redefining a function expects to see.
#[derive(Default)]
pub struct SourceIndex {
    defs: HashMap<String, Definition>,
}

impl SourceIndex {
    /// Record a top-level form if it defines a name. Everything else is ignored — this is a
    /// documentation index, not a history of evaluation.
    pub fn record(&mut self, form: &TopForm) {
        if let Some(name) = defined_name(&form.value) {
            self.defs.insert(
                name,
                Definition { comments: form.comments.clone(), source: form.source.clone() },
            );
        }
    }

    pub fn get(&self, name: &str) -> Option<&Definition> {
        self.defs.get(name)
    }
}

/// The name a top-level form binds, if it binds one: `(defn f …)`, `(defun f …)`,
/// `(defmacro f …)`, `(def f …)` and the `(def (f a b) …)` shorthand.
fn defined_name(form: &Value) -> Option<String> {
    let items = match form {
        Value::List(l) => l,
        _ => return None,
    };
    let head = match items.first() {
        Some(Value::Symbol(s)) => s.as_str(),
        _ => return None,
    };
    if !matches!(head, "def" | "defn" | "defun" | "defmacro") {
        return None;
    }
    match items.get(1) {
        Some(Value::Symbol(name)) => Some(name.clone()),
        // (def (name params…) body…)
        Some(Value::List(sig)) => match sig.first() {
            Some(Value::Symbol(name)) => Some(name.clone()),
            _ => None,
        },
        _ => None,
    }
}

// ── shared formatting ────────────────────────────────────────────────

fn kind_of(v: &Value) -> Option<&'static str> {
    match v {
        Value::Builtin(_) => Some("builtin"),
        Value::Function(_) => Some("function"),
        Value::Macro(_) => Some("macro"),
        _ => None,
    }
}

/// `(name a b . rest)` — a parameter list read back as a call.
fn params_sig(name: &str, params: &[Symbol], rest: &Option<Symbol>) -> String {
    let mut parts = vec![name.to_string()];
    parts.extend(params.iter().cloned());
    if let Some(r) = rest {
        parts.push(".".to_string());
        parts.push(r.clone());
    }
    format!("({})", parts.join(" "))
}

/// The call shape shown in a `(functions)` listing: the manual's signature for a builtin, the
/// real parameter list for anything defined in EELisp.
fn signature(name: &str, v: &Value) -> String {
    match v {
        Value::Builtin(_) => builtin_entry(name)
            .map(|e| e.sig.to_string())
            .unwrap_or_else(|| format!("({} …)", name)),
        Value::Function(f) => params_sig(name, &f.params, &f.rest),
        Value::Macro(m) => params_sig(name, &m.params, &m.rest),
        _ => String::new(),
    }
}

/// Every name visible from `env`, innermost binding winning — the same lookup order `get` uses.
fn visible_bindings(env: &Env) -> BTreeMap<String, Value> {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    let mut cur = Some(env.clone());
    while let Some(scope) = cur {
        let parent = {
            let s = scope.borrow();
            for (k, v) in s.vars.iter() {
                out.entry(k.clone()).or_insert_with(|| v.clone());
            }
            s.parent.clone()
        };
        cur = parent;
    }
    out
}

/// The filter argument of `(functions …)`: a string, symbol or keyword, all read as plain text.
fn filter_text(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::Str(s)) | Some(Value::Symbol(s)) | Some(Value::Keyword(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_lowercase())
            }
        }
        Some(other) => Some(print_value(other, false).to_lowercase()),
    }
}

/// The name argument of `(source …)`. The first argument arrives unevaluated (ArgMode::TableFirst),
/// so a bare `(source map)` works; `'map` and `"map"` are accepted too.
fn name_arg(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Symbol(s)) | Some(Value::Str(s)) | Some(Value::Keyword(s)) => Some(s.clone()),
        Some(Value::List(l)) => match (l.first(), l.get(1)) {
            // (quote x) — what 'x reads as
            (Some(Value::Symbol(q)), Some(Value::Symbol(s))) if q == "quote" => Some(s.clone()),
            (Some(Value::Symbol(q)), Some(Value::Str(s))) if q == "quote" => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Indent a block by two spaces, leaving blank lines blank.
fn indent(block: &str) -> String {
    block
        .lines()
        .map(|l| if l.trim().is_empty() { String::new() } else { format!("  {}", l) })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── (functions …) ────────────────────────────────────────────────────

/// One listing row.
struct Row {
    name: String,
    kind: &'static str,
    sig: String,
}

fn listing(env: &Env, filter: Option<&str>) -> (Vec<Row>, usize) {
    let matches = |n: &str| filter.map(|f| n.to_lowercase().contains(f)).unwrap_or(true);

    let mut rows: Vec<Row> = Vec::new();
    let mut total = 0usize;

    // Special forms are keywords, not bindings — the environment knows nothing about them.
    for e in SPECIAL_FORMS {
        total += 1;
        if matches(e.name) {
            rows.push(Row { name: e.name.to_string(), kind: "special", sig: e.sig.to_string() });
        }
    }
    for (name, v) in visible_bindings(env) {
        if let Some(kind) = kind_of(&v) {
            total += 1;
            if matches(&name) {
                let sig = signature(&name, &v);
                rows.push(Row { name, kind, sig });
            }
        }
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    (rows, total)
}

/// The listing is grouped by kind rather than laid out in three flat columns: the REPL panel is a
/// narrow one, and a `name · kind · signature` row wraps in it, which puts the tail of one row at
/// the left margin where the next name should be. Grouping moves the kind into a heading, leaving
/// two columns that fit.
fn render_listing(rows: &[Row], total: usize, filter: Option<&str>) -> String {
    if rows.is_empty() {
        return format!(
            "no function matching {:?} — (functions) lists all {}\n",
            filter.unwrap_or(""),
            total
        );
    }

    let mut out = String::new();
    // From the language outwards: the forms it evaluates itself, then what it ships, then yours.
    for (kind, heading) in KIND_ORDER {
        let group: Vec<&Row> = rows.iter().filter(|r| r.kind == *kind).collect();
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(heading);
        out.push('\n');
        // Wide enough for the names in this group; one long outlier widens only its own row.
        let width = group.iter().map(|r| r.name.chars().count()).max().unwrap_or(0).min(20);
        for r in group {
            out.push_str(&format!("  {:<w$}  {}\n", r.name, r.sig, w = width));
        }
    }
    out.push_str(&match filter {
        Some(f) => format!("\n— {} of {} functions matching {:?}\n", rows.len(), total, f),
        None => format!("\n— {} functions\n", total),
    });
    out
}

/// The groups a listing is printed in, in order.
const KIND_ORDER: &[(&str, &str)] = &[
    ("special", "special forms"),
    ("builtin", "builtins"),
    ("macro", "macros"),
    ("function", "functions"),
];

// ── (source …) ───────────────────────────────────────────────────────

/// A definition rebuilt from the live value, for functions the parser never saw in text —
/// anything defined inside another form, or handed to `def` as an `fn`.
fn reconstruct(name: &str, v: &Value) -> Option<String> {
    let (head, params, rest, body) = match v {
        Value::Function(f) => ("defn", &f.params, &f.rest, &f.body),
        Value::Macro(m) => ("defmacro", &m.params, &m.rest, &m.body),
        _ => return None,
    };
    let mut spec: Vec<String> = params.to_vec();
    if let Some(r) = rest {
        spec.push(".".to_string());
        spec.push(r.clone());
    }
    let mut out = format!("({} {} ({})", head, name, spec.join(" "));
    for e in body {
        out.push_str("\n  ");
        out.push_str(&print_value(e, true));
    }
    out.push(')');
    Some(out)
}

fn render_source(name: &str, env: &Env, index: &SourceIndex) -> Result<String, LispError> {
    let bound = env::get(env, name).ok();

    // A function written in EELisp and read from text — echo it back as written.
    if let Some(def) = index.get(name) {
        // `def` binds values as well as functions, so fall back to the value's own type name.
        let kind = match bound.as_ref() {
            Some(v) => kind_of(v).map(str::to_string).unwrap_or_else(|| type_name(v)),
            None => "definition".to_string(),
        };
        let mut out = format!("{} — {}\n\n", name, kind);
        if !def.comments.is_empty() {
            out.push_str(&indent(&def.comments));
            out.push('\n');
        }
        out.push_str(&indent(&def.source));
        out.push('\n');
        return Ok(out);
    }

    // A builtin: the manual is its source.
    if let Some(Value::Builtin(_)) = bound {
        return Ok(match builtin_entry(name) {
            Some(e) => render_entry(e, "builtin"),
            None => format!("{} — builtin\n\n  ({} …)\n  Undocumented.\n", name, name),
        });
    }

    // A special form, which is not a binding at all.
    if let Some(e) = special_form_entry(name) {
        return Ok(render_entry(e, "special form"));
    }

    match bound {
        // Defined in EELisp, but not at the top level of anything we parsed.
        Some(v @ Value::Function(_)) | Some(v @ Value::Macro(_)) => {
            let kind = kind_of(&v).unwrap_or("function");
            let mut out = format!("{} — {}\n\n", name, kind);
            out.push_str("  ;; no source text on record — rebuilt from the definition\n");
            out.push_str(&indent(&reconstruct(name, &v).unwrap_or_default()));
            out.push('\n');
            Ok(out)
        }
        Some(other) => Ok(format!(
            "{} — {}, not a function\n\n{}\n",
            name,
            type_name(&other),
            indent(&print_value(&other, true))
        )),
        None => Err(LispError::Runtime(format!(
            "source: nothing named {:?} — try (functions {:?})",
            name, name
        ))),
    }
}

fn render_entry(e: &Entry, kind: &str) -> String {
    let mut out = format!("{} — {}\n\n  {}\n  {}\n", e.name, kind, e.sig, e.summary);
    if !e.example.is_empty() {
        out.push('\n');
        out.push_str(&indent(e.example));
        out.push('\n');
    }
    out
}

// ── registration ─────────────────────────────────────────────────────

/// Register `functions` and `source`. Both write through the host's output channel — they are
/// things you read, so they print and return nil, the way `println` does.
pub fn register(env: &Env, out: Rc<RefCell<OutputState>>, index: Rc<RefCell<SourceIndex>>) {
    {
        let out = out.clone();
        let f = move |args: &[Value], env: &Env| -> Result<Value, LispError> {
            let filter = filter_text(args.first());
            let (rows, total) = listing(env, filter.as_deref());
            out.borrow_mut().emit(&render_listing(&rows, total, filter.as_deref()));
            Ok(Value::Null)
        };
        env::define(
            env,
            "functions",
            Value::Builtin(Rc::new(Builtin {
                name: "functions".into(),
                arg_mode: ArgMode::Eval,
                func: Box::new(f),
            })),
        );
    }
    {
        let out = out.clone();
        let index = index.clone();
        let f = move |args: &[Value], env: &Env| -> Result<Value, LispError> {
            let name = name_arg(args.first()).ok_or_else(|| {
                LispError::InvalidSyntax("source expects a function name, e.g. (source map)".into())
            })?;
            let text = render_source(&name, env, &index.borrow())?;
            out.borrow_mut().emit(&text);
            Ok(Value::Null)
        };
        env::define(
            env,
            "source",
            Value::Builtin(Rc::new(Builtin {
                name: "source".into(),
                // The name is passed unevaluated, so `(source map)` reads as well as `(source 'map)`.
                arg_mode: ArgMode::TableFirst,
                func: Box::new(f),
            })),
        );
    }
}

// ── the tables ───────────────────────────────────────────────────────
//
// Kept in the order the reference site uses, so the two stay easy to diff by eye.

macro_rules! doc {
    ($name:expr, $sig:expr, $summary:expr) => {
        Entry { name: $name, sig: $sig, summary: $summary, example: "" }
    };
    ($name:expr, $sig:expr, $summary:expr, $example:expr) => {
        Entry { name: $name, sig: $sig, summary: $summary, example: $example }
    };
}

/// Evaluated by the interpreter itself, so they don't follow the usual argument rules.
pub static SPECIAL_FORMS: &[Entry] = &[
    doc!("def", "(def name value)", "Defines a global binding.", "(def pi 3.14159)   → 3.14159"),
    doc!("defn", "(defn name (params) body…)", "Defines a function. A parameter list may end with `. rest` to collect the remaining arguments.",
         "(defn square (n) (* n n))\n(square 7)   → 49"),
    doc!("defun", "(defun name (params) body…)", "The Common Lisp spelling of defn — the same form.",
         "(defun square (n) (* n n))"),
    doc!("fn", "(fn (params) body…) → function", "An anonymous function.", "(map (fn (n) (* n 2)) '(1 2 3))   → (2 4 6)"),
    doc!("lambda", "(lambda (params) body…) → function", "The Common Lisp spelling of fn — the same form.",
         "(map (lambda (n) (* n 2)) '(1 2 3))   → (2 4 6)"),
    doc!("defmacro", "(defmacro name (params) body…)", "Defines a macro; rest parameters and quasiquote are allowed.",
         "(defmacro when (test . body)\n  `(if ,test (do ,@body) nil))"),
    doc!("let", "(let (a 1 b 2) body…)", "Local bindings, given as one flat list. Each may refer to the ones before it.",
         "(let (a 2 b (* a 3)) (+ a b))   → 8"),
    doc!("set!", "(set! name value)", "Reassigns an existing binding. It is an error if the name is unbound.",
         "(def n 1)\n(set! n (+ n 1))   → 2"),
    doc!("if", "(if test then else)", "Two-way branch. Only false and nil are falsy — 0, \"\" and () are all true.",
         "(if (> 3 2) \"bigger\" \"smaller\")   → \"bigger\""),
    doc!("cond", "(cond test result …)", "Flat multi-way branch: pairs of test and result, first match wins.",
         "(cond (< n 0) \"neg\"\n      (= n 0) \"zero\"\n      true    \"pos\")"),
    doc!("and", "(and a b …)", "Short-circuiting conjunction — returns the first falsy value, or the last one.",
         "(and 1 2 3)   → 3"),
    doc!("or", "(or a b …)", "Short-circuiting disjunction — returns the first truthy value.",
         "(or nil false 7)   → 7"),
    doc!("do", "(do e…)", "Evaluates its forms in order and returns the last.", "(do (println \"hi\") 42)   → 42"),
    doc!("begin", "(begin e…)", "The Scheme spelling of do — the same form.", "(begin 1 2 3)   → 3"),
    doc!("for-each", "(for-each var list body…)", "Runs the body once per element, for its effects. Returns nil.",
         "(for-each x '(1 2 3) (println x))"),
    doc!("loop", "(loop (a 0 b 1) body…)", "A tail-recursive loop with its own bindings; `recur` jumps back with new values.",
         "(loop (i 0 acc 0)\n  (if (> i 4) acc (recur (+ i 1) (+ acc i))))   → 10"),
    doc!("recur", "(recur v…)", "Jumps back to the enclosing loop with a new set of values. Tail position only.",
         "(recur (+ i 1) (* acc i))"),
    doc!("quote", "(quote x)  ·  'x", "Returns its argument unevaluated.", "'(1 2 3)   → (1 2 3)"),
    doc!("quasiquote", "(quasiquote x)  ·  `x", "A template: `,` unquotes one value, `,@` splices a list in.",
         "(def xs '(2 3))\n`(1 ,@xs 4)   → (1 2 3 4)"),
    doc!("unquote", "(unquote x)  ·  ,x", "Inside a quasiquote, evaluates x and drops the value in.", ""),
    doc!("unquote-splicing", "(unquote-splicing x)  ·  ,@x", "Inside a quasiquote, evaluates x and splices the list in.", ""),
];

/// The core language: arithmetic, strings, lists, dicts, types, dates, IO.
pub static CORE: &[Entry] = &[
    // ── arithmetic ───────────────────────────────────────────────────
    doc!("+", "(+ a b …) → number", "Adds every argument.", "(+ 1 2 3)   → 6"),
    doc!("-", "(- a b …) → number", "Subtracts the rest from the first. `(- 5)` is 5 — write -5 for a negative literal.", "(- 10 3 2)   → 5"),
    doc!("*", "(* a b …) → number", "Multiplies every argument.", "(* 2 3 4)   → 24"),
    doc!("/", "(/ a b …) → number", "Divides left to right. Dividing by zero is an error.", "(/ 100 5 2)   → 10"),
    doc!("mod", "(mod a b) → number", "The remainder of a divided by b.", "(mod 7 3)   → 1"),
    doc!("abs", "(abs n) → number", "Absolute value.", "(abs -4)   → 4"),
    doc!("min", "(min a b …) → number", "The smallest number. Lists are searched too, skipping nil and text — so (min B1:B9) reads a range.", "(min 3 '(1 nil 2))   → 1"),
    doc!("max", "(max a b …) → number", "The largest number. Lists are searched too, skipping nil and text.", "(max 3 '(1 nil 7))   → 7"),
    doc!("sum", "(sum a b …) → number", "Adds the numbers, looking inside lists and skipping nil and text — what a sheet's (sum B1:B9) needs. Nothing to add is 0.",
         "(sum 1 '(2 nil \"x\" 3))   → 6"),
    doc!("avg", "(avg a b …) → number", "The mean of the numbers, looking inside lists and skipping nil and text. An error when there are none.",
         "(avg '(2 4 nil))   → 3"),
    doc!("floor", "(floor n) → number", "Rounds down.", "(floor 2.7)   → 2"),
    doc!("ceil", "(ceil n) → number", "Rounds up.", "(ceil 2.1)   → 3"),
    doc!("round", "(round n) → number", "Rounds to the nearest whole number, halves away from zero.", "(round 2.5)   → 3"),
    doc!("pow", "(pow base exp) → number", "Raises base to exp.", "(pow 2 10)   → 1024"),
    doc!("expt", "(expt base exp) → number", "The Common Lisp spelling of pow — the same function.", "(expt 2 10)   → 1024"),
    // ── comparison & logic ───────────────────────────────────────────
    doc!("=", "(= a b …) → bool", "Structural equality: lists and dicts compare by content, functions never compare equal.",
         "(= '(1 2) '(1 2))   → true"),
    doc!("!=", "(!= a b) → bool", "The negation of =.", "(!= 1 2)   → true"),
    doc!("<", "(< a b …) → bool", "True when the arguments are in strictly ascending order.", "(< 1 2 3)   → true"),
    doc!(">", "(> a b …) → bool", "True when the arguments are in strictly descending order.", "(> 3 2 1)   → true"),
    doc!("<=", "(<= a b …) → bool", "Ascending order, ties allowed.", "(<= 1 1 2)   → true"),
    doc!(">=", "(>= a b …) → bool", "Descending order, ties allowed.", "(>= 2 1 1)   → true"),
    doc!("not", "(not x) → bool", "Logical negation. Only false and nil are falsy — 0, \"\" and () are all true.",
         "(not 0)   → false\n(not nil)   → true"),
    // ── strings ──────────────────────────────────────────────────────
    doc!("str", "(str a b …) → string", "Concatenates anything, converting as needed.", "(str \"x=\" 42)   → \"x=42\""),
    doc!("str-len", "(str-len s) → number", "Length in characters.", "(str-len \"hello\")   → 5"),
    doc!("str-upper", "(str-upper s) → string", "Upper-cases a string.", "(str-upper \"hi\")   → \"HI\""),
    doc!("str-lower", "(str-lower s) → string", "Lower-cases a string.", "(str-lower \"Hi\")   → \"hi\""),
    doc!("str-trim", "(str-trim s) → string", "Drops leading and trailing whitespace.", "(str-trim \"  hi  \")   → \"hi\""),
    doc!("str-split", "(str-split s sep) → list", "Splits on a separator.", "(str-split \"a,b,c\" \",\")   → (\"a\" \"b\" \"c\")"),
    doc!("str-join", "(str-join sep parts) → string", "Joins a list with a separator. Either argument order is accepted.",
         "(str-join \"-\" '(\"a\" \"b\"))   → \"a-b\""),
    doc!("str-contains", "(str-contains s needle) → bool", "Is needle anywhere in s?", "(str-contains \"hello\" \"ell\")   → true"),
    doc!("str-matches", "(str-matches s pattern) → bool", "Regular-expression test.", "(str-matches \"a1\" \"^[a-z][0-9]$\")   → true"),
    doc!("str-replace", "(str-replace s target repl) → string", "Replaces every occurrence of target.",
         "(str-replace \"a-b-c\" \"-\" \"+\")   → \"a+b+c\""),
    doc!("substr", "(substr s start end) → string", "A slice by character index: 0-based, end-exclusive.", "(substr \"hello\" 1 3)   → \"el\""),
    doc!("str-starts-with", "(str-starts-with s prefix) → bool", "Does s begin with prefix?", "(str-starts-with \"hello\" \"he\")   → true"),
    doc!("str-ends-with", "(str-ends-with s suffix) → bool", "Does s end with suffix?", "(str-ends-with \"hello\" \"lo\")   → true"),
    // ── lists ────────────────────────────────────────────────────────
    doc!("list", "(list a b …) → list", "Builds a list. `[a b]` is shorthand for the same thing.", "(list 1 2 3)   → (1 2 3)"),
    doc!("cons", "(cons x lst) → list", "Prepends one element.", "(cons 1 '(2 3))   → (1 2 3)"),
    doc!("head", "(head lst)", "The first element, or nil when the list is empty.", "(head '(1 2 3))   → 1"),
    doc!("car", "(car lst)", "The Lisp spelling of head — the same function.", "(car '(1 2 3))   → 1"),
    doc!("tail", "(tail lst) → list", "Everything after the first element.", "(tail '(1 2 3))   → (2 3)"),
    doc!("cdr", "(cdr lst) → list", "The Lisp spelling of tail — the same function.", "(cdr '(1 2 3))   → (2 3)"),
    doc!("nth", "(nth lst i)", "The element at index i, 0-based; nil past the end.", "(nth '(1 2 3) 1)   → 2"),
    doc!("length", "(length lst) → number", "How many elements the list has.", "(length '(1 2 3))   → 3"),
    doc!("append", "(append a b) → list", "Concatenates two lists.", "(append '(1 2) '(3))   → (1 2 3)"),
    doc!("reverse", "(reverse lst) → list", "The list back to front.", "(reverse '(1 2 3))   → (3 2 1)"),
    doc!("range", "(range from to step) → list", "A list of numbers, end-exclusive. With one argument it starts at 0; step defaults to 1.",
         "(range 1 5)   → (1 2 3 4)\n(range 3)     → (0 1 2)"),
    doc!("map", "(map f lst) → list", "Applies f to every element.", "(map (fn (n) (* n n)) '(1 2 3))   → (1 4 9)"),
    doc!("filter", "(filter pred lst) → list", "Keeps the elements pred says yes to.", "(filter even? '(1 2 3 4))   → (2 4)"),
    doc!("reduce", "(reduce f init lst)", "Folds the list left to right, f taking (accumulator element).",
         "(reduce + 0 '(1 2 3))   → 6"),
    doc!("sort-by", "(sort-by keyfn lst) → list", "A stable ascending sort by a numeric key.",
         "(sort-by str-len '(\"ccc\" \"a\" \"bb\"))   → (\"a\" \"bb\" \"ccc\")"),
    doc!("zip", "(zip a b) → list", "Pairs two lists up, stopping at the shorter one.", "(zip '(1 2) '(\"a\" \"b\"))   → ((1 \"a\") (2 \"b\"))"),
    doc!("empty?", "(empty? lst) → bool", "Is the list empty?", "(empty? '())   → true"),
    // ── dicts ────────────────────────────────────────────────────────
    doc!("dict", "(dict :k v …) → dict", "Builds a dict. `{:k v}` is the literal syntax; keys keep their insertion order.",
         "(dict :a 1 :b 2)   → {:a 1 :b 2}"),
    doc!("dict-get", "(dict-get d :k)", "The value at a key, or nil.", "(dict-get {:a 1} :a)   → 1"),
    doc!("dict-set", "(dict-set d :k v) → dict", "A new dict with the key set — the original is untouched.",
         "(dict-set {:a 1} :b 2)   → {:a 1 :b 2}"),
    doc!("dict-keys", "(dict-keys d) → list", "The keys, in insertion order.", "(dict-keys {:a 1 :b 2})   → (\"a\" \"b\")"),
    doc!("dict-values", "(dict-values d) → list", "The values, in insertion order.", "(dict-values {:a 1 :b 2})   → (1 2)"),
    doc!("dict-has", "(dict-has d :k) → bool", "Is the key present?", "(dict-has {:a 1} :a)   → true"),
    doc!("dict-merge", "(dict-merge a b) → dict", "Merges two dicts; keys from b win.", "(dict-merge {:a 1} {:a 9 :b 2})   → {:a 9 :b 2}"),
    // ── types & conversion ───────────────────────────────────────────
    doc!("type", "(type x) → string", "The type name: number, string, bool, keyword, nil, list, dict, function, record…",
         "(type '(1))   → \"list\""),
    doc!("number?", "(number? x) → bool", "Is it a number?", "(number? 1)   → true"),
    doc!("string?", "(string? x) → bool", "Is it a string?", "(string? \"a\")   → true"),
    doc!("bool?", "(bool? x) → bool", "Is it true or false?", "(bool? false)   → true"),
    doc!("list?", "(list? x) → bool", "Is it a list?", "(list? '(1))   → true"),
    doc!("nil?", "(nil? x) → bool", "Is it nil?", "(nil? nil)   → true"),
    doc!("symbol?", "(symbol? x) → bool", "Is it a symbol?", "(symbol? 'a)   → true"),
    doc!("keyword?", "(keyword? x) → bool", "Is it a keyword?", "(keyword? :a)   → true"),
    doc!("fn?", "(fn? x) → bool", "Is it callable — a function or a builtin?", "(fn? map)   → true"),
    doc!("->string", "(->string x) → string", "Coerces to a string.", "(->string 42)   → \"42\""),
    doc!("->number", "(->number x) → number", "Coerces to a number; nil when it doesn't read as one.", "(->number \"42\")   → 42"),
    doc!("->bool", "(->bool x) → bool", "Coerces to a bool: everything but false and nil is true.", "(->bool 0)   → true"),
    doc!("parse", "(parse s)", "Reads EELisp source into a value, without evaluating it.", "(parse \"(+ 1 2)\")   → (+ 1 2)"),
    // ── dates ────────────────────────────────────────────────────────
    doc!("now", "(now) → number", "The current time, in epoch seconds.", "(date-format (now) \"HH:mm\")   → \"09:41\""),
    doc!("today", "(today) → number", "Midnight today, in epoch seconds.", "(date-format (today) \"yyyy-MM-dd\")   → \"2026-09-11\""),
    doc!("date-format", "(date-format d pattern) → string", "Formats an epoch number or a \"YYYY-MM-DD\" string. Pattern defaults to \"yyyy-MM-dd HH:mm\".",
         "(date-format (today) \"EEEE\")   → \"Friday\""),
    doc!("date-add", "(date-add date n unit) → string", "Shifts a \"YYYY-MM-DD\" date by n :days, :weeks or :months.",
         "(date-add \"2026-09-11\" 3 :days)   → \"2026-09-14\""),
    doc!("date-diff", "(date-diff a b) → number", "Whole days from a to b, both \"YYYY-MM-DD\" strings.",
         "(date-diff \"2026-09-11\" \"2026-09-14\")   → 3"),
    // ── evaluation, JSON, network ────────────────────────────────────
    doc!("print", "(print x …) → nil", "Prints its arguments with no trailing newline. Captured by the host, not written to a terminal.",
         "(print \"a\" \"b\")"),
    doc!("println", "(println x …) → nil", "Prints its arguments and a newline.", "(println \"total:\" 42)"),
    doc!("eval", "(eval form)", "Evaluates a value as code — the other half of `parse`.", "(eval (parse \"(+ 1 2)\"))   → 3"),
    doc!("apply", "(apply f args)", "Calls f with a list of arguments.", "(apply + '(1 2 3))   → 6"),
    doc!("json-parse", "(json-parse s)", "Reads a JSON string into dicts, lists and scalars.", "(json-parse \"{\\\"a\\\":1}\")   → {:a 1}"),
    doc!("json-stringify", "(json-stringify v) → string", "Writes a value as JSON.", "(json-stringify {:a 1})   → \"{\\\"a\\\":1}\""),
    doc!("http-get", "(http-get url) → string", "A synchronous GET. With `http-post`, the only network access in the language.",
         "(json-parse (http-get \"https://example.com/api\"))"),
    doc!("http-post", "(http-post url body) → string", "A synchronous POST.", "(http-post \"https://example.com/api\" \"{}\")"),
    // ── functions / source ───────────────────────────────────────────
    doc!("functions", "(functions filter) → nil", "Lists every function, macro and special form in scope; the filter keeps the names containing it.",
         "(functions)          ; everything\n(functions \"date\")   ; just the date ones"),
    doc!("source", "(source name) → nil", "Shows a definition: the source and comments for EELisp code, the manual entry for a builtin.",
         "(source map)\n(source zzalinhar)"),
];

/// The dBASE-style database layer. Table names are passed unevaluated — write `contacts`, not
/// `'contacts` or `"contacts"`.
pub static DATABASE: &[Entry] = &[
    doc!("deftable", "(deftable name (fields…)) → table", "Defines a table. A field is `name:type` or `(name :type … :required … :default … :choices …)`; types are string, number, bool, date, memo, choice.",
         "(deftable contacts (name:string age:number))"),
    doc!("insert", "(insert table {dict}) → record", "Inserts a row and returns it, id included.",
         "(insert contacts {:name \"Ada\" :age 36})"),
    doc!("query", "(query table :where … :params … :order … :asc … :desc … :limit … :select …) → result-set",
         "Selects rows. `:where` is SQL with `?` placeholders filled from `:params`.",
         "(query contacts :where \"age > ?\" :params '(30) :order \"name\")"),
    doc!("records", "(records rs) → list", "The rows of a result-set, as an ordinary list.",
         "(map record-id (records (query contacts)))"),
    doc!("update", "(update table id {dict}) → record", "Changes the named fields of one row.",
         "(update contacts 1 {:age 37})"),
    doc!("delete", "(delete table id) → bool", "A soft delete — the row is marked, not removed. See `pack`.", "(delete contacts 1)"),
    doc!("pack", "(pack table) → number", "Physically removes the soft-deleted rows and returns how many went.", "(pack contacts)"),
    doc!("count-records", "(count-records table :where … :params …) → number", "How many rows match.",
         "(count-records contacts :where \"age > ?\" :params '(30))"),
    doc!("tables", "(tables) → list", "The names of every defined table.", "(tables)   → (\"contacts\")"),
    doc!("describe", "(describe table) → table", "The schema of a table: its fields and their types.", "(describe contacts)"),
    doc!("drop-table", "(drop-table table) → bool", "Deletes a table and everything in it.", "(drop-table contacts)"),
    doc!("database-info", "(database-info) → dict", "Which database file the tables and agenda live in — `:memory:` when nothing is kept. `:error` says why, when the host asked for a file that couldn't be opened.",
         "(database-info)   → {:path \"/Users/me/notes/.eeditor/eeditor.db\"}"),
    doc!("browse", "(browse table) → table-view", "An interactive grid of the rows — a widget in the editor, an ASCII table on the command line.",
         "(browse contacts)"),
    doc!("edit", "(edit table id) → form-view", "An interactive form for one record.", "(edit contacts 1)"),
    doc!("defform", "(defform name table (field)…) → form-view", "A named form; a field given an expression becomes a computed field.",
         "(defform total invoices (qty) (price) (sum :number (* qty price)))"),
    doc!("field-get", "(field-get r \"name\")", "One field of a record.", "(field-get r \"name\")   → \"Ada\""),
    doc!("field-set", "(field-set r \"name\" v) → record", "A copy of the record with one field changed.", "(field-set r \"age\" 37)"),
    doc!("record-id", "(record-id r) → number", "The record's id.", "(record-id r)   → 1"),
];

/// The Lotus-Agenda PIM: items, categories, rules, views, templates and agenda files.
pub static AGENDA: &[Entry] = &[
    doc!("add-item", "(add-item text :when … :priority … :category … :notes …) → item", "Adds an item with explicit properties.",
         "(add-item \"call Bob\" :when \"2026-09-14\" :priority 1)"),
    doc!("add-item-today", "(add-item-today text) → item", "Adds an item due today.", "(add-item-today \"water the plants\")"),
    doc!("add", "(add text) → item", "Adds an item written in plain language — the date, priority and people are parsed out of the text.",
         "(add \"call Bob tomorrow !!\")"),
    doc!("smart-parse", "(smart-parse text) → dict", "The same parse as `add`, without storing anything.",
         "(smart-parse \"lunch friday @ana\")"),
    doc!("items", "(items :search … :category … :priority … :when-before … :when-after …) → list", "The items matching a filter; no filter means all of them.",
         "(items :category \"work\" :when-before \"2026-10-01\")"),
    doc!("items-on", "(items-on date) → list", "Items due on one day.", "(items-on \"2026-09-11\")"),
    doc!("items-between", "(items-between from to) → list", "Items due in a date range, ends included.",
         "(items-between \"2026-09-01\" \"2026-09-30\")"),
    doc!("item-get", "(item-get id) → item", "One item by id.", "(item-get 3)"),
    doc!("item-set", "(item-set id :k v …) → item", "Changes an item's text, properties or notes.", "(item-set 3 :priority 1)"),
    doc!("item-done", "(item-done id) → item", "Marks an item done; a recurring item rolls forward to its next date.", "(item-done 3)"),
    doc!("item-count", "(item-count) → number", "How many items the agenda holds.", "(item-count)"),
    doc!("every", "(every n unit) → string", "A recurrence — :days, :weeks or :months — for an item's :recurrence property.",
         "(add-item \"rent\" :recurrence (every 1 :months))"),
    doc!("defcategory", "(defcategory work/calls) → list", "Defines a category. Paths are hierarchical and the parents are implied.",
         "(defcategory work/calls)"),
    doc!("assign", "(assign id \"path\") → item", "Files an item under a category.", "(assign 3 \"work/calls\")"),
    doc!("unassign", "(unassign id \"path\") → item", "Removes an item from a category.", "(unassign 3 \"work/calls\")"),
    doc!("categories", "(categories) → list", "Every category, as paths.", "(categories)   → (\"work\" \"work/calls\")"),
    doc!("defrule", "(defrule name :when … :assign … :action …) → dict", "A rule that files items automatically. `:assign` and `:action` may repeat.",
         "(defrule calls :when (match \"call\") :assign \"work/calls\")"),
    doc!("apply-rules", "(apply-rules id) → number", "Runs the rules over one item, or over all of them with no id. Returns how many changed.",
         "(apply-rules)   → 4"),
    doc!("auto-categorize", "(auto-categorize on) → bool", "Whether new items get the rules applied as they arrive.", "(auto-categorize true)"),
    doc!("rules", "(rules) → list", "Every defined rule.", "(rules)"),
    doc!("drop-rule", "(drop-rule \"name\") → bool", "Deletes a rule.", "(drop-rule \"calls\")"),
    doc!("defview", "(defview name (:category … :group-by …)) → dict", "A saved, filtered — optionally grouped — view of the agenda.",
         "(defview today (:when-before \"2026-09-12\"))"),
    doc!("show", "(show name) → list", "Runs a saved view.", "(show today)"),
    doc!("views", "(views) → list", "Every saved view.", "(views)"),
    doc!("drop-view", "(drop-view \"name\") → bool", "Deletes a saved view.", "(drop-view \"today\")"),
    doc!("deftemplate", "(deftemplate name (…)) → dict", "A template of item properties to stamp out repeatedly.",
         "(deftemplate standup (:priority 2 :category \"work\"))"),
    doc!("from-template", "(from-template name text) → item", "Adds an item from a template.", "(from-template standup \"monday standup\")"),
    doc!("templates", "(templates) → list", "Every template.", "(templates)"),
    doc!("drop-template", "(drop-template name) → bool", "Deletes a template.", "(drop-template standup)"),
    doc!("open-agenda", "(open-agenda \"file.db\") → string", "Opens an agenda file as an isolated database and gives it a name.",
         "(open-agenda \"work.db\")"),
    doc!("use-agenda", "(use-agenda name) → string", "Switches which open agenda the item functions act on.", "(use-agenda work)"),
    doc!("close-agenda", "(close-agenda name) → bool", "Closes an open agenda.", "(close-agenda work)"),
    doc!("agendas", "(agendas) → list", "Every open agenda, the current one first.", "(agendas)"),
    doc!("export-agenda", "(export-agenda \"f.json\") → number", "Writes the agenda to JSON and returns how many items went.",
         "(export-agenda \"backup.json\")"),
    doc!("import-agenda", "(import-agenda \"f.json\") → number", "Reads items back from JSON, in one transaction.",
         "(import-agenda \"backup.json\")"),
];

/// Sheets: a grid whose formulas are EELisp, each sheet a `.eesheet` file. A sheet is named
/// `"Budget"`, `"money/Budget"` or by absolute path — relative to (current-dir), `.eesheet` implied.
/// Cells are `"C3"`, areas `"A1:C9"`. Inside a formula, `C3` and `A1:C9` are the cells' values, and
/// `Rates!A1` reads a sheet of that name beside this one.
pub static SHEETS: &[Entry] = &[
    doc!("sheet-new", "(sheet-new name) → string", "Creates an empty sheet file and returns its path. An error if the file exists.",
         "(sheet-new \"Budget\")"),
    doc!("sheet-open", "(sheet-open name) → dict", "The whole sheet for a host to draw: `{:path :version :cells ((row col input value error fmt) …) :widths ((col width) …)}`, rows and columns from 0. Runs no formula — values are the ones stored.",
         "(sheet-open \"Budget\")"),
    doc!("sheet-close", "(sheet-close name) → nil", "Closes the file. Do it before renaming or deleting one.", "(sheet-close \"Budget\")"),
    doc!("sheet-get", "(sheet-get name cell)", "One cell's value. A cell whose formula failed is an error.", "(sheet-get \"Budget\" \"C3\")   → 1650"),
    doc!("sheet-rows", "(sheet-rows name area) → list", "An area's values as a list of rows.",
         "(sheet-rows \"Budget\" \"A1:B2\")   → ((\"rent\" 1200) (\"food\" 450))"),
    doc!("sheet-set", "(sheet-set name cell input) → list", "Types into a cell — `1200`, `rent`, `'42` for text, `=(sum B1:B2)` for a formula — recalculates what depends on it and saves. Given a list of rows, types a whole block from that corner, as a paste would. Returns the changed cells as `(row col input value error fmt)`.",
         "(sheet-set \"Budget\" \"B2\" \"450\")\n(sheet-set \"Budget\" \"C1\" '((\"=(sum B1:B2)\") (\"=(/ C1 12)\")))"),
    doc!("sheet-put", "(sheet-put name cell data) → list", "Writes a block with its top-left corner at cell: a list of rows, one flat row, or a result set — a header row, then its records. Values stay values: text that would read as a number or formula is kept as text.",
         "(sheet-put \"Report\" \"A1\" (query contacts :order \"name\"))"),
    doc!("sheet-copy", "(sheet-copy name area) → list", "What was typed into an area, as rows of text — the other half of sheet-paste.",
         "(sheet-copy \"Budget\" \"B1:B3\")   → ((\"1200\") (\"450\") (\"=(sum B1:B2)\"))"),
    doc!("sheet-paste", "(sheet-paste name cell rows from) → list", "Types rows of input from a cell. Given `from` — where they were copied — every formula's references move by the distance travelled, so a pasted total adds up its new neighbours. Without it the text is typed exactly, which is what a paste from another program needs.",
         "(sheet-paste \"Budget\" \"C1\" (sheet-copy \"Budget\" \"B1:B3\") \"B1\")"),
    doc!("sheet-fill", "(sheet-fill name source target) → list", "Repeats the source over the target area, each copy's references shifted by where it lands — filling a column of totals down. The source's own cells are left alone.",
         "(sheet-fill \"Budget\" \"C1\" \"C2:C12\")"),
    doc!("sheet-recalc", "(sheet-recalc name) → list", "Reruns every formula — for ones that read the database or the clock, and for another sheet that wasn't open at the time, none of which are tracked.",
         "(sheet-recalc \"Budget\")"),
    doc!("sheet-format", "(sheet-format name area fmt) → list", "Merges a format into every cell of an area — a key set to nil is removed, and a nil format clears it. Given rows of formats instead, sets each cell's exactly from the area's corner. The keys are the host's: :num, :dp, :cur, :bold, :italic, :align.",
         "(sheet-format \"Budget\" \"B1:B9\" {:num \"currency\" :dp 2})"),
    doc!("sheet-col-width", "(sheet-col-width name col width) → nil", "A column's width, or nil for the default.", "(sheet-col-width \"Budget\" \"A\" 160)"),
    doc!("sheet-insert-rows", "(sheet-insert-rows name row n) → dict", "Inserts n rows (default 1) before a row numbered from 1. Formulas are rewritten to follow their cells.",
         "(sheet-insert-rows \"Budget\" 3)"),
    doc!("sheet-delete-rows", "(sheet-delete-rows name row n) → dict", "Deletes n rows from a row numbered from 1. A formula that read a deleted cell reads #REF!.",
         "(sheet-delete-rows \"Budget\" 3 2)"),
    doc!("sheet-insert-cols", "(sheet-insert-cols name col n) → dict", "Inserts n columns before a column.", "(sheet-insert-cols \"Budget\" \"B\")"),
    doc!("sheet-delete-cols", "(sheet-delete-cols name col n) → dict", "Deletes n columns from a column.", "(sheet-delete-cols \"Budget\" \"B\")"),
    doc!("sheet-version", "(sheet-version name) → number", "A counter that moves on every write — how a host notices a sheet changed.",
         "(sheet-version \"Budget\")   → 12"),
];

/// The editor RPC — installed by the host, so these answer inside an editor and are inert on the
/// bare command line (reads give \"\" or nil, writes do nothing).
pub static EDITOR: &[Entry] = &[
    doc!("buffer-text", "(buffer-text) → string", "The whole document being edited.", "(str-len (buffer-text))"),
    doc!("current-file", "(current-file) → string", "The path of the open note.", "(current-file)   → \"/notes/today.md\""),
    doc!("current-dir", "(current-dir) → string", "The workspace root — the folder being edited, not the process's working directory.",
         "(current-dir)   → \"/Users/me/notes\""),
    doc!("cursor-pos", "(cursor-pos) → number", "The cursor's character offset in the document.", "(cursor-pos)   → 142"),
    doc!("selection", "(selection) → list", "The selection as (from to); both the same when nothing is selected.", "(selection)   → (10 24)"),
    doc!("set-cursor", "(set-cursor n) → nil", "Moves the cursor to a character offset.", "(set-cursor 0)"),
    doc!("insert-at", "(insert-at pos text) → nil", "Inserts text at an offset.", "(insert-at (cursor-pos) \"— \")"),
    doc!("replace-range", "(replace-range from to text) → nil", "Replaces a range of the document.",
         "(replace-range 0 5 \"Hello\")"),
];
