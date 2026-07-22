//! Agenda PIM core (ANALYSIS §4.10) — items, recurrence, templates, and multi-agenda, built on
//! the `Database` layer. Categories / rules / views (the reflective-eval "magic") are the
//! remaining step-3b slice; the 5 system tables are still created so `tables` matches EELisp.
//!
//! Property/category columns are hand-modeled as JSON (via serde_json). Property values round-trip
//! as strings, matching EELisp semantics (`(= priority "1")`, not `1`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::database::{Database, Query};
use crate::env::{self, Env};
use crate::value::*;

// ── multi-agenda registry ────────────────────────────────────────────

/// Holds the inactive agendas by name; the active one lives in the interpreter's `Rc<RefCell<Database>>`.
pub struct Agendas {
    pub active_name: String,
    pub inactive: HashMap<String, Database>,
    pub auto_categorize: bool,
}

impl Agendas {
    pub fn new(active_name: String) -> Self {
        Agendas { active_name, inactive: HashMap::new(), auto_categorize: false }
    }
}

pub fn agenda_name_from_path(path: &str) -> String {
    if path == ":memory:" {
        return "memory".to_string();
    }
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("agenda")
        .to_string()
}

// ── system tables ────────────────────────────────────────────────────

fn field(name: &str, ftype: FieldType, required: bool) -> FieldDef {
    FieldDef { name: name.to_string(), ftype, required, default: None, choices: vec![] }
}

pub fn ensure_agenda_tables(db: &mut Database) -> Result<(), LispError> {
    use FieldType::*;
    let tables = [
        TableDef {
            name: "_items".into(),
            fields: vec![
                field("text", String, true),
                field("notes", Memo, false),
                field("categories", String, false),
                field("properties", String, false),
                field("created", String, false),
                field("modified", String, false),
            ],
        },
        TableDef {
            name: "_categories".into(),
            fields: vec![
                field("name", String, true),
                field("parent", String, false),
                field("exclusive", Bool, false),
                field("conditions", String, false),
            ],
        },
        TableDef {
            name: "_rules".into(),
            fields: vec![
                field("name", String, true),
                field("condition", Memo, false),
                field("actions", Memo, false),
                field("enabled", Bool, false),
            ],
        },
        TableDef {
            name: "_views".into(),
            fields: vec![
                field("name", String, true),
                field("filter", Memo, false),
                field("group_by", String, false),
                field("sort_by", String, false),
                field("sort_asc", Bool, false),
                field("columns", String, false),
            ],
        },
        TableDef {
            name: "_templates".into(),
            fields: vec![
                field("name", String, true),
                field("text_template", String, true),
                field("notes", Memo, false),
                field("categories", String, false),
                field("properties", String, false),
                field("created", String, false),
                field("modified", String, false),
            ],
        },
    ];
    for t in tables {
        db.create_table(t)?;
    }
    Ok(())
}

// ── JSON (de)serialization for the categories / properties columns ────

fn serialize_cats(cats: &[String]) -> String {
    serde_json::Value::Array(cats.iter().map(|c| serde_json::Value::String(c.clone())).collect())
        .to_string()
}

fn parse_cats(s: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| {
            v.as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        })
        .unwrap_or_default()
}

fn serialize_props(props: &OrderedDict) -> String {
    let mut map = serde_json::Map::new();
    for k in &props.keys {
        let sv = match &props.map[k] {
            Value::Str(s) => s.clone(),
            other => crate::printer::print_value(other, false),
        };
        map.insert(k.clone(), serde_json::Value::String(sv));
    }
    serde_json::Value::Object(map).to_string()
}

fn parse_props(s: &str) -> OrderedDict {
    let mut od = OrderedDict::default();
    if let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(s) {
        for (k, v) in m {
            let sv = match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            od.insert(k, Value::Str(sv));
        }
    }
    od
}

// ── small helpers ────────────────────────────────────────────────────

fn sval(od: &OrderedDict, k: &str) -> String {
    match od.get(k) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => crate::printer::print_value(other, false),
    }
}

fn record_to_item(r: &Record) -> Item {
    Item {
        id: r.id,
        text: sval(&r.data, "text"),
        notes: sval(&r.data, "notes"),
        categories: parse_cats(&sval(&r.data, "categories")),
        properties: parse_props(&sval(&r.data, "properties")),
        created: sval(&r.data, "created"),
        modified: sval(&r.data, "modified"),
    }
}

fn item_row(text: &str, notes: &str, cats: &[String], props: &OrderedDict, created: &str, modified: &str) -> OrderedDict {
    let mut d = OrderedDict::default();
    d.insert("text".into(), Value::Str(text.to_string()));
    d.insert("notes".into(), Value::Str(notes.to_string()));
    d.insert("categories".into(), Value::Str(serialize_cats(cats)));
    d.insert("properties".into(), Value::Str(serialize_props(props)));
    d.insert("created".into(), Value::Str(created.to_string()));
    d.insert("modified".into(), Value::Str(modified.to_string()));
    d
}

fn item_value(rec: Value) -> Value {
    match &rec {
        Value::Record(r) => Value::Item(Rc::new(record_to_item(r))),
        _ => rec,
    }
}

