//! Minimal REPL / file runner, matching the Swift CLI's shape (ANALYSIS §4.12).
//!   eelisp            — REPL
//!   eelisp <file>     — run a file
//!   eelisp -e "<expr>"— evaluate one expression
//!   eelisp --serve [--workspace <dir>] [--db <file>]
//!                     — JSON-line RPC for a host; `--db` keeps tables and agenda in a file

use std::io::{self, BufRead, Write};

use eelisp::printer::print_value;
use eelisp::{Interpreter, Value};

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for c in s.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth <= 0 && !in_str
}

/// JSON-line RPC over stdin/stdout. Two accepted framings, one `eval_host` envelope out per request:
///   • JSON:  `{"src": "..."}`  — one request per line (newlines survive, escaped in the string)
///   • raw:   bare EELisp, accumulated across lines until parens balance
/// This is the pipe a native host (e.g. a Tauri sidecar / dev bridge) drives.
fn run_serve(it: &Interpreter) {
    it.set_echo(false);
    let stdin = io::stdin();
    let mut buffer = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim();
        // JSON-framed request (only when not mid-accumulation)
        if buffer.is_empty() && trimmed.starts_with('{') {
            if let Ok(req) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(src) = req.get("src").and_then(|s| s.as_str()) {
                    println!("{}", it.eval_host(src));
                    io::stdout().flush().ok();
                    continue;
                }
            }
        }
        // raw framing: accumulate until balanced
        buffer.push_str(&line);
        if balanced(&buffer) && !buffer.trim().is_empty() {
            println!("{}", it.eval_host(buffer.trim()));
            io::stdout().flush().ok();
            buffer.clear();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();

    if args.first().map(String::as_str) == Some("--serve") {
        // `--db <file>`: a host that wants its data kept names the file. Without it the engine is
        // in memory, as it always was — the smoke test relies on starting clean. A file that
        // can't be opened doesn't stop the server: it runs in memory and says so.
        let it = match flag("--db") {
            Some(db) => {
                let it = Interpreter::with_database_or_memory(&db);
                if let Some(e) = it.database_error.borrow().as_ref() {
                    eprintln!("[eelisp] {} — running in memory", e);
                }
                it
            }
            None => Interpreter::new(),
        };
        // `--workspace <dir>`: the dev bridge serves a folder, so tell the engine which one.
        // Without it `(current-dir)` is empty here while the desktop app answers properly, and the
        // same snippet would behave differently in the browser.
        if let Some(dir) = flag("--workspace") {
            it.editor.borrow_mut().current_dir = Some(Box::new(move || dir.clone()));
        }
        run_serve(&it);
        return;
    }

    let it = Interpreter::new();
    if !args.is_empty() {
        if args[0] == "-e" {
            let src = args[1..].join(" ");
            match it.eval_str(&src) {
                Ok(v) => println!("{}", print_value(&v, true)),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            match std::fs::read_to_string(&args[0]) {
                Ok(src) => match it.eval_str(&src) {
                    Ok(v) => {
                        if !matches!(v, Value::Null) {
                            println!("{}", print_value(&v, true));
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    println!("EELisp v0.2 (Rust) — :quit to exit");
    let stdin = io::stdin();
    let mut buffer = String::new();
    loop {
        let prompt = if buffer.is_empty() { "λ> " } else { ".. " };
        print!("{}", prompt);
        io::stdout().flush().ok();

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            println!("\nGoodbye!");
            break;
        }
        let trimmed = line.trim();
        if buffer.is_empty() && (trimmed == ":quit" || trimmed == ":q") {
            break;
        }
        if buffer.is_empty() && trimmed.is_empty() {
            continue;
        }
        buffer.push_str(&line);
        if balanced(&buffer) {
            match it.eval_str(&buffer) {
                Ok(v) => println!("{}", print_value(&v, true)),
                Err(e) => println!("Error: {}", e),
            }
            buffer.clear();
        }
    }
}
