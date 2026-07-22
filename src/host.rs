//! Host boundary (ANALYSIS §5): serialize any `Value` — including the structured `.tableView` /
//! `.formView` — to JSON for a frontend (Tauri/WASM), and parse plain JSON into a `Value`.
//!
//! Encoding: scalars/list are natural JSON; the rich types are tagged objects (`$dict`, `$record`,
//! `$tableView`, …) so a TypeScript client can discriminate and render them.

use serde_json::{json, Value as J};

use crate::printer::print_value;
use crate::value::*;

pub fn to_json(v: &Value) -> J {
    match v {
        Value::Number(n) => json!(n),
        Value::Str(s) => json!(s),
        Value::Bool(b) => json!(b),
        Value::Null => J::Null,
        Value::Symbol(s) => json!({ "$sym": s }),
        Value::Keyword(k) => json!({ "$kw": k }),
        Value::List(l) => J::Array(l.iter().map(to_json).collect()),
        Value::Dict(d) => json!({ "$dict": ordered_pairs(d) }),
        Value::Table(t) => json!({ "$table": table_json(t) }),
        Value::Record(r) => json!({ "$record": record_json(r) }),
        Value::ResultSet(rs) => json!({ "$resultSet": resultset_json(rs) }),
        Value::Item(it) => json!({ "$item": item_json(it) }),
        Value::TableView(tv) => json!({
            "$tableView": {
                "tableName": tv.table_name,
                "tableDef": table_json(&tv.table_def),
                "resultSet": resultset_json(&tv.result_set),
            }
        }),
        Value::FormView(fv) => json!({
            "$formView": {
                "tableName": fv.table_name,
                "tableDef": table_json(&fv.table_def),
                "resultSet": resultset_json(&fv.result_set),
                "computedFields": fv.computed_fields.iter().map(|c| json!({
                    "name": c.name,
                    "type": c.ftype.as_str(),
                    "expression": print_value(&c.expression, true),
                })).collect::<Vec<_>>(),
                "isStandalone": fv.is_standalone,
            }
        }),
        Value::Function(f) => json!({ "$fn": f.name.clone().unwrap_or_else(|| "anonymous".into()) }),
        Value::Builtin(b) => json!({ "$builtin": b.name }),
        Value::Macro(m) => json!({ "$macro": m.name.clone().unwrap_or_else(|| "anonymous".into()) }),
    }
}

fn ordered_pairs(d: &OrderedDict) -> J {
    J::Array(d.keys.iter().map(|k| json!([k, to_json(&d.map[k])])).collect())
}

fn data_object(d: &OrderedDict) -> J {
    let mut m = serde_json::Map::new();
    for k in &d.keys {
        m.insert(k.clone(), to_json(&d.map[k]));
    }
    J::Object(m)
}

fn field_json(f: &FieldDef) -> J {
    json!({
        "name": f.name,
        "type": f.ftype.as_str(),
        "required": f.required,
        "choices": f.choices,
    })
}

fn table_json(t: &TableDef) -> J {
    json!({ "name": t.name, "fields": t.fields.iter().map(field_json).collect::<Vec<_>>() })
}

fn record_json(r: &Record) -> J {
    json!({ "table": r.table, "id": r.id, "data": data_object(&r.data) })
}

fn resultset_json(rs: &ResultSet) -> J {
    json!({
        "table": rs.table,
        "columns": rs.columns,
        "records": rs.records.iter().map(record_json).collect::<Vec<_>>(),
    })
}

fn item_json(it: &Item) -> J {
    json!({
        "id": it.id,
        "text": it.text,
        "notes": it.notes,
        "categories": it.categories,
        "properties": data_object(&it.properties),
        "created": it.created,
        "modified": it.modified,
    })
}

/// Parse plain JSON (e.g. an API payload) into a Value: objects→dicts (keys sorted, like EELisp),
/// arrays→lists, with scalars mapped directly. Used by `json-parse`.
pub fn from_json(j: &J) -> Value {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::List(std::rc::Rc::new(a.iter().map(from_json).collect())),
        J::Object(m) => {
            let mut d = OrderedDict::default();
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys {
                d.insert(k.clone(), from_json(&m[k]));
            }
            Value::Dict(std::rc::Rc::new(d))
        }
    }
}
