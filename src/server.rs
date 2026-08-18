//! Native host binding (ANALYSIS §5, step 5).
//!
//! `Interpreter` is single-threaded (`Rc`/`RefCell`), so it can't be a Tauri/multithreaded shared
//! state directly. `EngineHandle` owns the interpreter on a dedicated thread and talks to it over a
//! channel, exposing a `Send + Sync` `eval(src) -> String` (JSON envelope). A Tauri command holds
//! one of these in `State` and forwards `src`; the return is exactly `Interpreter::eval_host`.

use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::Interpreter;

enum Job {
    Eval(String, Sender<String>),
    Stop,
}

pub struct EngineHandle {
    tx: Sender<Job>,
    thread: Option<thread::JoinHandle<()>>,
}

impl EngineHandle {
    /// Spawn an interpreter thread backed by `db_path` (`:memory:` or a file).
    pub fn spawn(db_path: String) -> EngineHandle {
        Self::spawn_with(db_path, |_| {})
    }

    /// Spawn an interpreter thread and let the host install its editor callbacks before the first
    /// job runs. `setup` runs *on* the interpreter thread — the only place the `Rc`-based
    /// `Interpreter` exists — so what it captures must be `Send`: an `Arc<Mutex<…>>` the host also
    /// holds is the usual shape, and it keeps reading current rather than freezing a value here.
    pub fn spawn_with(db_path: String, setup: impl FnOnce(&Interpreter) + Send + 'static) -> EngineHandle {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = thread::spawn(move || {
            let it = Interpreter::with_database(&db_path);
            it.set_echo(false); // output is captured into the envelope, not stdout
            setup(&it);
            while let Ok(job) = rx.recv() {
                match job {
                    Job::Eval(src, reply) => {
                        let _ = reply.send(it.eval_host(&src));
                    }
                    Job::Stop => break,
                }
            }
        });
        EngineHandle { tx, thread: Some(thread) }
    }

    /// Evaluate `src`, returning the JSON envelope (`{ok,result,output}` / `{ok:false,error}`).
    pub fn eval(&self, src: &str) -> String {
        let (rtx, rrx) = mpsc::channel();
        if self.tx.send(Job::Eval(src.to_string(), rtx)).is_err() {
            return r#"{"ok":false,"error":"engine stopped"}"#.to_string();
        }
        rrx.recv().unwrap_or_else(|_| r#"{"ok":false,"error":"engine dropped reply"}"#.to_string())
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Job::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