fn read_item(db: &Database, id: i64) -> Result<Option<Item>, LispError> {
    let q = Query {
        table: "_items".into(),
        where_: Some("_id = ?".into()),
        params: vec![Value::Number(id as f64)],
        order: None,
        ascending: true,
        limit: Some(1),
        select: None,
    };
    Ok(db.query_rows(&q)?.first().map(record_to_item))
}

// ── item operations ──────────────────────────────────────────────────

pub struct ItemInput {
    pub text: String,
    pub notes: String,
    pub when: Option<String>,
    pub priority: Option<i64>,
    pub category: Option<String>,
    pub recur: Option<String>,
}

fn props_from_input(when: &Option<String>, priority: Option<i64>, recur: &Option<String>) -> OrderedDict {
    let mut p = OrderedDict::default();
    if let Some(w) = when {
        p.insert("when".into(), Value::Str(w.clone()));
    }
    if let Some(pr) = priority {
        p.insert("priority".into(), Value::Str(pr.to_string()));
    }
    if let Some(r) = recur {
        p.insert("recurrence".into(), Value::Str(r.clone()));
    }
    p
}

pub fn add_item(db: &Database, input: ItemInput) -> Result<Value, LispError> {
    let props = props_from_input(&input.when, input.priority, &input.recur);
    let cats: Vec<String> = input.category.into_iter().collect();
    let now = iso_now();
    let row = item_row(&input.text, &input.notes, &cats, &props, &now, &now);
    Ok(item_value(db.insert("_items", &row)?))
}

pub fn item_count(db: &Database, category: Option<&str>) -> Result<Value, LispError> {
    match category {
        Some(c) => db.count("_items", Some("categories LIKE ?"), &[Value::Str(format!("%\"{}\"%", c))]),
        None => db.count("_items", None, &[]),
    }
}

pub fn item_get(db: &Database, id: i64) -> Result<Value, LispError> {
    match read_item(db, id)? {
        Some(it) => Ok(Value::Item(Rc::new(it))),
        None => Err(LispError::Runtime(format!("Item not found: {}", id))),
    }
}

pub fn item_done(db: &Database, id: i64) -> Result<Value, LispError> {
    let item = match read_item(db, id)? {
        Some(it) => it,
        None => return Ok(Value::Bool(false)),
    };
    let recurrence = item.properties.get("recurrence").and_then(as_string);
    let when = item.properties.get("when").and_then(as_string);
    db.delete("_items", id)?; // soft-delete

    if let (Some(rec), Some(w)) = (recurrence, when) {
        if let Some(next) = advance_date(&w, &rec) {
            let mut props = item.properties.clone();
            props.insert("when".into(), Value::Str(next));
            let now = iso_now();
            let row = item_row(&item.text, &item.notes, &item.categories, &props, &now, &now);
            return Ok(item_value(db.insert("_items", &row)?));
        }
    }
    Ok(Value::Bool(true))
}

/// (item-set id :text .. :notes .. :when .. :priority .. :other ..)
pub fn item_set(db: &Database, id: i64, updates: &[(String, Value)]) -> Result<Value, LispError> {
    let mut item = match read_item(db, id)? {
        Some(it) => it,
        None => return Err(LispError::Runtime(format!("Item not found: {}", id))),
    };
    for (key, val) in updates {
        match key.as_str() {
            "text" => item.text = as_string(val).unwrap_or_default(),
            "notes" => item.notes = as_string(val).unwrap_or_default(),
            _ => {
                let sv = as_string(val).unwrap_or_else(|| crate::printer::print_value(val, false));
                item.properties.insert(key.clone(), Value::Str(sv));
            }
        }
    }
    let now = iso_now();
    let mut row = item_row(&item.text, &item.notes, &item.categories, &item.properties, &item.created, &now);
    row.keys.retain(|k| k != "created"); // don't overwrite created on update
    row.map.remove("created");
    db.update("_items", id, &row)?;
    Ok(Value::Item(Rc::new(item)))
}

/// Filters for `(items ...)`. Returns a display result-set (id + text/when/priority/categories).
pub struct ItemFilter {
    pub search: Option<String>,
    pub category: Option<String>,
    pub priority: Option<i64>,
    pub when_before: Option<String>,
    pub when_after: Option<String>,
}

pub fn items(db: &Database, f: &ItemFilter) -> Result<Value, LispError> {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if let Some(s) = &f.search {
        clauses.push("(text LIKE ? OR notes LIKE ?)".into());
        params.push(Value::Str(format!("%{}%", s)));
        params.push(Value::Str(format!("%{}%", s)));
    }
    if let Some(c) = &f.category {
        clauses.push("categories LIKE ?".into());
        params.push(Value::Str(format!("%\"{}\"%", c)));
    }
    if let Some(p) = f.priority {
        clauses.push("properties LIKE ?".into());
        params.push(Value::Str(format!("%\"priority\":\"{}\"%", p)));
    }
    if let Some(d) = &f.when_before {
        clauses.push("json_extract(properties,'$.when') < ?".into());
        params.push(Value::Str(d.clone()));
    }
    if let Some(d) = &f.when_after {
        clauses.push("json_extract(properties,'$.when') >= ?".into());
        params.push(Value::Str(d.clone()));
    }
    let where_ = if clauses.is_empty() { None } else { Some(clauses.join(" AND ")) };
    query_items_display(db, where_, params, Some("created".into()), false)
}

