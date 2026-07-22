//! The universal `Value` type and the error type.
//!
//! Mirrors the Swift `indirect enum Value` (ANALYSIS §4.1). All numbers are `f64`. Lists are
//! `Rc<Vec<Value>>` (cheap clone). Dicts preserve insertion order. Equality matches EELisp:
//! atoms/lists/dicts compare structurally; functions/builtins/macros are never equal.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::env::Env;

pub type Symbol = String;

#[derive(Clone)]
pub enum Value {
    Symbol(Symbol),
    Str(String),
    Number(f64),
    Bool(bool),
    Keyword(Symbol),
    Null,
    List(Rc<Vec<Value>>),
    Dict(Rc<OrderedDict>),
    Function(Rc<Function>),
    Builtin(Rc<Builtin>),
    Macro(Rc<Macro>),
    // Database layer (ANALYSIS §4.9). Never compare equal (fall through in PartialEq).
    Table(Rc<TableDef>),
    Record(Rc<Record>),
    ResultSet(Rc<ResultSet>),
    // Agenda layer (ANALYSIS §4.10).
    Item(Rc<Item>),
    // Interactive views (ANALYSIS §5) — the structured host/UI contract.
    TableView(Rc<TableView>),
    FormView(Rc<FormView>),
}

/// Insertion-ordered map — load-bearing for record/dict/form ordering (ANALYSIS §4.1).
#[derive(Clone, Default)]
pub struct OrderedDict {
    pub keys: Vec<Symbol>,
    pub map: HashMap<Symbol, Value>,
}

impl OrderedDict {
    pub fn insert(&mut self, k: Symbol, v: Value) {
        if !self.map.contains_key(&k) {
            self.keys.push(k.clone());
        }
        self.map.insert(k, v);
    }
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.map.get(k)
    }
}

impl PartialEq for OrderedDict {
    fn eq(&self, o: &Self) -> bool {
        self.keys == o.keys && self.keys.iter().all(|k| self.map.get(k) == o.map.get(k))
    }
}

pub struct Function {
    pub name: Option<String>,
    pub params: Vec<Symbol>,
    pub rest: Option<Symbol>,
    pub body: Vec<Value>,
    pub closure: Env,
}

pub struct Macro {
    pub name: Option<String>,
    pub params: Vec<Symbol>,
    pub rest: Option<Symbol>, // fixed vs Swift: macros DO carry a rest param
    pub body: Vec<Value>,
    pub closure: Env,
}

/// How a builtin's arguments are evaluated before the call.
pub enum ArgMode {
    /// Standard: every argument is evaluated (the default).
    Eval,
    /// dBASE table commands: arg 0 (the table name) passes as a raw symbol, the rest evaluate.
    TableFirst,
    /// deftable / defform: all arguments pass raw (schema forms must not be evaluated).
    AllRaw,
}

pub struct Builtin {
    pub name: String,
    pub arg_mode: ArgMode,
    // Boxed closure (not a fn pointer) so builtins can capture state — e.g. the database handle.
    pub func: Box<dyn Fn(&[Value], &Env) -> Result<Value, LispError>>,
}

// ── Database value types (ANALYSIS §4.9) ─────────────────────────────

#[derive(Clone, PartialEq, Debug)]
pub enum FieldType {
    String,
    Number,
    Bool,
    Date,
    Memo,
    Choice,
}

impl FieldType {
    pub fn parse(s: &str) -> FieldType {
        match s {
            "number" => FieldType::Number,
            "bool" => FieldType::Bool,
            "date" => FieldType::Date,
            "memo" => FieldType::Memo,
            "choice" => FieldType::Choice,
            _ => FieldType::String,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::String => "string",
            FieldType::Number => "number",
            FieldType::Bool => "bool",
            FieldType::Date => "date",
            FieldType::Memo => "memo",
            FieldType::Choice => "choice",
        }
    }
    pub fn sqlite_type(&self) -> &'static str {
        match self {
            FieldType::Number => "REAL",
            FieldType::Bool => "INTEGER",
            _ => "TEXT",
        }
    }
}

#[derive(Clone)]
pub struct FieldDef {
    pub name: String,
    pub ftype: FieldType,
    pub required: bool,
    pub default: Option<Value>,
    pub choices: Vec<String>,
}

#[derive(Clone)]
pub struct TableDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

impl TableDef {
    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }
}

