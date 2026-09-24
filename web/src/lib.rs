//! The EELisp engine in a browser.
//!
//! One `Engine` is one interpreter with its own SQLite database, in memory. `eval` takes source and
//! returns the same JSON envelope the native hosts get from `EngineHandle::eval` —
//! `{"ok":true,"result":…,"output":"…"}` or `{"ok":false,"error":"…"}` — so the app's
//! `EngineClient` can talk to it with the transport swapped and nothing else.
//!
//! There is no thread: the interpreter runs on the caller's (the page's, or a Web Worker's).

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Engine {
    it: eelisp::Interpreter,
}

#[wasm_bindgen]
impl Engine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Engine {
        let it = eelisp::Interpreter::new();
        it.set_echo(false); // output is captured into the envelope, not printed
        Engine { it }
    }

    /// Evaluate `src`; the JSON envelope.
    pub fn eval(&self, src: &str) -> String {
        self.it.eval_host(src)
    }

    /// The whole database as the bytes of a SQLite file — what the page keeps between visits, and
    /// what *Save data…* writes.
    #[wasm_bindgen(js_name = exportDb)]
    pub fn export_db(&self) -> Result<Vec<u8>, JsError> {
        self.it.export_database().map_err(|e| JsError::new(&e.to_string()))
    }

    /// Replace the database with the bytes of a SQLite file. Bytes that aren't one are refused and
    /// the current data kept.
    #[wasm_bindgen(js_name = importDb)]
    pub fn import_db(&self, bytes: &[u8]) -> Result<(), JsError> {
        self.it.import_database(bytes).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Rows changed since the database was opened or imported: when it moves, there is something
    /// new to keep.
    pub fn changes(&self) -> f64 {
        self.it.database_changes() as f64
    }

    /// A sheet as the bytes of a `.eesheet` file, open from now on under `name` — the name forms and
    /// formulas use for it. The page has no files: this is how an exported app's sheets arrive.
    #[wasm_bindgen(js_name = importSheet)]
    pub fn import_sheet(&self, name: &str, bytes: &[u8]) -> Result<(), JsError> {
        self.it.import_sheet(name, bytes).map_err(|e| JsError::new(&e.to_string()))
    }

    /// An open sheet as the bytes of a `.eesheet` file — what the page keeps after it changed.
    #[wasm_bindgen(js_name = exportSheet)]
    pub fn export_sheet(&self, name: &str) -> Result<Vec<u8>, JsError> {
        self.it.export_sheet(name).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Every open sheet and its version, as JSON: `[["examples/Budget.eesheet", 7], …]`. A version
    /// that moved is a sheet to keep again.
    #[wasm_bindgen(js_name = sheetVersions)]
    pub fn sheet_versions(&self) -> String {
        let v: Vec<(String, i64)> = self.it.sheet_versions();
        let items: Vec<String> = v
            .iter()
            .map(|(p, n)| format!("[{},{}]", json_string(p), n))
            .collect();
        format!("[{}]", items.join(","))
    }
}

/// A string as a JSON string literal.
fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
