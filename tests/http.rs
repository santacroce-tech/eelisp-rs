//! http-get / http-post options, against a server on this machine: the headers a request carries,
//! and a timeout that gives up on a server that never answers.

#![cfg(feature = "http")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use eelisp::printer::print_value;
use eelisp::Interpreter;

/// A server for one request: answers 200 with `body`, and hands back the request it got.
fn one_shot(body: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        // the whole request — headers, then as much body as Content-Length says — before answering
        let mut req = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = s.read(&mut buf).unwrap();
            req.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&req).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let len = text.to_lowercase().lines()
                    .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if req.len() >= end + 4 + len || n == 0 {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        tx.send(String::from_utf8_lossy(&req).to_string()).unwrap();
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        s.write_all(resp.as_bytes()).unwrap();
    });
    (url, rx)
}

fn ev(it: &Interpreter, src: &str) -> String {
    print_value(&it.eval_str(src).unwrap_or_else(|e| panic!("{src}\n  → {e}")), true)
}

#[test]
fn headers_go_with_the_request() {
    let (url, got) = one_shot("ok");
    let it = Interpreter::new();
    let r = ev(&it, &format!(r#"(http-get "{url}" :headers {{"x-api-key" "secret" "accept" "text/csv"}})"#));
    assert!(r.contains(":status 200") && r.contains(r#":body "ok""#), "{r}");
    let req = got.recv().unwrap().to_lowercase();
    assert!(req.contains("x-api-key: secret"), "{req}");
    assert!(req.contains("accept: text/csv"), "{req}");
}

#[test]
fn a_post_takes_the_same_options() {
    let (url, got) = one_shot("{}");
    let it = Interpreter::new();
    ev(&it, &format!(r#"(http-post "{url}" "a,b" :content-type "text/csv" :headers {{"x-token" "t"}} :timeout 5)"#));
    let req = got.recv().unwrap().to_lowercase();
    assert!(req.contains("content-type: text/csv") && req.contains("x-token: t"), "{req}");
}

#[test]
fn a_server_that_never_answers_gives_up_at_the_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    thread::spawn(move || {
        let (_s, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_secs(10)); // holds the connection, says nothing
    });
    let it = Interpreter::new();
    let started = Instant::now();
    let r = it.eval_str(&format!(r#"(http-get "{url}" :timeout 1)"#));
    assert!(r.is_err(), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "waited {:?}", started.elapsed());
}

#[test]
fn bad_options_say_what_is_wrong() {
    let it = Interpreter::new();
    let e = |src: &str| it.eval_str(src).unwrap_err().to_string();
    assert!(e(r#"(http-get "http://127.0.0.1:9/" :timeout 0)"#).contains(":timeout"));
    assert!(e(r#"(http-get "http://127.0.0.1:9/" :headers "x")"#).contains(":headers is a dict"));
    assert!(e(r#"(http-get "http://127.0.0.1:9/" :retries 3)"#).contains("unknown option :retries"));
}
