//! SQLite database layer (ANALYSIS §4.9) — the dBASE-style engine, rewritten on `rusqlite`.
//!
//! Fixes carried over from ANALYSIS §6:
//!   * schema is stored as **JSON** in `_eelisp_schema`, so field defaults and choices survive a
//!     reload (the Swift custom `name:type:req:def` format dropped them).
//!   * reads dispatch by the declared field type, not by storage type.
//!
//! WASM note: this backend links SQLite (C) via rusqlite's `bundled` feature — native only. The
//! future web build swaps this module for a wa-sqlite/OPFS backend behind the same method surface.

use std::collections::HashMap;
use std::rc::Rc;

use rusqlite::params;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::Connection;

use crate::printer::print_value;
use crate::value::*;

pub struct Database {
    conn: Connection,
    defs: HashMap<String, TableDef>,
}

fn db_err(e: impl std::fmt::Display) -> LispError {
    LispError::Database(e.to_string())
}

pub struct Query {
    pub table: String,
    pub where_: Option<String>,
    pub params: Vec<Value>,
    pub order: Option<String>,
    pub ascending: bool,
    pub limit: Option<i64>,
    pub select: Option<Vec<String>>,
}

impl Database {
    pub fn open(path: &str) -> Result<Self, LispError> {
        let conn = Connection::open(path).map_err(db_err)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _eelisp_schema (name TEXT PRIMARY KEY, def TEXT NOT NULL)",
            [],
        )
        .map_err(db_err)?;
        let mut db = Database { conn, defs: HashMap::new() };
        db.load_defs()?;
        Ok(db)
    }

    fn load_defs(&mut self) -> Result<(), LispError> {
        let mut stmt = self.conn.prepare("SELECT name, def FROM _eelisp_schema").map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(db_err)?;
        for r in rows {
            let (name, def_json) = r.map_err(db_err)?;
            if let Some(def) = deserialize_def(&def_json) {
                self.defs.insert(name, def);
            }
        }
        Ok(())
    }

    pub fn list_tables(&self) -> Vec<String> {
        let mut names: Vec<String> = self.defs.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn get_def(&self, table: &str) -> Option<TableDef> {
        self.defs.get(table).cloned()
    }

    pub fn create_table(&mut self, def: TableDef) -> Result<Value, LispError> {
        // re-deftable is a no-op that returns the existing def (matches EELisp)
        if let Some(existing) = self.defs.get(&def.name) {
            return Ok(Value::Table(Rc::new(existing.clone())));
        }
        let mut cols = vec![
            "_id INTEGER PRIMARY KEY AUTOINCREMENT".to_string(),
            "_deleted INTEGER DEFAULT 0".to_string(),
        ];
        for fd in &def.fields {
            let mut col = format!("{} {}", fd.name, fd.ftype.sqlite_type());
            if fd.required {
                col.push_str(" NOT NULL");
            }
            if let Some(dv) = &fd.default {
                col.push_str(&format!(" DEFAULT {}", sql_default(dv)));
            }
            cols.push(col);
        }
        let sql = format!("CREATE TABLE IF NOT EXISTS {} ({})", def.name, cols.join(", "));
        self.conn.execute(&sql, []).map_err(db_err)?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO _eelisp_schema (name, def) VALUES (?1, ?2)",
                params![def.name, serialize_def(&def)],
            )
            .map_err(db_err)?;
        self.defs.insert(def.name.clone(), def.clone());
        Ok(Value::Table(Rc::new(def)))
    }

    pub fn drop_table(&mut self, table: &str) -> Result<Value, LispError> {
        self.conn.execute(&format!("DROP TABLE IF EXISTS {}", table), []).map_err(db_err)?;
        self.conn
            .execute("DELETE FROM _eelisp_schema WHERE name = ?1", params![table])
            .map_err(db_err)?;
        self.defs.remove(table);
        Ok(Value::Bool(true))
    }

    pub fn insert(&self, table: &str, data: &OrderedDict) -> Result<Value, LispError> {
        let def = self.def_or_err(table)?;
        for k in &data.keys {
            if def.field(k).is_none() {
                return Err(db_err(format!("Field not found: {} in {}", k, table)));
            }
        }
        let cols = data.keys.clone();
        let placeholders: Vec<String> = (1..=cols.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table,
            cols.join(", "),
            placeholders.join(", ")
        );
        let vals: Vec<SqlValue> = cols.iter().map(|c| to_sql(&data.map[c])).collect();
        self.conn.execute(&sql, rusqlite::params_from_iter(vals)).map_err(db_err)?;
        let id = self.conn.last_insert_rowid();
        Ok(Value::Record(Rc::new(Record {
            table: table.to_string(),
            id,
            data: data.clone(),
            deleted: false,
        })))
    }

    pub fn query(&self, q: &Query) -> Result<Value, LispError> {
        let columns: Vec<String> = self.columns_for(q)?;
        let records = self.query_rows(q)?;
        Ok(Value::ResultSet(Rc::new(ResultSet { table: q.table.clone(), records, columns })))
    }

    fn columns_for(&self, q: &Query) -> Result<Vec<String>, LispError> {
        let def = self.def_or_err(&q.table)?;
        Ok(match &q.select {
            Some(cols) => cols.clone(),
            None => def.fields.iter().map(|f| f.name.clone()).collect(),
        })
    }

    /// Like `query`, but returns the raw records (the agenda layer builds on this).
    pub fn query_rows(&self, q: &Query) -> Result<Vec<Record>, LispError> {
        let def = self.def_or_err(&q.table)?;
        let columns = self.columns_for(q)?;
        let types: Vec<FieldType> = columns
            .iter()
            .map(|c| def.field(c).map(|f| f.ftype.clone()).unwrap_or(FieldType::String))
            .collect();

        let mut sql = format!("SELECT _id, {} FROM {} WHERE _deleted = 0", columns.join(", "), q.table);
        if let Some(w) = &q.where_ {
            sql.push_str(&format!(" AND ({})", w));
        }
        if let Some(o) = &q.order {
            sql.push_str(&format!(" ORDER BY {} {}", o, if q.ascending { "ASC" } else { "DESC" }));
        }
        if let Some(l) = q.limit {
            sql.push_str(&format!(" LIMIT {}", l));
        }

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let vals: Vec<SqlValue> = q.params.iter().map(to_sql).collect();
        let mut rows = stmt.query(rusqlite::params_from_iter(vals)).map_err(db_err)?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().map_err(db_err)? {
            let id: i64 = row.get(0).map_err(db_err)?;
            let mut data = OrderedDict::default();
            for (i, col) in columns.iter().enumerate() {
                let vref = row.get_ref(i + 1).map_err(db_err)?;
                data.insert(col.clone(), from_sql(vref, &types[i]));
            }
            records.push(Record { table: q.table.clone(), id, data, deleted: false });
        }
        Ok(records)
    }

    /// Soft-delete rows matching a raw WHERE clause (used by drop-template / drop-rule / drop-view).
    pub fn soft_delete_where(&self, table: &str, where_: &str, params: &[Value]) -> Result<i64, LispError> {
        let sql = format!("UPDATE {} SET _deleted = 1 WHERE _deleted = 0 AND ({})", table, where_);
        let vals: Vec<SqlValue> = params.iter().map(to_sql).collect();
        let changed = self.conn.execute(&sql, rusqlite::params_from_iter(vals)).map_err(db_err)?;
        Ok(changed as i64)
    }

    pub fn update(&self, table: &str, id: i64, data: &OrderedDict) -> Result<Value, LispError> {
        let def = self.def_or_err(table)?;
        for k in &data.keys {
            if def.field(k).is_none() {
                return Err(db_err(format!("Field not found: {} in {}", k, table)));
            }
        }
        let sets: Vec<String> =
            data.keys.iter().enumerate().map(|(i, c)| format!("{} = ?{}", c, i + 1)).collect();
        let sql = format!(
            "UPDATE {} SET {} WHERE _deleted = 0 AND _id = ?{}",
            table,
            sets.join(", "),
            data.keys.len() + 1
        );
        let mut vals: Vec<SqlValue> = data.keys.iter().map(|c| to_sql(&data.map[c])).collect();
        vals.push(SqlValue::Integer(id));
        let changed = self.conn.execute(&sql, rusqlite::params_from_iter(vals)).map_err(db_err)?;
        Ok(Value::Number(changed as f64))
    }

    pub fn delete(&self, table: &str, id: i64) -> Result<Value, LispError> {
        let sql = format!("UPDATE {} SET _deleted = 1 WHERE _deleted = 0 AND _id = ?1", table);
        let changed = self.conn.execute(&sql, params![id]).map_err(db_err)?;
        Ok(Value::Number(changed as f64))
    }

    pub fn count(&self, table: &str, where_: Option<&str>, params: &[Value]) -> Result<Value, LispError> {
        let mut sql = format!("SELECT COUNT(*) FROM {} WHERE _deleted = 0", table);
        if let Some(w) = where_ {
            sql.push_str(&format!(" AND ({})", w));
        }
        let vals: Vec<SqlValue> = params.iter().map(to_sql).collect();
        let n: i64 = self
            .conn
            .query_row(&sql, rusqlite::params_from_iter(vals), |r| r.get(0))
            .map_err(db_err)?;
        Ok(Value::Number(n as f64))
    }

    pub fn pack(&self, table: &str) -> Result<Value, LispError> {
        self.conn.execute(&format!("DELETE FROM {} WHERE _deleted = 1", table), []).map_err(db_err)?;
        Ok(Value::Bool(true))
    }

    fn def_or_err(&self, table: &str) -> Result<TableDef, LispError> {
        self.defs.get(table).cloned().ok_or_else(|| db_err(format!("Table not found: {}", table)))
    }

    // Explicit transactions so batch operations (apply-rules, import) are atomic and fast.
    pub fn begin(&self) -> Result<(), LispError> {
        self.conn.execute("BEGIN", []).map_err(db_err)?;
        Ok(())
    }
    pub fn commit(&self) -> Result<(), LispError> {
        self.conn.execute("COMMIT", []).map_err(db_err)?;
        Ok(())
    }
    pub fn rollback(&self) -> Result<(), LispError> {
        self.conn.execute("ROLLBACK", []).map_err(db_err)?;
        Ok(())
    }
}