pub fn items_on(db: &Database, date: &str) -> Result<Value, LispError> {
    query_items_display(
        db,
        Some("json_extract(properties,'$.when') = ?".into()),
        vec![Value::Str(date.to_string())],
        Some("created".into()),
        false,
    )
}

pub fn items_between(db: &Database, start: &str, end: &str) -> Result<Value, LispError> {
    query_items_display(
        db,
        Some("json_extract(properties,'$.when') >= ? AND json_extract(properties,'$.when') <= ?".into()),
        vec![Value::Str(start.to_string()), Value::Str(end.to_string())],
        Some("json_extract(properties,'$.when')".into()),
        true,
    )
}

fn query_items_display(
    db: &Database,
    where_: Option<String>,
    params: Vec<Value>,
    order: Option<String>,
    ascending: bool,
) -> Result<Value, LispError> {
    let q = Query { table: "_items".into(), where_, params, order, ascending, limit: None, select: None };
    let rows = db.query_rows(&q)?;
    let columns =
        vec!["text".to_string(), "when".into(), "priority".into(), "categories".into(), "recurrence".into()];
    let records = rows
        .iter()
        .map(|r| {
            let item = record_to_item(r);
            let mut data = OrderedDict::default();
            data.insert("text".into(), Value::Str(item.text.clone()));
            data.insert("when".into(), Value::Str(sval(&item.properties, "when")));
            data.insert("priority".into(), Value::Str(sval(&item.properties, "priority")));
            data.insert("categories".into(), Value::Str(item.categories.join(", ")));
            data.insert("recurrence".into(), Value::Str(sval(&item.properties, "recurrence")));
            Record { table: "_items".into(), id: item.id, data, deleted: false }
        })
        .collect();
    Ok(Value::ResultSet(Rc::new(ResultSet { table: "_items".into(), records, columns })))
}

// ── templates ────────────────────────────────────────────────────────

pub struct TemplateInput {
    pub name: String,
    pub text: String,
    pub notes: String,
    pub category: Option<String>,
    pub priority: Option<i64>,
    pub recur: Option<String>,
}

pub fn deftemplate(db: &Database, t: TemplateInput) -> Result<Value, LispError> {
    let props = props_from_input(&None, t.priority, &t.recur);
    let cats: Vec<String> = t.category.into_iter().collect();
    let now = iso_now();
    let mut d = OrderedDict::default();
    d.insert("name".into(), Value::Str(t.name.clone()));
    d.insert("text_template".into(), Value::Str(t.text));
    d.insert("notes".into(), Value::Str(t.notes));
    d.insert("categories".into(), Value::Str(serialize_cats(&cats)));
    d.insert("properties".into(), Value::Str(serialize_props(&props)));
    d.insert("created".into(), Value::Str(now.clone()));
    d.insert("modified".into(), Value::Str(now));
    db.insert("_templates", &d)?;
    Ok(Value::Str(format!("Template defined: {}", t.name)))
}

/// (from-template name :when .. :priority .. :category .. :notes ..)
pub fn from_template(db: &Database, name: &str, overrides: &[(String, Value)]) -> Result<Value, LispError> {
    let q = Query {
        table: "_templates".into(),
        where_: Some("name = ?".into()),
        params: vec![Value::Str(name.to_string())],
        order: None,
        ascending: true,
        limit: Some(1),
        select: None,
    };
    let row = match db.query_rows(&q)?.into_iter().next() {
        Some(r) => r,
        None => return Err(LispError::Runtime(format!("Template not found: {}", name))),
    };
    let text = sval(&row.data, "text_template");
    let notes = sval(&row.data, "notes");
    let mut cats = parse_cats(&sval(&row.data, "categories"));
    let mut props = parse_props(&sval(&row.data, "properties"));

    for (k, v) in overrides {
        match k.as_str() {
            "when" => {
                props.insert("when".into(), Value::Str(as_string(v).unwrap_or_default()));
            }
            "priority" => {
                let p = match v {
                    Value::Number(n) => (*n as i64).to_string(),
                    other => as_string(other).unwrap_or_default(),
                };
                props.insert("priority".into(), Value::Str(p));
            }
            "category" => {
                if let Some(c) = as_string(v) {
                    if !cats.contains(&c) {
                        cats.push(c);
                    }
                }
            }
            "notes" => { /* handled below */ }
            _ => {}
        }
    }
    let notes = overrides
        .iter()
        .find(|(k, _)| k == "notes")
        .and_then(|(_, v)| as_string(v))
        .unwrap_or(notes);

    let now = iso_now();
    let drow = item_row(&text, &notes, &cats, &props, &now, &now);
    Ok(item_value(db.insert("_items", &drow)?))
}

