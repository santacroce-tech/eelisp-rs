//! EELisp — Rust rewrite of the engine core.
//!
//! This is the portable asset from the SPEC/ANALYSIS: language + (later) SQLite + agenda.
//! It maps 1:1 onto the Swift `Value` enum but fixes the documented bugs:
//!   * proper tail-call optimization (the eval loop is a trampoline)
//!   * quasiquote / unquote / unquote-splicing are implemented
//!   * macros support `. rest` params (so `when`/`unless`/`pipe` actually work)
//!   * clean tokenization (no operator-number splitting, no positional `-` heuristic)
//!
//! Done: language core + the SQLite database layer (`database` / `db_builtins`).
//! Still TODO (see ANALYSIS §4.10–4.12): the agenda PIM, dates, HTTP/JSON, and the structured
//! `.tableView`/`.formView` view values + serde boundary.

pub mod agenda;
pub mod agenda_builtins;
pub mod builtins;
pub mod database;
pub mod dates;
pub mod db_builtins;
pub mod docs;
pub mod editor;
pub mod env;
pub mod eval;
pub mod host;
pub mod interpreter;
pub mod lexer;
pub mod output;
pub mod parser;
pub mod prelude;
pub mod printer;
/// A thread per engine — native hosts only; a browser runs the interpreter on its own thread.
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod sheet;
pub mod sheet_builtins;
pub mod sheet_ref;
pub mod smart_parser;
pub mod value;

pub use interpreter::Interpreter;
pub use value::{LispError, Value};