#[derive(Clone)]
pub struct Record {
    pub table: String,
    pub id: i64,
    pub data: OrderedDict,
    pub deleted: bool,
}

#[derive(Clone)]
pub struct ResultSet {
    pub table: String,
    pub records: Vec<Record>,
    pub columns: Vec<String>,
}

// ── Interactive views (ANALYSIS §5) ──────────────────────────────────
// browse → TableView (grid), edit → FormView (CRUD form), defform → FormView (calculator).

#[derive(Clone)]
pub struct TableView {
    pub table_name: String,
    pub table_def: TableDef,
    pub result_set: ResultSet,
}

#[derive(Clone)]
pub struct ComputedField {
    pub name: String,
    pub ftype: FieldType,
    pub expression: Value, // AST — the host re-evaluates on input change
}

#[derive(Clone)]
pub struct FormView {
    pub table_name: String,
    pub table_def: TableDef,
    pub result_set: ResultSet,
    pub computed_fields: Vec<ComputedField>,
    pub is_standalone: bool, // defform (calculator) vs edit (db-backed CRUD)
}

// ── Agenda item (ANALYSIS §4.10) ─────────────────────────────────────

#[derive(Clone)]
pub struct Item {
    pub id: i64,
    pub text: String,
    pub notes: String,
    pub categories: Vec<String>,
    /// when / priority / recurrence / … — values round-trip as strings (matches EELisp).
    pub properties: OrderedDict,
    pub created: String,
    pub modified: String,
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Symbol(a), Symbol(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Number(a), Number(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Keyword(a), Keyword(b)) => a == b,
            (Null, Null) => true,
            (List(a), List(b)) => a == b,
            (Dict(a), Dict(b)) => a == b,
            _ => false, // functions / builtins / macros are never equal
        }
    }
}

/// Only `false` and `nil` are falsy — `0`, `""`, `()` are all truthy (ANALYSIS §4.1).
pub fn is_truthy(v: &Value) -> bool {
    !matches!(v, Value::Bool(false) | Value::Null)
}

pub fn type_name(v: &Value) -> String {
    match v {
        Value::Symbol(_) => "symbol",
        Value::Str(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "bool",
        Value::Keyword(_) => "keyword",
        Value::Null => "nil",
        Value::List(_) => "list",
        Value::Dict(_) => "dict",
        Value::Function(_) => "function",
        Value::Builtin(_) => "builtin",
        Value::Macro(_) => "macro",
        Value::Table(_) => "table",
        Value::Record(_) => "record",
        Value::ResultSet(_) => "result-set",
        Value::Item(_) => "item",
        Value::TableView(_) => "table-view",
        Value::FormView(_) => "form-view",
    }
    .to_string()
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", crate::printer::print_value(self, false))
    }
}
impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", crate::printer::print_value(self, true))
    }
}

#[derive(Clone)]
pub enum LispError {
    UndefinedSymbol(String),
    TypeMismatch { expected: String, got: String },
    Arity { func: String, expected: String, got: usize },
    InvalidSyntax(String),
    DivisionByZero,
    IndexOutOfBounds { index: i64, len: usize },
    Parse(String),
    Database(String),
    Runtime(String),
    /// Control signal for `recur` → carries the next iteration's values up to the enclosing `loop`.
    /// Not a user-visible error; only surfaces as one if `recur` is used outside a `loop`.
    Recur(Vec<Value>),
}

impl fmt::Display for LispError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LispError::UndefinedSymbol(s) => write!(f, "Undefined symbol: {}", s),
            LispError::TypeMismatch { expected, got } => {
                write!(f, "Type mismatch: expected {}, got {}", expected, got)
            }
            LispError::Arity { func, expected, got } => {
                write!(f, "Arity mismatch: {} expects {}, got {}", func, expected, got)
            }
            LispError::InvalidSyntax(s) => write!(f, "Invalid syntax: {}", s),
            LispError::DivisionByZero => write!(f, "Division by zero"),
            LispError::IndexOutOfBounds { index, len } => {
                write!(f, "Index out of bounds: {} (len {})", index, len)
            }
            LispError::Parse(s) => write!(f, "Parse error: {}", s),
            LispError::Database(s) => write!(f, "Database error: {}", s),
            LispError::Runtime(s) => write!(f, "Error: {}", s),
            LispError::Recur(_) => write!(f, "recur used outside of a loop"),
        }
    }
}
impl fmt::Debug for LispError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self)
    }
}
impl std::error::Error for LispError {}
