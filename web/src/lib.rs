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
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
