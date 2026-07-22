//! dBASE-style database builtins (ANALYSIS §4.4). Each closure captures the shared `Database`.
//!
//! Table commands use `ArgMode::TableFirst` / `AllRaw` so the table name and schema forms are
//! not evaluated (a bare `contacts` symbol must not be looked up). Record accessors
//! (field-get / field-set / record-id / records) are ordinary evaluated builtins.

use std::cell::RefCell;
use std::rc::Rc;

use crate::database::{Database, Query};
use crate::env::{self, Env};
use crate::value::*;

type Shared = Rc<RefCell<Database>>;

fn define_db(
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

pub fn register(env: &Env, db: Shared) {
    {
        let db = db.clone();
        define_db(env, "deftable", ArgMode::AllRaw, move |args, _| {
            let def = parse_table_def(args)?;
            db.borrow_mut().create_table(def)
        });
    }
    {
        let db = db.clone();
        define_db(env, "insert", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            let data = as_dict(args.get(1))?;
            db.borrow().insert(&table, &data)
        });
    }
    {
        let db = db.clone();
        define_db(env, "query", ArgMode::TableFirst, move |args, _| {
            let q = parse_query(args)?;
            db.borrow().query(&q)
        });
    }
    {
        let db = db.clone();
        define_db(env, "update", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            let id = as_int(args.get(1))?;
            let data = as_dict(args.get(2))?;
            db.borrow().update(&table, id, &data)
        });
    }
    {
        let db = db.clone();
        define_db(env, "delete", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            let id = as_int(args.get(1))?;
            db.borrow().delete(&table, id)
        });
    }
    {
        let db = db.clone();
        define_db(env, "count-records", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            let (where_, params) = parse_where_params(&args[1..]);
            db.borrow().count(&table, where_.as_deref(), &params)
        });
    }
    {
        let db = db.clone();
        define_db(env, "pack", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            db.borrow().pack(&table)
        });
    }
    {
        let db = db.clone();
        define_db(env, "describe", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            db.borrow()
                .get_def(&table)
                .map(|d| Value::Table(Rc::new(d)))
                .ok_or_else(|| LispError::Database(format!("Table not found: {}", table)))
        });
    }
    {
        let db = db.clone();
        define_db(env, "drop-table", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            db.borrow_mut().drop_table(&table)
        });
    }
    {
        let db = db.clone();
        define_db(env, "tables", ArgMode::Eval, move |_, _| {
            Ok(Value::List(Rc::new(
                db.borrow().list_tables().into_iter().map(Value::Symbol).collect(),
            )))
        });
    }

    // ── interactive views (ANALYSIS §5) ──
    {
        let db = db.clone();
        define_db(env, "browse", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            let q = parse_query(args)?;
            let d = db.borrow();
            let def = d.get_def(&table).ok_or_else(|| LispError::Database(format!("Table not found: {}", table)))?;
            let records = d.query_rows(&q)?;
            let columns = def.fields.iter().map(|f| f.name.clone()).collect();
            let rs = ResultSet { table: table.clone(), records, columns };
            Ok(Value::TableView(Rc::new(TableView { table_name: table, table_def: def, result_set: rs })))
        });
    }
    {
        let db = db.clone();
        define_db(env, "edit", ArgMode::TableFirst, move |args, _| {
            let table = table_name(args.first())?;
            let q = parse_query(args)?;
            let d = db.borrow();
            let def = d.get_def(&table).ok_or_else(|| LispError::Database(format!("Table not found: {}", table)))?;
            let records = d.query_rows(&q)?;
            let columns = def.fields.iter().map(|f| f.name.clone()).collect();
            let rs = ResultSet { table: table.clone(), records, columns };
            Ok(Value::FormView(Rc::new(FormView {
                table_name: table,
                table_def: def,
                result_set: rs,
                computed_fields: vec![],
                is_standalone: false,
            })))
        });
    }
    {
        let db = db.clone();
        define_db(env, "defform", ArgMode::AllRaw, move |args, _| {
            let name = sym_or_str(args.first().unwrap_or(&Value::Null))
                .ok_or_else(|| LispError::InvalidSyntax("defform needs a name".into()))?;
            let form_fields: Vec<FieldDef> = match args.get(1) {
                Some(Value::List(l)) => l.iter().filter_map(parse_form_field).collect(),
                _ => vec![],
            };
            let computed = parse_computed(kw(args, "computed"));
            match kw(args, "source").and_then(sym_or_str) {
                Some(src) => {
                    let d = db.borrow();
                    let def = d.get_def(&src).ok_or_else(|| LispError::Database(format!("Table not found: {}", src)))?;
                    let q = Query { table: src.clone(), where_: None, params: vec![], order: None, ascending: true, limit: None, select: None };
                    let records = d.query_rows(&q)?;
                    let columns = def.fields.iter().map(|f| f.name.clone()).collect();
                    let rs = ResultSet { table: src, records, columns };
                    Ok(Value::FormView(Rc::new(FormView {
                        table_name: name,
                        table_def: def,
                        result_set: rs,
                        computed_fields: computed,
                        is_standalone: false,
                    })))
                }
                None => {
                    let def = TableDef { name: name.clone(), fields: form_fields };
                    let blank = blank_record(&name, &def);
                    let columns = def.fields.iter().map(|f| f.name.clone()).collect();
                    let rs = ResultSet { table: name.clone(), records: vec![blank], columns };
                    Ok(Value::FormView(Rc::new(FormView {
                        table_name: name,
                        table_def: def,
                        result_set: rs,
                        computed_fields: computed,
                        is_standalone: true,
                    })))
                }
            }
        });
    }

    // ── record accessors — ordinary evaluated builtins ──
    define_db(env, "field-get", ArgMode::Eval, |args, _| {
        let rec = as_record(args.first())?;
        let key = as_key(args.get(1))?;
        Ok(rec.data.get(&key).cloned().unwrap_or(Value::Null))
    });
    define_db(env, "field-set", ArgMode::Eval, |args, _| {
        let rec = as_record(args.first())?;
        let key = as_key(args.get(1))?;
        let val = args.get(2).cloned().unwrap_or(Value::Null);
        let mut new = (*rec).clone();
        new.data.insert(key, val);
        Ok(Value::Record(Rc::new(new)))
    });
    define_db(env, "record-id", ArgMode::Eval, |args, _| {
        let rec = as_record(args.first())?;
        Ok(Value::Number(rec.id as f64))
    });
    define_db(env, "records", ArgMode::Eval, |args, _| match args.first() {
        Some(Value::ResultSet(rs)) => Ok(Value::List(Rc::new(
            rs.records.iter().map(|r| Value::Record(Rc::new(r.clone()))).collect(),
        ))),
        other => Err(LispError::TypeMismatch {
            expected: "result-set".into(),
            got: other.map(type_name).unwrap_or_else(|| "nil".into()),
        }),
    });
}

