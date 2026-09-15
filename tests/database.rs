//! The engine's database outlives the process: a host names a file, the tables and agenda live in
//! it, and a host whose workspace moves can point the running engine at another one.

use std::path::PathBuf;

use eelisp::server::EngineHandle;
use eelisp::Interpreter;

/// A fresh folder under the system temp dir, unique to one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("eelisp-db-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn result(envelope: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(envelope).unwrap();
    assert_eq!(v["ok"], true, "{envelope}");
    match &v["result"] {
        // every number is an f64 in the engine; `3.0_f64.to_string()` is "3"
        serde_json::Value::Number(n) => n.as_f64().unwrap().to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `(database-info)` as (path, error) — the dict comes back as `{"$dict": [[k, v], …]}`.
fn info(engine: &EngineHandle) -> (String, Option<String>) {
    let v: serde_json::Value = serde_json::from_str(&engine.eval("(database-info)")).unwrap();
    let mut path = String::new();
    let mut error = None;
    for pair in v["result"]["$dict"].as_array().unwrap() {
        match pair[0].as_str().unwrap() {
            "path" => path = pair[1].as_str().unwrap().to_string(),
            "error" => error = Some(pair[1].as_str().unwrap().to_string()),
            _ => {}
        }
    }
    (path, error)
}

#[test]
fn a_file_database_keeps_items_and_tables_across_engines() {
    let db = scratch("keeps").join("eeditor.db").display().to_string();
    {
        let engine = EngineHandle::spawn(db.clone());
        result(&engine.eval("(add-item \"call Bob\")"));
        result(&engine.eval("(deftable contacts (name:string))"));
        result(&engine.eval("(insert contacts {:name \"Ada\"})"));
    } // dropped: the thread stops, the connection closes

    let engine = EngineHandle::spawn(db.clone());
    assert_eq!(result(&engine.eval("(item-count)")), "1");
    assert_eq!(result(&engine.eval("(count-records contacts)")), "1");
    assert_eq!(info(&engine), (db, None));
}

#[test]
fn the_folder_is_created_when_missing() {
    // a fresh workspace has no .eeditor/ yet
    let db = scratch("mkdir").join(".eeditor").join("eeditor.db");
    let it = Interpreter::try_with_database(&db.display().to_string()).expect("opens");
    it.eval_str("(add-item \"x\")").unwrap();
    assert!(db.is_file());
}

#[test]
fn an_unopenable_file_falls_back_to_memory_and_says_why() {
    // a folder can't be made inside a regular file
    let blocker = scratch("blocked").join("not-a-folder");
    std::fs::write(&blocker, "").unwrap();
    let db = blocker.join("eeditor.db").display().to_string();

    let engine = EngineHandle::spawn(db.clone());
    // still an engine: it evaluates, and the agenda works for the session
    assert_eq!(result(&engine.eval("(+ 1 2)")), "3");
    result(&engine.eval("(add-item \"kept for now\")"));
    assert_eq!(result(&engine.eval("(item-count)")), "1");

    let (path, error) = info(&engine);
    assert_eq!(path, ":memory:");
    let error = error.expect("an error is reported");
    assert!(error.contains(&db), "names the file: {error}");
}

#[test]
fn memory_is_not_an_error() {
    let engine = EngineHandle::spawn(":memory:".into());
    assert_eq!(info(&engine), (":memory:".to_string(), None));
}

#[test]
fn open_database_moves_the_engine_to_another_file() {
    let dir = scratch("move");
    let first = dir.join("first").join("eeditor.db").display().to_string();
    let second = dir.join("second").join("eeditor.db").display().to_string();

    let engine = EngineHandle::spawn(first.clone());
    engine.eval("(def kept \"definitions survive a move\")");
    result(&engine.eval("(add-item \"in first\")"));

    engine.open_database(&second).expect("opens");
    assert_eq!(info(&engine), (second.clone(), None));
    assert_eq!(result(&engine.eval("(item-count)")), "0");
    result(&engine.eval("(add-item \"in second\")"));
    result(&engine.eval("(add-item \"also in second\")"));
    assert!(result(&engine.eval("kept")).contains("survive"));
    // the agenda is named after its file
    assert!(result(&engine.eval("(agendas)")).contains("eeditor [active]"));

    engine.open_database(&first).expect("reopens");
    assert_eq!(result(&engine.eval("(item-count)")), "1");
}

#[test]
fn a_failed_move_lands_in_memory_not_in_the_old_file() {
    let dir = scratch("failed-move");
    let good = dir.join("eeditor.db").display().to_string();
    let blocker = dir.join("not-a-folder");
    std::fs::write(&blocker, "").unwrap();
    let bad = blocker.join("eeditor.db").display().to_string();

    let engine = EngineHandle::spawn(good.clone());
    result(&engine.eval("(add-item \"stays in good\")"));

    let err = engine.open_database(&bad).expect_err("can't open");
    assert!(!err.is_empty());
    let (path, error) = info(&engine);
    assert_eq!(path, ":memory:");
    assert!(error.unwrap().contains(&bad));

    // nothing written now reaches the old workspace's file
    result(&engine.eval("(add-item \"not in good\")"));
    engine.open_database(&good).expect("reopens");
    assert_eq!(result(&engine.eval("(item-count)")), "1");
    assert_eq!(info(&engine), (good, None), "a successful open clears the error");
}
