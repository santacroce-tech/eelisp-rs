//! Agenda builtins (ANALYSIS §4.10). Item/template builtins capture the active `Database`;
//! multi-agenda builtins additionally capture the `Agendas` registry and swap the active DB.
//!
//! Bare-symbol arguments (template names, `(use-agenda memory)`) use `ArgMode::TableFirst`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::agenda::{self, Agendas, ItemFilter, ItemInput, TemplateInput};
use crate::database::Database;
use crate::env::{self, Env};
use crate::value::*;

type Db = Rc<RefCell<Database>>;
type Reg = Rc<RefCell<Agendas>>;

fn defb(
    env: &Env,
    name: &str,
    mode: ArgMode,
    f: impl Fn(&[Value], &Env) -> Result<Value, LispError> + 'static,
) {
    env::define(
        env,
        name,
        Value::Builtin(Rc::new(Builtin { name: name.to_string(), arg_mode: mode, func: Box::new(f) })),
    );
}

pub fn register(env: &Env, db: Db, reg: Reg) {
    // ── every (recurrence pattern string) ──
    defb(env, "every", ArgMode::Eval, |args, _| {
        let n = match args.first() {
            Some(Value::Number(n)) => *n as i64,
            _ => return Err(LispError::InvalidSyntax("every expects a number".into())),
        };
        let unit = match args.get(1) {
            Some(Value::Keyword(k)) | Some(Value::Symbol(k)) => k.clone(),
            _ => "days".into(),
        };
        Ok(Value::Str(format!("every:{}:{}", n, unit)))
    });

    // ── items ──
    {
        let db = db.clone();
        let reg = reg.clone();
        defb(env, "add-item", ArgMode::Eval, move |args, ev| {
            let input = ItemInput {
                text: str_arg(args.first(), "add-item")?,
                notes: kw(args, "notes").and_then(str_val).unwrap_or_default(),
                when: kw(args, "when").and_then(str_val),
                priority: kw(args, "priority").and_then(int_val),
                category: kw(args, "category").and_then(sym_or_str),
                recur: kw(args, "recur").and_then(recur_val),
            };
            let result = agenda::add_item(&db.borrow(), input)?;
            maybe_auto_categorize(&db, &reg, ev, &result)?;
            Ok(result)
        });
    }
    {
        let db = db.clone();
        let reg = reg.clone();
        defb(env, "add-item-today", ArgMode::Eval, move |args, ev| {
            let input = ItemInput {
                text: str_arg(args.first(), "add-item-today")?,
                notes: kw(args, "notes").and_then(str_val).unwrap_or_default(),
                when: Some(agenda::today()),
                priority: kw(args, "priority").and_then(int_val),
                category: kw(args, "category").and_then(sym_or_str),
                recur: kw(args, "recur").and_then(recur_val),
            };
            let result = agenda::add_item(&db.borrow(), input)?;
            maybe_auto_categorize(&db, &reg, ev, &result)?;
            Ok(result)
        });
    }
    {
        let db = db.clone();
        defb(env, "item-count", ArgMode::Eval, move |args, _| {
            let cat = kw(args, "category").and_then(str_val);
            agenda::item_count(&db.borrow(), cat.as_deref())
        });
    }
    {
        let db = db.clone();
        defb(env, "item-get", ArgMode::Eval, move |args, _| {
            agenda::item_get(&db.borrow(), int_arg(args.first())?)
        });
    }
    {
        let db = db.clone();
        defb(env, "item-set", ArgMode::Eval, move |args, _| {
            let id = int_arg(args.first())?;
            let updates = collect_kw_pairs(&args[1.min(args.len())..]);
            agenda::item_set(&db.borrow(), id, &updates)
        });
    }
    {
        let db = db.clone();
        defb(env, "item-done", ArgMode::Eval, move |args, _| {
            agenda::item_done(&db.borrow(), int_arg(args.first())?)
        });
    }
    {
        let db = db.clone();
        defb(env, "items", ArgMode::Eval, move |args, _| {
            let f = ItemFilter {
                search: kw(args, "search").and_then(str_val),
                category: kw(args, "category").and_then(sym_or_str),
                priority: kw(args, "priority").and_then(int_val),
                when_before: kw(args, "when-before").and_then(str_val),
                when_after: kw(args, "when-after").and_then(str_val),
            };
            agenda::items(&db.borrow(), &f)
        });
    }
    {
        let db = db.clone();
        defb(env, "items-on", ArgMode::Eval, move |args, _| {
            agenda::items_on(&db.borrow(), &str_arg(args.first(), "items-on")?)
        });
    }
    {
        let db = db.clone();
        defb(env, "items-between", ArgMode::Eval, move |args, _| {
            let start = str_arg(args.first(), "items-between")?;
            let end = str_arg(args.get(1), "items-between")?;
            agenda::items_between(&db.borrow(), &start, &end)
        });
    }

    // ── templates ──
    {
        let db = db.clone();
        defb(env, "deftemplate", ArgMode::TableFirst, move |args, _| {
            let t = TemplateInput {
                name: sym_or_str(args.first().unwrap_or(&Value::Null))
                    .ok_or_else(|| LispError::InvalidSyntax("deftemplate needs a name".into()))?,
                text: kw(args, "text").and_then(str_val).unwrap_or_default(),
                notes: kw(args, "notes").and_then(str_val).unwrap_or_default(),
                category: kw(args, "category").and_then(sym_or_str),
                priority: kw(args, "priority").and_then(int_val),
                recur: kw(args, "recur").and_then(recur_val),
            };
            agenda::deftemplate(&db.borrow(), t)
        });
    }
    {
        let db = db.clone();
        defb(env, "from-template", ArgMode::TableFirst, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("from-template needs a name".into()))?;
            let overrides = collect_kw_pairs(&args[1.min(args.len())..]);
            agenda::from_template(&db.borrow(), &name, &overrides)
        });
    }
    {
        let db = db.clone();
        defb(env, "templates", ArgMode::Eval, move |_, _| agenda::templates(&db.borrow()));
    }
    {
        let db = db.clone();
        defb(env, "drop-template", ArgMode::TableFirst, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("drop-template needs a name".into()))?;
            agenda::drop_template(&db.borrow(), &name)
        });
    }

    // ── categories ──
    {
        let db = db.clone();
        defb(env, "defcategory", ArgMode::AllRaw, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("defcategory needs a name".into()))?;
            let parent = kw(args, "parent").and_then(sym_or_str);
            let exclusive = kw(args, "exclusive").map(is_truthy).unwrap_or(false);
            let children: Vec<String> = match kw(args, "children") {
                Some(Value::List(l)) => l.iter().filter_map(sym_or_str).collect(),
                _ => vec![],
            };
            agenda::defcategory(&db.borrow(), &name, parent, exclusive, &children)
        });
    }
    {
        let db = db.clone();
        defb(env, "assign", ArgMode::Eval, move |args, _| {
            let id = int_arg(args.first())?;
            let cat = str_arg(args.get(1), "assign")?;
            agenda::assign(&db.borrow(), id, &cat)
        });
    }
    {
        let db = db.clone();
        defb(env, "unassign", ArgMode::Eval, move |args, _| {
            let id = int_arg(args.first())?;
            let cat = str_arg(args.get(1), "unassign")?;
            agenda::unassign(&db.borrow(), id, &cat)
        });
    }
    {
        let db = db.clone();
        defb(env, "categories", ArgMode::Eval, move |_, _| agenda::categories(&db.borrow()));
    }

    // ── rules ──
    {
        let db = db.clone();
        defb(env, "defrule", ArgMode::AllRaw, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("defrule needs a name".into()))?;
            let condition = kw(args, "when").cloned().unwrap_or(Value::Bool(false));
            let mut actions: Vec<Value> = Vec::new();
            for cat in kw_all(args, "assign") {
                if let Some(c) = str_val(cat) {
                    actions.push(assign_action(&c));
                }
            }
            for a in kw_all(args, "action") {
                actions.push(a.clone());
            }
            agenda::defrule(&db.borrow(), &name, condition, actions, true)
        });
    }
    {
        let db = db.clone();
        defb(env, "apply-rules", ArgMode::Eval, move |args, ev| {
            let id = match args.first() {
                Some(Value::Number(n)) => Some(*n as i64),
                _ => None,
            };
            agenda::apply_rules(&db, ev, id)
        });
    }
    {
        let reg = reg.clone();
        defb(env, "auto-categorize", ArgMode::Eval, move |args, _| {
            let on = args.first().map(is_truthy).unwrap_or(false);
            reg.borrow_mut().auto_categorize = on;
            Ok(Value::Bool(on))
        });
    }
    {
        let db = db.clone();
        defb(env, "rules", ArgMode::Eval, move |_, _| agenda::list_rules(&db.borrow()));
    }
    {
        let db = db.clone();
        defb(env, "drop-rule", ArgMode::Eval, move |args, _| {
            agenda::drop_rule(&db.borrow(), &str_arg(args.first(), "drop-rule")?)
        });
    }

    // ── views ──
    {
        let db = db.clone();
        defb(env, "defview", ArgMode::AllRaw, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("defview needs a name".into()))?;
            let filter = kw(args, "filter").cloned();
            let sort_by = kw(args, "sort-by").and_then(sym_or_str);
            let group_by = kw(args, "group-by").and_then(sym_or_str);
            let sort_asc = !kw(args, "desc").map(is_truthy).unwrap_or(false);
            agenda::defview(&db.borrow(), &name, filter, sort_by, group_by, sort_asc)
        });
    }
    {
        let db = db.clone();
        defb(env, "show", ArgMode::TableFirst, move |args, ev| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("show needs a view name".into()))?;
            agenda::show(&db.borrow(), ev, &name)
        });
    }
    {
        let db = db.clone();
        defb(env, "views", ArgMode::Eval, move |_, _| agenda::list_views(&db.borrow()));
    }
    {
        let db = db.clone();
        defb(env, "drop-view", ArgMode::Eval, move |args, _| {
            agenda::drop_view(&db.borrow(), &str_arg(args.first(), "drop-view")?)
        });
    }

    // ── smart input ──
    {
        let db = db.clone();
        defb(env, "add", ArgMode::Eval, move |args, _| {
            agenda::add_smart(&db.borrow(), &str_arg(args.first(), "add")?)
        });
    }
    defb(env, "smart-parse", ArgMode::Eval, |args, _| {
        Ok(agenda::smart_parse_dict(&str_arg(args.first(), "smart-parse")?))
    });

    // ── multi-agenda ──
    {
        let reg = reg.clone();
        defb(env, "agendas", ArgMode::Eval, move |_, _| {
            let r = reg.borrow();
            let mut lines = vec![format!("{} [active]", r.active_name)];
            let mut names: Vec<&String> = r.inactive.keys().collect();
            names.sort();
            lines.extend(names.into_iter().cloned());
            Ok(Value::Str(lines.join("\n")))
        });
    }
    {
        let db = db.clone();
        let reg = reg.clone();
        defb(env, "open-agenda", ArgMode::Eval, move |args, _| {
            let path = str_arg(args.first(), "open-agenda")?;
            let name = agenda::agenda_name_from_path(&path);
            let mut r = reg.borrow_mut();
            if r.active_name == name {
                return Ok(Value::Str(format!("Already using agenda: {}", name)));
            }
            let target = match r.inactive.remove(&name) {
                Some(existing) => existing,
                None => {
                    let mut d = Database::open(&path)?;
                    agenda::ensure_agenda_tables(&mut d)?;
                    d
                }
            };
            let old = std::mem::replace(&mut *db.borrow_mut(), target);
            let old_name = std::mem::replace(&mut r.active_name, name.clone());
            r.inactive.insert(old_name, old);
            Ok(Value::Str(format!("Opened agenda: {}", name)))
        });
    }
    {
        let db = db.clone();
        let reg = reg.clone();
        defb(env, "use-agenda", ArgMode::TableFirst, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("use-agenda needs a name".into()))?;
            let mut r = reg.borrow_mut();
            if r.active_name == name {
                return Ok(Value::Str(format!("Already using agenda: {}", name)));
            }
            match r.inactive.remove(&name) {
                Some(target) => {
                    let old = std::mem::replace(&mut *db.borrow_mut(), target);
                    let old_name = std::mem::replace(&mut r.active_name, name.clone());
                    r.inactive.insert(old_name, old);
                    Ok(Value::Str(format!("Switched to agenda: {}", name)))
                }
                None => {
                    let mut avail: Vec<&String> = r.inactive.keys().collect();
                    avail.push(&r.active_name);
                    avail.sort();
                    Err(LispError::Runtime(format!(
                        "Agenda not found: {}. Available: {}",
                        name,
                        avail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                    )))
                }
            }
        });
    }
    {
        let db = db.clone();
        let reg = reg.clone();
        defb(env, "close-agenda", ArgMode::TableFirst, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("close-agenda needs a name".into()))?;
            let mut r = reg.borrow_mut();
            if r.active_name == name {
                // pick a replacement to activate (prefer "memory")
                let next = if r.inactive.contains_key("memory") {
                    Some("memory".to_string())
                } else {
                    r.inactive.keys().next().cloned()
                };
                match next {
                    Some(nn) => {
                        let target = r.inactive.remove(&nn).unwrap();
                        let _closed = std::mem::replace(&mut *db.borrow_mut(), target); // dropped
                        r.active_name = nn;
                        Ok(Value::Str(format!("Closed agenda: {}", name)))
                    }
                    None => Err(LispError::Runtime("cannot close the only open agenda".into())),
                }
            } else if r.inactive.remove(&name).is_some() {
                Ok(Value::Str(format!("Closed agenda: {}", name)))
            } else {
                Err(LispError::Runtime(format!("Agenda not found: {}", name)))
            }
        });
    }
    {
        let db = db.clone();
        let reg = reg.clone();
        defb(env, "export-agenda", ArgMode::Eval, move |args, _| {
            let name = str_arg(args.first(), "export-agenda")?;
            let path = kw(args, "path")
                .and_then(str_val)
                .ok_or_else(|| LispError::Runtime("export-agenda needs :path".into()))?;
            let r = reg.borrow();
            let json = if r.active_name == name {
                agenda::export_json(&db.borrow(), &name)?
            } else if let Some(other) = r.inactive.get(&name) {
                agenda::export_json(other, &name)?
            } else {
                return Err(LispError::Runtime(format!("Agenda not found: {}", name)));
            };
            std::fs::write(&path, json).map_err(|e| LispError::Runtime(e.to_string()))?;
            Ok(Value::Str(format!("Exported agenda '{}' to {}", name, path)))
        });
    }
    {
        let db = db.clone();
        defb(env, "import-agenda", ArgMode::Eval, move |args, _| {
            let path = str_arg(args.first(), "import-agenda")?;
            let json = std::fs::read_to_string(&path).map_err(|e| LispError::Runtime(e.to_string()))?;
            agenda::import_json(&db.borrow(), &json)
        });
    }
}