pub fn templates(db: &Database) -> Result<Value, LispError> {
    let q = Query {
        table: "_templates".into(),
        where_: None,
        params: vec![],
        order: Some("name".into()),
        ascending: true,
        limit: None,
        select: None,
    };
    let rows = db.query_rows(&q)?;
    if rows.is_empty() {
        return Ok(Value::Str("(no templates defined)".into()));
    }
    let lines: Vec<String> = rows
        .iter()
        .map(|r| format!("{} — {}", sval(&r.data, "name"), sval(&r.data, "text_template")))
        .collect();
    Ok(Value::Str(lines.join("\n")))
}

pub fn drop_template(db: &Database, name: &str) -> Result<Value, LispError> {
    db.soft_delete_where("_templates", "name = ?", &[Value::Str(name.to_string())])?;
    Ok(Value::Null)
}

// ── export / import ──────────────────────────────────────────────────

const AGENDA_TABLES: [&str; 5] = ["_items", "_categories", "_rules", "_views", "_templates"];

fn record_to_json(r: &Record) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for k in &r.data.keys {
        let jv = match &r.data.map[k] {
            Value::Str(s) => serde_json::Value::String(s.clone()),
            Value::Number(n) => serde_json::json!(n),
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Null => serde_json::Value::Null,
            other => serde_json::Value::String(crate::printer::print_value(other, false)),
        };
        m.insert(k.clone(), jv);
    }
    serde_json::Value::Object(m)
}

pub fn export_json(db: &Database, agenda_name: &str) -> Result<String, LispError> {
    let mut tables = serde_json::Map::new();
    for t in AGENDA_TABLES {
        let q = Query { table: t.to_string(), where_: None, params: vec![], order: None, ascending: true, limit: None, select: None };
        let rows = db.query_rows(&q)?;
        let arr: Vec<serde_json::Value> = rows.iter().map(record_to_json).collect();
        tables.insert(t.to_string(), serde_json::Value::Array(arr));
    }
    let root = serde_json::json!({ "version": 1, "agenda": agenda_name, "tables": serde_json::Value::Object(tables) });
    serde_json::to_string_pretty(&root).map_err(|e| LispError::Runtime(e.to_string()))
}

pub fn import_json(db: &Database, json: &str) -> Result<Value, LispError> {
    db.begin()?;
    let r = import_json_inner(db, json);
    match &r {
        Ok(_) => db.commit()?,
        Err(_) => {
            let _ = db.rollback();
        }
    }
    r
}