// ── argument helpers ──────────────────────────────────────────────

fn sym_or_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) | Value::Symbol(s) => Some(s.clone()),
        _ => None,
    }
}

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

fn table_name(v: Option<&Value>) -> Result<String, LispError> {
    match v {
        Some(Value::Symbol(s)) | Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(LispError::InvalidSyntax("expected a table name".into())),
    }
}

fn as_dict(v: Option<&Value>) -> Result<OrderedDict, LispError> {
    match v {
        Some(Value::Dict(d)) => Ok((**d).clone()),
        other => Err(LispError::TypeMismatch {
            expected: "dict".into(),
            got: other.map(type_name).unwrap_or_else(|| "nil".into()),
        }),
    }
}

fn as_int(v: Option<&Value>) -> Result<i64, LispError> {
    match v {
        Some(Value::Number(n)) => Ok(*n as i64),
        other => Err(LispError::TypeMismatch {
            expected: "number".into(),
            got: other.map(type_name).unwrap_or_else(|| "nil".into()),
        }),
    }
}

fn as_record(v: Option<&Value>) -> Result<Rc<Record>, LispError> {
    match v {
        Some(Value::Record(r)) => Ok(r.clone()),
        other => Err(LispError::TypeMismatch {
            expected: "record".into(),
            got: other.map(type_name).unwrap_or_else(|| "nil".into()),
        }),
    }
}

fn as_key(v: Option<&Value>) -> Result<String, LispError> {
    match v {
        Some(Value::Keyword(k)) | Some(Value::Symbol(k)) => Ok(k.clone()),
        _ => Err(LispError::InvalidSyntax("expected a keyword".into())),
    }
}

fn parse_table_def(args: &[Value]) -> Result<TableDef, LispError> {
    let name = table_name(args.first())?;
    let fields_list = match args.get(1) {
        Some(Value::List(l)) => l.clone(),
        _ => return Err(LispError::InvalidSyntax("deftable expects a field list".into())),
    };
    let mut fields = Vec::new();
    for f in fields_list.iter() {
        fields.push(parse_field(f)?);
    }
    Ok(TableDef { name, fields })
}

fn parse_field(v: &Value) -> Result<FieldDef, LispError> {
    match v {
        // short form:  name:type
        Value::Symbol(s) => {
            let mut parts = s.splitn(2, ':');
            let name = parts.next().unwrap_or("").to_string();
            let ty = parts.next().unwrap_or("string");
            Ok(FieldDef { name, ftype: FieldType::parse(ty), required: false, default: None, choices: vec![] })
        }
        // long form:  (name :type T :required B :default V :choices (...))
        Value::List(l) => {
            let name = match l.first() {
                Some(Value::Symbol(s)) => s.clone(),
                _ => return Err(LispError::InvalidSyntax("bad field def".into())),
            };
            let mut fd =
                FieldDef { name, ftype: FieldType::String, required: false, default: None, choices: vec![] };
            let mut i = 1;
            while i + 1 < l.len() {
                if let Value::Keyword(k) = &l[i] {
                    let val = &l[i + 1];
                    match k.as_str() {
                        "type" => {
                            if let Value::Symbol(t) | Value::Str(t) = val {
                                fd.ftype = FieldType::parse(t);
                            }
                        }
                        "required" => fd.required = is_truthy(val),
                        "default" => fd.default = Some(val.clone()),
                        "choices" => {
                            if let Value::List(cs) = val {
                                fd.choices = cs
                                    .iter()
                                    .filter_map(|c| match c {
                                        Value::Str(s) | Value::Symbol(s) => Some(s.clone()),
                                        _ => None,
                                    })
                                    .collect();
                            }
                        }
                        _ => {}
                    }
                }
                i += 2;
            }
            Ok(fd)
        }
        _ => Err(LispError::InvalidSyntax("bad field def".into())),
    }
}