// ── argument helpers ──────────────────────────────────────────────────

fn maybe_auto_categorize(db: &Db, reg: &Reg, env: &Env, result: &Value) -> Result<(), LispError> {
    if reg.borrow().auto_categorize {
        if let Value::Item(it) = result {
            agenda::apply_rules(db, env, Some(it.id))?;
        }
    }
    Ok(())
}

fn assign_action(cat: &str) -> Value {
    Value::List(Rc::new(vec![
        Value::Symbol("assign".into()),
        Value::Symbol("id".into()),
        Value::Str(cat.to_string()),
    ]))
}

/// Every value following an occurrence of keyword `:key`.
fn kw_all<'a>(args: &'a [Value], key: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if let Value::Keyword(k) = &args[i] {
            if k == key {
                out.push(&args[i + 1]);
            }
        }
        i += 1;
    }
    out
}

/// Find the value following keyword `:key` in an argument list.
fn kw<'a>(args: &'a [Value], key: &str) -> Option<&'a Value> {
    let mut i = 0;
    while i + 1 < args.len() {
        if let Value::Keyword(k) = &args[i] {
            if k == key {
                return Some(&args[i + 1]);
            }
        }
        i += 1;
    }
    None
}

fn collect_kw_pairs(args: &[Value]) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if let Value::Keyword(k) = &args[i] {
            out.push((k.clone(), args[i + 1].clone()));
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

fn str_val(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn sym_or_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) | Value::Symbol(s) => Some(s.clone()),
        _ => None,
    }
}

fn int_val(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => Some(*n as i64),
        _ => None,
    }
}

fn recur_val(v: &Value) -> Option<String> {
    match v {
        Value::Keyword(k) | Value::Symbol(k) => Some(k.clone()),
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn str_arg(v: Option<&Value>, who: &str) -> Result<String, LispError> {
    match v {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(LispError::InvalidSyntax(format!("{} expects a string", who))),
    }
}

fn int_arg(v: Option<&Value>) -> Result<i64, LispError> {
    match v {
        Some(Value::Number(n)) => Ok(*n as i64),
        other => Err(LispError::TypeMismatch {
            expected: "number".into(),
            got: other.map(type_name).unwrap_or_else(|| "nil".into()),
        }),
    }
}