fn import_json_inner(db: &Database, json: &str) -> Result<Value, LispError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| LispError::Runtime(e.to_string()))?;
    if v.get("version").and_then(|x| x.as_i64()) != Some(1) {
        return Err(LispError::Runtime("unsupported agenda export version".into()));
    }
    let tables = v.get("tables").and_then(|t| t.as_object()).ok_or_else(|| LispError::Runtime("missing tables".into()))?;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for t in AGENDA_TABLES {
        if let Some(arr) = tables.get(t).and_then(|x| x.as_array()) {
            for row in arr {
                if let Some(obj) = row.as_object() {
                    let mut d = OrderedDict::default();
                    for (k, jv) in obj {
                        d.insert(k.clone(), json_to_value(jv));
                    }
                    if db.insert(t, &d).is_ok() {
                        *counts.entry(t).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    Ok(Value::Str(format!(
        "Imported {} items, {} templates",
        counts.get("_items").copied().unwrap_or(0),
        counts.get("_templates").copied().unwrap_or(0),
    )))
}

fn json_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Null => Value::Null,
        other => Value::Str(other.to_string()),
    }
}

// ── shared item persistence ──────────────────────────────────────────

fn persist_item(db: &Database, item: &Item) -> Result<(), LispError> {
    let now = iso_now();
    let mut row = OrderedDict::default();
    row.insert("text".into(), Value::Str(item.text.clone()));
    row.insert("notes".into(), Value::Str(item.notes.clone()));
    row.insert("categories".into(), Value::Str(serialize_cats(&item.categories)));
    row.insert("properties".into(), Value::Str(serialize_props(&item.properties)));
    row.insert("modified".into(), Value::Str(now));
    db.update("_items", item.id, &row)?;
    Ok(())
}

fn load_all_items(db: &Database) -> Result<Vec<Item>, LispError> {
    let q = Query {
        table: "_items".into(),
        where_: None,
        params: vec![],
        order: Some("created".into()),
        ascending: true,
        limit: None,
        select: None,
    };
    Ok(db.query_rows(&q)?.iter().map(record_to_item).collect())
}

// ── categories ───────────────────────────────────────────────────────

fn category_row(db: &Database, name: &str) -> Result<Option<Record>, LispError> {
    let q = Query {
        table: "_categories".into(),
        where_: Some("name = ?".into()),
        params: vec![Value::Str(name.to_string())],
        order: None,
        ascending: true,
        limit: Some(1),
        select: None,
    };
    Ok(db.query_rows(&q)?.into_iter().next())
}

fn upsert_category(db: &Database, name: &str, parent: Option<&str>, exclusive: bool) -> Result<(), LispError> {
    db.soft_delete_where("_categories", "name = ?", &[Value::Str(name.to_string())])?;
    let mut d = OrderedDict::default();
    d.insert("name".into(), Value::Str(name.to_string()));
    d.insert("parent".into(), parent.map(|p| Value::Str(p.to_string())).unwrap_or(Value::Null));
    d.insert("exclusive".into(), Value::Bool(exclusive));
    d.insert("conditions".into(), Value::Str("[]".into()));
    db.insert("_categories", &d)?;
    Ok(())
}

fn ensure_category(db: &Database, name: &str) -> Result<(), LispError> {
    if category_row(db, name)?.is_none() {
        let parent = name.rfind('/').map(|i| name[..i].to_string());
        if let Some(p) = &parent {
            ensure_category(db, p)?;
        }
        upsert_category(db, name, parent.as_deref(), false)?;
    }
    Ok(())
}

pub fn defcategory(db: &Database, name: &str, parent: Option<String>, exclusive: bool, children: &[String]) -> Result<Value, LispError> {
    let parent = parent.or_else(|| name.rfind('/').map(|i| name[..i].to_string()));
    if let Some(p) = &parent {
        ensure_category(db, p)?;
    }
    upsert_category(db, name, parent.as_deref(), exclusive)?;
    for child in children {
        upsert_category(db, &format!("{}/{}", name, child), Some(name), false)?;
    }
    Ok(Value::Str(format!("Category defined: {}", name)))
}

fn is_exclusive(r: &Record) -> bool {
    matches!(r.data.get("exclusive"), Some(Value::Bool(true)) | Some(Value::Number(_)))
        && sval(&r.data, "exclusive") != "false"
        && sval(&r.data, "exclusive") != "0"
}

fn enforce_exclusivity(db: &Database, item: &mut Item, cat: &str) -> Result<(), LispError> {
    if let Some(idx) = cat.rfind('/') {
        let parent = &cat[..idx];
        if let Some(prow) = category_row(db, parent)? {
            if is_exclusive(&prow) {
                let q = Query {
                    table: "_categories".into(),
                    where_: Some("parent = ?".into()),
                    params: vec![Value::Str(parent.to_string())],
                    order: None,
                    ascending: true,
                    limit: None,
                    select: None,
                };
                let siblings: Vec<String> = db.query_rows(&q)?.iter().map(|s| sval(&s.data, "name")).collect();
                item.categories.retain(|c| c == cat || !siblings.contains(c));
            }
        }
    }
    Ok(())
}

pub fn assign(db: &Database, id: i64, cat: &str) -> Result<Value, LispError> {
    let mut item = read_item(db, id)?.ok_or_else(|| LispError::Runtime(format!("Item not found: {}", id)))?;
    enforce_exclusivity(db, &mut item, cat)?;
    if !item.categories.iter().any(|c| c == cat) {
        item.categories.push(cat.to_string());
    }
    persist_item(db, &item)?;
    Ok(Value::Item(Rc::new(item)))
}

pub fn unassign(db: &Database, id: i64, cat: &str) -> Result<Value, LispError> {
    let mut item = read_item(db, id)?.ok_or_else(|| LispError::Runtime(format!("Item not found: {}", id)))?;
    item.categories.retain(|c| c != cat);
    persist_item(db, &item)?;
    Ok(Value::Item(Rc::new(item)))
}

pub fn categories(db: &Database) -> Result<Value, LispError> {
    let q = Query {
        table: "_categories".into(),
        where_: None,
        params: vec![],
        order: Some("name".into()),
        ascending: true,
        limit: None,
        select: None,
    };
    let rows = db.query_rows(&q)?;
    if rows.is_empty() {
        return Ok(Value::Str("(no categories defined)".into()));
    }
    let lines: Vec<String> = rows
        .iter()
        .map(|r| {
            let name = sval(&r.data, "name");
            let depth = name.matches('/').count();
            let tag = if is_exclusive(r) { " [exclusive]" } else { "" };
            format!("{}{}{}", "  ".repeat(depth), name, tag)
        })
        .collect();
    Ok(Value::Str(lines.join("\n")))
}

// ── rules (the reflective engine) ─────────────────────────────────────

struct RuleDef {
    condition: Value,
    actions: Vec<Value>,
    enabled: bool,
}

fn make_builtin(name: &str, f: impl Fn(&[Value], &Env) -> Result<Value, LispError> + 'static) -> Value {
    Value::Builtin(Rc::new(Builtin { name: name.to_string(), arg_mode: ArgMode::Eval, func: Box::new(f) }))
}

/// Build a child env binding the item's fields + `get` / `has-category` / `overdue?` / `match`.
fn build_item_env(base: &Env, item: &Item, matches: Rc<RefCell<Vec<Value>>>) -> Env {
    let e = env::child(base);
    env::define(&e, "text", Value::Str(item.text.clone()));
    env::define(&e, "notes", Value::Str(item.notes.clone()));
    env::define(&e, "id", Value::Number(item.id as f64));
    env::define(
        &e,
        "categories",
        Value::List(Rc::new(item.categories.iter().cloned().map(Value::Str).collect())),
    );
    env::define(&e, "created", Value::Str(item.created.clone()));
    env::define(&e, "modified", Value::Str(item.modified.clone()));
    for k in &item.properties.keys {
        env::define(&e, k, item.properties.map[k].clone());
    }

    let props = item.properties.clone();
    env::define(&e, "get", make_builtin("get", move |args, _| {
        let key = match args.first() {
            Some(Value::Keyword(k)) | Some(Value::Symbol(k)) => k.clone(),
            _ => return Ok(Value::Null),
        };
        Ok(props.get(&key).cloned().unwrap_or(Value::Null))
    }));

    let cats = item.categories.clone();
    env::define(&e, "has-category", make_builtin("has-category", move |args, _| {
        let path = match args.first() {
            Some(Value::Str(s)) | Some(Value::Symbol(s)) => s.clone(),
            _ => return Ok(Value::Bool(false)),
        };
        let prefix = format!("{}/", path);
        Ok(Value::Bool(cats.iter().any(|c| *c == path || c.starts_with(&prefix))))
    }));

    let when = item.properties.get("when").and_then(as_string);
    env::define(&e, "overdue?", make_builtin("overdue?", move |_, _| {
        Ok(Value::Bool(matches_overdue(&when)))
    }));

    let mh = matches;
    env::define(&e, "match", make_builtin("match", move |args, _| {
        let n = match args.first() {
            Some(Value::Number(n)) => *n as usize,
            _ => 0,
        };
        Ok(mh.borrow().get(n + 1).cloned().unwrap_or(Value::Null))
    }));
    e
}

fn matches_overdue(when: &Option<String>) -> bool {
    match when {
        Some(w) => w.as_str() < today().as_str(),
        None => false,
    }
}

fn load_rules(db: &Database) -> Result<Vec<RuleDef>, LispError> {
    let q = Query {
        table: "_rules".into(),
        where_: None,
        params: vec![],
        order: Some("name".into()),
        ascending: true,
        limit: None,
        select: None,
    };
    let mut rules = Vec::new();
    for r in db.query_rows(&q)? {
        let condition = parse_one(&sval(&r.data, "condition")).unwrap_or(Value::Bool(false));
        let actions = sval(&r.data, "actions")
            .split("@@")
            .filter(|s| !s.is_empty())
            .filter_map(parse_one)
            .collect();
        let enabled = match r.data.get("enabled") {
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => *n != 0.0,
            _ => true,
        };
        rules.push(RuleDef { condition, actions, enabled });
    }
    Ok(rules)
}

fn parse_one(src: &str) -> Option<Value> {
    crate::parser::parse(src).ok().and_then(|mut fs| if fs.is_empty() { None } else { Some(fs.remove(0)) })
}

pub fn defrule(db: &Database, name: &str, condition: Value, actions: Vec<Value>, enabled: bool) -> Result<Value, LispError> {
    db.soft_delete_where("_rules", "name = ?", &[Value::Str(name.to_string())])?;
    let cond_src = crate::printer::print_value(&condition, true);
    let act_src: Vec<String> = actions.iter().map(|a| crate::printer::print_value(a, true)).collect();
    let mut d = OrderedDict::default();
    d.insert("name".into(), Value::Str(name.to_string()));
    d.insert("condition".into(), Value::Str(cond_src));
    d.insert("actions".into(), Value::Str(act_src.join("@@")));
    d.insert("enabled".into(), Value::Bool(enabled));
    db.insert("_rules", &d)?;
    Ok(Value::Str(format!("Rule defined: {}", name)))
}

/// Apply enabled rules to one item (`Some(id)`) or all items. Returns the count of changed items.
/// Wrapped in a transaction so the whole pass is atomic (and fast) — the many action writes commit
/// as one unit, and any hard failure rolls back cleanly.
pub fn apply_rules(db: &Rc<RefCell<Database>>, base: &Env, item_id: Option<i64>) -> Result<Value, LispError> {
    db.borrow().begin()?;
    let result = apply_rules_inner(db, base, item_id);
    match &result {
        Ok(_) => db.borrow().commit()?,
        Err(_) => {
            let _ = db.borrow().rollback();
        }
    }
    result
}

/// Actions (`assign`/`unassign`/`item-set`) self-persist; results sync back into the in-memory item
/// so later rules in the same pass see the changes. No DB borrow is held during evaluation.
/// A single action that errors is non-fatal (skipped), matching EELisp.
fn apply_rules_inner(db: &Rc<RefCell<Database>>, base: &Env, item_id: Option<i64>) -> Result<Value, LispError> {
    let (rules, mut items) = {
        let d = db.borrow();
        let rules = load_rules(&d)?;
        let items = match item_id {
            Some(id) => read_item(&d, id)?.into_iter().collect(),
            None => load_all_items(&d)?,
        };
        (rules, items)
    };

    let mut changed_count = 0;
    for item in items.iter_mut() {
        let mut changed = false;
        for rule in &rules {
            if !rule.enabled {
                continue;
            }
            let matches = Rc::new(RefCell::new(Vec::new()));
            let rule_env = build_item_env(base, item, matches.clone());
            let cond = match crate::eval::eval(rule.condition.clone(), rule_env.clone()) {
                Ok(v) => v,
                Err(_) => continue, // a rule that errors is skipped
            };
            if !is_truthy(&cond) {
                continue;
            }
            if let Value::List(l) = &cond {
                *matches.borrow_mut() = l.iter().cloned().collect();
            }
            for action in &rule.actions {
                let result = match crate::eval::eval(action.clone(), rule_env.clone()) {
                    Ok(v) => v,
                    Err(_) => continue, // a failing action is non-fatal
                };
                if let Value::Item(new_item) = result {
                    // sync the DB-persisted result back into the working item
                    item.text = new_item.text.clone();
                    item.notes = new_item.notes.clone();
                    item.categories = new_item.categories.clone();
                    item.properties = new_item.properties.clone();
                    changed = true;
                }
            }
        }
        if changed {
            changed_count += 1;
        }
    }
    Ok(Value::Number(changed_count as f64))
}

pub fn list_rules(db: &Database) -> Result<Value, LispError> {
    let q = Query {
        table: "_rules".into(),
        where_: None,
        params: vec![],
        order: Some("name".into()),
        ascending: true,
        limit: None,
        select: None,
    };
    let rows = db.query_rows(&q)?;
    if rows.is_empty() {
        return Ok(Value::Str("(no rules defined)".into()));
    }
    let lines: Vec<String> = rows
        .iter()
        .map(|r| format!("{}: {}", sval(&r.data, "name"), sval(&r.data, "condition")))
        .collect();
    Ok(Value::Str(lines.join("\n")))
}

pub fn drop_rule(db: &Database, name: &str) -> Result<Value, LispError> {
    db.soft_delete_where("_rules", "name = ?", &[Value::Str(name.to_string())])?;
    Ok(Value::Null)
}

// ── views ─────────────────────────────────────────────────────────────

pub fn defview(db: &Database, name: &str, filter: Option<Value>, sort_by: Option<String>, group_by: Option<String>, sort_asc: bool) -> Result<Value, LispError> {
    db.soft_delete_where("_views", "name = ?", &[Value::Str(name.to_string())])?;
    let mut d = OrderedDict::default();
    d.insert("name".into(), Value::Str(name.to_string()));
    d.insert("filter".into(), Value::Str(filter.map(|f| crate::printer::print_value(&f, true)).unwrap_or_default()));
    d.insert("group_by".into(), group_by.map(Value::Str).unwrap_or(Value::Null));
    d.insert("sort_by".into(), sort_by.map(Value::Str).unwrap_or(Value::Null));
    d.insert("sort_asc".into(), Value::Bool(sort_asc));
    d.insert("columns".into(), Value::Str("[]".into()));
    db.insert("_views", &d)?;
    Ok(Value::Str(format!("View defined: {}", name)))
}

fn extract_sort_value(item: &Item, field: &str) -> String {
    match field {
        "when" => {
            let w = sval(&item.properties, "when");
            if w.is_empty() { "9999-99-99".into() } else { w }
        }
        "priority" => {
            let p = sval(&item.properties, "priority");
            if p.is_empty() { "999".into() } else { p }
        }
        "text" => item.text.clone(),
        other => sval(&item.properties, other),
    }
}

pub fn show(db: &Database, base: &Env, name: &str) -> Result<Value, LispError> {
    let vq = Query {
        table: "_views".into(),
        where_: Some("name = ?".into()),
        params: vec![Value::Str(name.to_string())],
        order: None,
        ascending: true,
        limit: Some(1),
        select: None,
    };
    let view = db.query_rows(&vq)?.into_iter().next().ok_or_else(|| LispError::Runtime(format!("View not found: {}", name)))?;
    let filter_src = sval(&view.data, "filter");
    let filter = if filter_src.is_empty() { None } else { parse_one(&filter_src) };
    let sort_by = sval(&view.data, "sort_by");
    let group_by = sval(&view.data, "group_by");
    let sort_asc = !matches!(view.data.get("sort_asc"), Some(Value::Bool(false)));

    let mut items = load_all_items(db)?;
    if let Some(f) = &filter {
        items.retain(|it| {
            let holder = Rc::new(RefCell::new(Vec::new()));
            let fenv = build_item_env(base, it, holder);
            matches!(crate::eval::eval(f.clone(), fenv), Ok(v) if is_truthy(&v))
        });
    }
    if !sort_by.is_empty() {
        items.sort_by(|a, b| {
            let (ka, kb) = (extract_sort_value(a, &sort_by), extract_sort_value(b, &sort_by));
            if sort_asc { ka.cmp(&kb) } else { kb.cmp(&ka) }
        });
    }

    if !group_by.is_empty() {
        let mut groups: Vec<(String, Vec<&Item>)> = Vec::new();
        for it in &items {
            let key = if group_by == "category" {
                it.categories.first().cloned().unwrap_or_else(|| "(none)".into())
            } else {
                let v = sval(&it.properties, &group_by);
                if v.is_empty() { "(none)".into() } else { v }
            };
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(it),
                None => groups.push((key, vec![it])),
            }
        }
        let mut out = String::new();
        for (k, its) in groups {
            out.push_str(&format!("▸ {} ({})\n", k, its.len()));
            for it in its {
                out.push_str(&format!("    {}\n", it.text));
            }
        }
        return Ok(Value::Str(out.trim_end().to_string()));
    }

    // non-grouped → display result-set
    let columns = vec!["text".to_string(), "when".into(), "priority".into(), "categories".into()];
    let records = items
        .iter()
        .map(|it| {
            let mut data = OrderedDict::default();
            data.insert("text".into(), Value::Str(it.text.clone()));
            data.insert("when".into(), Value::Str(sval(&it.properties, "when")));
            data.insert("priority".into(), Value::Str(sval(&it.properties, "priority")));
            data.insert("categories".into(), Value::Str(it.categories.join(", ")));
            Record { table: "_items".into(), id: it.id, data, deleted: false }
        })
        .collect();
    Ok(Value::ResultSet(Rc::new(ResultSet { table: "_items".into(), records, columns })))
}

pub fn list_views(db: &Database) -> Result<Value, LispError> {
    let q = Query {
        table: "_views".into(),
        where_: None,
        params: vec![],
        order: Some("name".into()),
        ascending: true,
        limit: None,
        select: None,
    };
    let rows = db.query_rows(&q)?;
    if rows.is_empty() {
        return Ok(Value::Str("(no views defined)".into()));
    }
    Ok(Value::Str(rows.iter().map(|r| sval(&r.data, "name")).collect::<Vec<_>>().join("\n")))
}

pub fn drop_view(db: &Database, name: &str) -> Result<Value, LispError> {
    db.soft_delete_where("_views", "name = ?", &[Value::Str(name.to_string())])?;
    Ok(Value::Null)
}

// ── smart natural-language input ──────────────────────────────────────

pub fn smart_parse_dict(input: &str) -> Value {
    let r = crate::smart_parser::parse(input, &today());
    let mut d = OrderedDict::default();
    d.insert("text".into(), Value::Str(r.text));
    d.insert("when".into(), r.when.map(Value::Str).unwrap_or(Value::Null));
    d.insert("priority".into(), r.priority.map(|p| Value::Number(p as f64)).unwrap_or(Value::Null));
    d.insert("who".into(), Value::List(Rc::new(r.who.into_iter().map(Value::Str).collect())));
    Value::Dict(Rc::new(d))
}

pub fn add_smart(db: &Database, input: &str) -> Result<Value, LispError> {
    let r = crate::smart_parser::parse(input, &today());
    let mut props = OrderedDict::default();
    if let Some(w) = &r.when {
        props.insert("when".into(), Value::Str(w.clone()));
    }
    if let Some(p) = r.priority {
        props.insert("priority".into(), Value::Str(p.to_string()));
    }
    if !r.who.is_empty() {
        props.insert("who".into(), Value::Str(r.who.join(", ")));
    }
    let now = iso_now();
    let row = item_row(&r.text, "", &[], &props, &now, &now);
    Ok(item_value(db.insert("_items", &row)?))
}

// ── recurrence + dates (no external crate; Hinnant's civil algorithms) ─

pub fn advance_date(when: &str, recur: &str) -> Option<String> {
    match recur {
        "daily" => add_days(when, 1),
        "weekly" => add_days(when, 7),
        "monthly" => add_months(when, 1),
        other => {
            // "every:N:unit"
            let parts: Vec<&str> = other.split(':').collect();
            if parts.len() == 3 && parts[0] == "every" {
                let n: i64 = parts[1].parse().ok()?;
                match parts[2] {
                    "days" => add_days(when, n),
                    "weeks" => add_days(when, n * 7),
                    "months" => add_months(when, n),
                    _ => None,
                }
            } else {
                None
            }
        }
    }
}

fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let head: String = s.chars().take(10).collect();
    let parts: Vec<&str> = head.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

fn fmt_ymd(y: i64, m: i64, d: i64) -> String {
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(y) { 29 } else { 28 },
        _ => 30,
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn add_days(date: &str, n: i64) -> Option<String> {
    let (y, m, d) = parse_ymd(date)?;
    let (ny, nm, nd) = civil_from_days(days_from_civil(y, m, d) + n);
    Some(fmt_ymd(ny, nm, nd))
}

fn add_months(date: &str, n: i64) -> Option<String> {
    let (y, m, d) = parse_ymd(date)?;
    let total = (m - 1) + n;
    let ny = y + total.div_euclid(12);
    let nm = total.rem_euclid(12) + 1;
    let nd = d.min(days_in_month(ny, nm));
    Some(fmt_ymd(ny, nm, nd))
}

/// Current timestamp `YYYY-MM-DDTHH:MM:SSZ` (native clock; the WASM build will inject time).
pub fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!("{}T{:02}:{:02}:{:02}Z", fmt_ymd(y, m, d), rem / 3600, (rem % 3600) / 60, rem % 60)
}

pub fn today() -> String {
    iso_now().chars().take(10).collect()
}

// ── value accessor ───────────────────────────────────────────────────

fn as_string(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}