/// defform field: `name:type` (symbol) or `(name:choice "a" "b")` (list).
fn parse_form_field(v: &Value) -> Option<FieldDef> {
    match v {
        Value::Symbol(s) => {
            let mut parts = s.splitn(2, ':');
            let name = parts.next().unwrap_or("").to_string();
            let ty = parts.next().unwrap_or("string");
            Some(FieldDef { name, ftype: FieldType::parse(ty), required: false, default: None, choices: vec![] })
        }
        Value::List(l) => {
            if let Some(Value::Symbol(head)) = l.first() {
                let mut parts = head.splitn(2, ':');
                let name = parts.next().unwrap_or("").to_string();
                let ty = parts.next().unwrap_or("string");
                let choices = l.iter().skip(1).filter_map(|c| match c {
                    Value::Str(s) => Some(s.clone()),
                    _ => None,
                }).collect();
                Some(FieldDef { name, ftype: FieldType::parse(ty), required: false, default: None, choices })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_computed(v: Option<&Value>) -> Vec<ComputedField> {
    match v {
        Some(Value::List(pairs)) => pairs
            .iter()
            .filter_map(|p| {
                if let Value::List(pair) = p {
                    if let Some(Value::Symbol(n)) = pair.first() {
                        return Some(ComputedField {
                            name: n.clone(),
                            ftype: FieldType::Number,
                            expression: pair.get(1).cloned().unwrap_or(Value::Null),
                        });
                    }
                }
                None
            })
            .collect(),
        _ => vec![],
    }
}

fn blank_record(table: &str, def: &TableDef) -> Record {
    let mut data = OrderedDict::default();
    for f in &def.fields {
        let v = match f.ftype {
            FieldType::Number => Value::Number(0.0),
            FieldType::Bool => Value::Bool(false),
            FieldType::Choice => Value::Str(f.choices.first().cloned().unwrap_or_default()),
            _ => Value::Str(String::new()),
        };
        data.insert(f.name.clone(), v);
    }
    Record { table: table.to_string(), id: 0, data, deleted: false }
}

fn parse_query(args: &[Value]) -> Result<Query, LispError> {
    let table = table_name(args.first())?;
    let mut q = Query {
        table,
        where_: None,
        params: vec![],
        order: None,
        ascending: true,
        limit: None,
        select: None,
    };
    let rest = &args[1..];
    let mut i = 0;
    while i < rest.len() {
        if let Value::Keyword(k) = &rest[i] {
            let val = rest.get(i + 1);
            match k.as_str() {
                "where" => {
                    if let Some(Value::Str(s)) = val {
                        q.where_ = Some(s.clone());
                    }
                }
                "params" => {
                    if let Some(Value::List(l)) = val {
                        q.params = l.iter().cloned().collect();
                    }
                }
                "order" => match val {
                    Some(Value::Str(s)) | Some(Value::Symbol(s)) => q.order = Some(s.clone()),
                    _ => {}
                },
                "asc" => {
                    if let Some(v) = val {
                        q.ascending = is_truthy(v);
                    }
                }
                "desc" => {
                    if let Some(v) = val {
                        q.ascending = !is_truthy(v);
                    }
                }
                "limit" => {
                    if let Some(Value::Number(n)) = val {
                        q.limit = Some(*n as i64);
                    }
                }
                "select" => {
                    if let Some(Value::List(l)) = val {
                        q.select = Some(
                            l.iter()
                                .filter_map(|x| match x {
                                    Value::Symbol(s) | Value::Str(s) | Value::Keyword(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .collect(),
                        );
                    }
                }
                _ => {}
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(q)
}

fn parse_where_params(args: &[Value]) -> (Option<String>, Vec<Value>) {
    let mut where_ = None;
    let mut params = vec![];
    let mut i = 0;
    while i < args.len() {
        if let Value::Keyword(k) = &args[i] {
            match k.as_str() {
                "where" => {
                    if let Some(Value::Str(s)) = args.get(i + 1) {
                        where_ = Some(s.clone());
                    }
                }
                "params" => {
                    if let Some(Value::List(l)) = args.get(i + 1) {
                        params = l.iter().cloned().collect();
                    }
                }
                _ => {}
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    (where_, params)
}