// ── Value ⇄ SQLite mapping ────────────────────────────────────────

fn to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() && n.abs() < 9.0e18 {
                SqlValue::Integer(*n as i64)
            } else {
                SqlValue::Real(*n)
            }
        }
        Value::Str(s) => SqlValue::Text(s.clone()),
        Value::Bool(b) => SqlValue::Integer(if *b { 1 } else { 0 }),
        Value::Null => SqlValue::Null,
        other => SqlValue::Text(print_value(other, false)),
    }
}

fn from_sql(vref: ValueRef, ftype: &FieldType) -> Value {
    match vref {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => match ftype {
            FieldType::Bool => Value::Bool(i != 0),
            _ => Value::Number(i as f64),
        },
        ValueRef::Real(f) => Value::Number(f),
        ValueRef::Text(t) => Value::Str(String::from_utf8_lossy(t).to_string()),
        ValueRef::Blob(_) => Value::Null,
    }
}

fn sql_default(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Number(_) => print_value(v, false),
        Value::Bool(b) => if *b { "1".into() } else { "0".into() },
        _ => "NULL".into(),
    }
}

// ── schema JSON (defaults & choices survive a reload) ─────────────

fn serialize_def(def: &TableDef) -> String {
    let fields: Vec<serde_json::Value> = def
        .fields
        .iter()
        .map(|f| {
            serde_json::json!({
                "name": f.name,
                "type": f.ftype.as_str(),
                "required": f.required,
                "default": f.default.as_ref().map(|v| print_value(v, true)),
                "choices": f.choices,
            })
        })
        .collect();
    serde_json::json!({ "name": def.name, "fields": fields }).to_string()
}

fn deserialize_def(json: &str) -> Option<TableDef> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let mut fields = Vec::new();
    for f in v.get("fields")?.as_array()? {
        let fname = f.get("name")?.as_str()?.to_string();
        let ftype = FieldType::parse(f.get("type").and_then(|t| t.as_str()).unwrap_or("string"));
        let required = f.get("required").and_then(|r| r.as_bool()).unwrap_or(false);
        let default = f.get("default").and_then(|d| d.as_str()).and_then(|s| {
            crate::parser::parse(s).ok().and_then(|mut forms| {
                if forms.is_empty() {
                    None
                } else {
                    Some(forms.remove(0))
                }
            })
        });
        let choices = f
            .get("choices")
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        fields.push(FieldDef { name: fname, ftype, required, default, choices });
    }
    Some(TableDef { name, fields })
}
