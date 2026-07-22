//! Value → string (ANALYSIS §4). `readable=true` quotes/escapes strings; `false` is display form.
//! Integers print without a trailing `.0`.

use crate::value::*;

pub fn print_value(v: &Value, readable: bool) -> String {
    match v {
        Value::Symbol(s) => s.clone(),
        Value::Str(s) => {
            if readable {
                format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                s.clone()
            }
        }
        Value::Number(n) => fmt_num(*n),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Keyword(k) => format!(":{}", k),
        Value::Null => "nil".to_string(),
        Value::List(l) => {
            let inner: Vec<String> = l.iter().map(|x| print_value(x, readable)).collect();
            format!("({})", inner.join(" "))
        }
        Value::Dict(d) => {
            let inner: Vec<String> = d
                .keys
                .iter()
                .map(|k| format!(":{} {}", k, print_value(&d.map[k], readable)))
                .collect();
            format!("{{{}}}", inner.join(" "))
        }
        Value::Function(f) => {
            format!("#<fn {}>", f.name.clone().unwrap_or_else(|| "anonymous".into()))
        }
        Value::Builtin(b) => format!("#<builtin {}>", b.name),
        Value::Macro(m) => {
            format!("#<macro {}>", m.name.clone().unwrap_or_else(|| "anonymous".into()))
        }
        Value::Table(t) => {
            let cols: Vec<String> =
                t.fields.iter().map(|fd| format!("{}:{}", fd.name, fd.ftype.as_str())).collect();
            format!("#<table {} ({})>", t.name, cols.join(" "))
        }
        Value::Record(r) => {
            let mut parts = vec![format!("id: {}", r.id)];
            for k in &r.data.keys {
                parts.push(format!("{}: {}", k, print_value(&r.data.map[k], true)));
            }
            format!("{{{}}}", parts.join(", "))
        }
        Value::ResultSet(rs) => format_result_set(rs),
        Value::Item(it) => format!("#<item {}: {}>", it.id, it.text),
        Value::TableView(tv) => format_result_set(&tv.result_set),
        Value::FormView(fv) => {
            let cols: Vec<String> = fv.table_def.fields.iter().map(|f| f.name.clone()).collect();
            format!(
                "#<form {} ({}) {} record(s){}>",
                fv.table_name,
                cols.join(" "),
                fv.result_set.records.len(),
                if fv.is_standalone { " standalone" } else { "" }
            )
        }
    }
}

/// Minimal ASCII table for a result-set (the dBASE feel). TODO: column auto-sizing polish.
fn format_result_set(rs: &ResultSet) -> String {
    if rs.records.is_empty() {
        return format!("(empty result-set: {})", rs.table);
    }
    let mut cols = vec!["id".to_string()];
    cols.extend(rs.columns.iter().cloned());

    let mut widths: Vec<usize> = cols.iter().map(|c| c.len()).collect();
    let rows: Vec<Vec<String>> = rs
        .records
        .iter()
        .map(|r| {
            let mut cells = vec![r.id.to_string()];
            for c in &rs.columns {
                cells.push(r.data.get(c).map(|v| print_value(v, false)).unwrap_or_default());
            }
            for (i, cell) in cells.iter().enumerate() {
                if cell.len() > widths[i] {
                    widths[i] = cell.len();
                }
            }
            cells
        })
        .collect();

    let sep = |left: &str, mid: &str, right: &str| -> String {
        let segs: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        format!("{}{}{}", left, segs.join(mid), right)
    };
    let fmt_row = |cells: &[String]| -> String {
        let segs: Vec<String> =
            cells.iter().enumerate().map(|(i, c)| format!(" {:width$} ", c, width = widths[i])).collect();
        format!("│{}│", segs.join("│"))
    };

    let mut out = String::new();
    out.push_str(&sep("┌", "┬", "┐"));
    out.push('\n');
    out.push_str(&fmt_row(&cols));
    out.push('\n');
    out.push_str(&sep("├", "┼", "┤"));
    out.push('\n');
    for row in &rows {
        out.push_str(&fmt_row(row));
        out.push('\n');
    }
    out.push_str(&sep("└", "┴", "┘"));
    out
}

fn fmt_num(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}
