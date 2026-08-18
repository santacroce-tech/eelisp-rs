//! Step 5 — the native binding (`EngineHandle`) + the closed-engine loose ends (HTTP arg
//! validation, transactional batch ops).

use eelisp::server::EngineHandle;
use eelisp::Interpreter;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn engine_handle_is_send_sync() {
    // Required so a Tauri app can hold it in shared State.
    assert_send_sync::<EngineHandle>();
}

#[test]
fn engine_handle_evaluates_over_a_thread() {
    let engine = EngineHandle::spawn(":memory:".to_string());
    let out = engine.eval("(+ 1 2)");
    assert!(out.contains("\"ok\":true"));
    assert!(out.contains("\"result\":3"));

    // state persists across calls on the engine thread
    engine.eval("(def x 10)");
    assert!(engine.eval("x").contains("\"result\":10"));

    // structured results come back tagged
    engine.eval("(deftable c (n:string))");
    engine.eval("(insert c {:n \"z\"})");
    assert!(engine.eval("(browse c)").contains("$tableView"));
}

// ── closed engine: HTTP + transactions ──

#[test]
fn http_builtins_validate_args() {
    // registered and validate types without touching the network
    let it = Interpreter::new();
    assert!(it.eval_str("(http-get 42)").is_err());
    assert!(it.eval_str("(http-post 42 \"body\")").is_err());
}

#[test]
fn apply_rules_batch_still_works_transactionally() {
    // exercises the begin/commit path over multiple items in one transaction
    let it = Interpreter::new();
    it.eval_str("(defrule urgent :when (str-contains text \"X\") :assign \"flag/on\")").unwrap();
    for _ in 0..5 {
        it.eval_str("(add-item \"X item\")").unwrap();
    }
    it.eval_str("(add-item \"quiet\")").unwrap();
    let changed = eelisp::printer::print_value(&it.eval_str("(apply-rules)").unwrap(), false);
    assert_eq!(changed, "5");
    assert_eq!(
        eelisp::printer::print_value(&it.eval_str("(length (records (items :category \"flag/on\")))").unwrap(), false),
        "5"
    );
}

/// The host installs its editor callbacks on the interpreter thread and keeps reading through a
/// shared handle, so (current-dir) tracks the workspace instead of freezing at spawn time.
#[test]
fn spawn_with_installs_host_callbacks_that_stay_live() {
    use std::sync::{Arc, Mutex};

    let root = Arc::new(Mutex::new("/notes".to_string()));
    let shared = root.clone();
    let engine = EngineHandle::spawn_with(":memory:".into(), move |it| {
        it.editor.borrow_mut().current_dir = Some(Box::new(move || shared.lock().unwrap().clone()));
    });

    assert!(engine.eval("(current-dir)").contains("/notes"));
    *root.lock().unwrap() = "/elsewhere".to_string();
    assert!(engine.eval("(current-dir)").contains("/elsewhere"));
}
