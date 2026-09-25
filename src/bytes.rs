//! Byte buffers and bitwise integers — what code that works on raw memory needs.
//!
//! A byte buffer is the one **mutable** value in EELisp: `(bset b i v)` changes the buffer for
//! everyone holding it, the way a machine's RAM behaves. Everything else stays immutable. A buffer
//! crosses the JSON envelope as base64 (`{"$bytes": "…"}`), which a browser's `atob` reads.
//!
//! Numbers are `f64`, so the bitwise builtins take integers only — a fraction, or anything past
//! 2^53 where an `f64` stops being exact, is an error rather than a silently wrong answer.

use std::cell::RefCell;
use std::rc::Rc;

use crate::env::{self, Env};
use crate::value::*;

/// The largest integer an `f64` holds exactly.
const EXACT: f64 = 9_007_199_254_740_992.0;

fn fail(msg: impl Into<String>) -> LispError {
    LispError::Runtime(msg.into())
}

fn define(env: &Env, name: &str, f: impl Fn(&[Value], &Env) -> Result<Value, LispError> + 'static) {
    env::define(
        env,
        name,
        Value::Builtin(Rc::new(Builtin { name: name.to_string(), arg_mode: ArgMode::Eval, func: Box::new(f) })),
    );
}

fn arity(func: &str, args: &[Value], min: usize, max: usize) -> Result<(), LispError> {
    if args.len() < min || args.len() > max {
        let expected = if min == max { min.to_string() } else { format!("{min}-{max}") };
        return Err(LispError::Arity { func: func.into(), expected, got: args.len() });
    }
    Ok(())
}

/// An exact integer argument.
pub(crate) fn int(func: &str, v: &Value) -> Result<i64, LispError> {
    match v {
        Value::Number(n) if n.fract() == 0.0 && n.abs() <= EXACT => Ok(*n as i64),
        Value::Number(n) => Err(fail(format!("{func}: {n} isn't an exact integer"))),
        other => Err(LispError::TypeMismatch { expected: "number".into(), got: type_name(other) }),
    }
}

fn num(func: &str, n: i64) -> Result<Value, LispError> {
    if (n as f64).abs() > EXACT {
        return Err(fail(format!("{func}: the result is past 2^53, where numbers stop being exact")));
    }
    Ok(Value::Number(n as f64))
}

pub(crate) fn buf(v: &Value) -> Result<Rc<RefCell<Vec<u8>>>, LispError> {
    match v {
        Value::Bytes(b) => Ok(b.clone()),
        other => Err(LispError::TypeMismatch { expected: "bytes".into(), got: type_name(other) }),
    }
}

/// An index into a buffer of `len` bytes; `end` allows `len` itself (the end of a run).
fn index(func: &str, v: &Value, len: usize, end: bool) -> Result<usize, LispError> {
    let i = int(func, v)?;
    let limit = if end { len as i64 } else { len as i64 - 1 };
    if i < 0 || i > limit {
        return Err(LispError::IndexOutOfBounds { index: i, len });
    }
    Ok(i as usize)
}

/// `start` and `len` naming a run inside a buffer of `total` bytes.
fn run(func: &str, start: &Value, len: &Value, total: usize) -> Result<(usize, usize), LispError> {
    let s = index(func, start, total, true)?;
    let n = int(func, len)?;
    if n < 0 || s + n as usize > total {
        return Err(fail(format!("{func}: {n} bytes from {s} runs past the end of {total}")));
    }
    Ok((s, n as usize))
}

pub(crate) fn new_bytes(v: Vec<u8>) -> Value {
    Value::Bytes(Rc::new(RefCell::new(v)))
}

pub fn register(env: &Env) {
    // ── bitwise ──
    define(env, "band", |a, _| {
        arity("band", a, 2, 2)?;
        num("band", int("band", &a[0])? & int("band", &a[1])?)
    });
    define(env, "bor", |a, _| {
        arity("bor", a, 2, 2)?;
        num("bor", int("bor", &a[0])? | int("bor", &a[1])?)
    });
    define(env, "bxor", |a, _| {
        arity("bxor", a, 2, 2)?;
        num("bxor", int("bxor", &a[0])? ^ int("bxor", &a[1])?)
    });
    define(env, "bnot", |a, _| {
        arity("bnot", a, 1, 1)?;
        num("bnot", !int("bnot", &a[0])?)
    });
    define(env, "shl", |a, _| {
        arity("shl", a, 2, 2)?;
        let (x, n) = (int("shl", &a[0])?, int("shl", &a[1])?);
        if !(0..=53).contains(&n) {
            return Err(fail(format!("shl: can't shift by {n}")));
        }
        num("shl", x.checked_shl(n as u32).filter(|r| r >> n == x).ok_or_else(|| fail("shl: the result is past 2^53"))?)
    });
    define(env, "shr", |a, _| {
        arity("shr", a, 2, 2)?;
        let (x, n) = (int("shr", &a[0])?, int("shr", &a[1])?);
        if !(0..=63).contains(&n) {
            return Err(fail(format!("shr: can't shift by {n}")));
        }
        num("shr", x >> n)
    });

    // ── byte buffers ──
    define(env, "make-bytes", |a, _| {
        arity("make-bytes", a, 1, 2)?;
        let n = int("make-bytes", &a[0])?;
        if !(0..=16 << 20).contains(&n) {
            return Err(fail(format!("make-bytes: {n} isn't a size from 0 to 16 MB")));
        }
        let fill = match a.get(1) {
            Some(v) => int("make-bytes", v)? as u8,
            None => 0,
        };
        Ok(new_bytes(vec![fill; n as usize]))
    });
    define(env, "bytes-len", |a, _| {
        arity("bytes-len", a, 1, 1)?;
        Ok(Value::Number(buf(&a[0])?.borrow().len() as f64))
    });
    define(env, "bget", |a, _| {
        arity("bget", a, 2, 2)?;
        let b = buf(&a[0])?;
        let b = b.borrow();
        let i = index("bget", &a[1], b.len(), false)?;
        Ok(Value::Number(b[i] as f64))
    });
    define(env, "bset", |a, _| {
        arity("bset", a, 3, 3)?;
        let b = buf(&a[0])?;
        let mut b = b.borrow_mut();
        let i = index("bset", &a[1], b.len(), false)?;
        let v = int("bset", &a[2])? as u8; // & 0xff, two's complement for negatives
        b[i] = v;
        Ok(Value::Number(v as f64))
    });
    define(env, "bfill", |a, _| {
        arity("bfill", a, 4, 4)?;
        let b = buf(&a[0])?;
        let mut b = b.borrow_mut();
        let (s, n) = run("bfill", &a[1], &a[2], b.len())?;
        let v = int("bfill", &a[3])? as u8;
        b[s..s + n].fill(v);
        Ok(Value::Null)
    });
    define(env, "bcopy", |a, _| {
        arity("bcopy", a, 5, 5)?;
        let (dst, src) = (buf(&a[0])?, buf(&a[2])?);
        let n = &a[4];
        if Rc::ptr_eq(&dst, &src) {
            let mut b = dst.borrow_mut();
            let (si, len) = run("bcopy", &a[3], n, b.len())?;
            let (di, _) = run("bcopy", &a[1], n, b.len())?;
            b.copy_within(si..si + len, di);
        } else {
            let s = src.borrow();
            let mut d = dst.borrow_mut();
            let (si, len) = run("bcopy", &a[3], n, s.len())?;
            let (di, _) = run("bcopy", &a[1], n, d.len())?;
            d[di..di + len].copy_from_slice(&s[si..si + len]);
        }
        Ok(Value::Null)
    });
    define(env, "list->bytes", |a, _| {
        arity("list->bytes", a, 1, 1)?;
        match &a[0] {
            Value::List(l) => {
                Ok(new_bytes(l.iter().map(|v| int("list->bytes", v).map(|n| n as u8)).collect::<Result<_, _>>()?))
            }
            other => Err(LispError::TypeMismatch { expected: "list".into(), got: type_name(other) }),
        }
    });
    define(env, "bytes->list", |a, _| {
        arity("bytes->list", a, 1, 1)?;
        let b = buf(&a[0])?;
        let items = b.borrow().iter().map(|&x| Value::Number(x as f64)).collect();
        Ok(Value::List(Rc::new(items)))
    });
    define(env, "bytes->base64", |a, _| {
        arity("bytes->base64", a, 1, 3)?;
        let b = buf(&a[0])?;
        let b = b.borrow();
        let (s, n) = match (a.get(1), a.get(2)) {
            (Some(s), Some(n)) => run("bytes->base64", s, n, b.len())?,
            (None, None) => (0, b.len()),
            _ => return Err(fail("bytes->base64: give both start and length, or neither")),
        };
        Ok(Value::Str(base64(&b[s..s + n])))
    });
    define(env, "base64->bytes", |a, _| {
        arity("base64->bytes", a, 1, 1)?;
        match &a[0] {
            Value::Str(s) => Ok(new_bytes(unbase64(s).ok_or_else(|| fail("base64->bytes: that isn't base64"))?)),
            other => Err(LispError::TypeMismatch { expected: "string".into(), got: type_name(other) }),
        }
    });
}

/// Standard base64 (RFC 4648, with padding) — what a browser's `atob` reads.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Bytes from standard base64; None when it isn't.
pub(crate) fn unbase64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    if clean.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for q in clean.chunks(4) {
        let pad = q.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in q.iter().enumerate() {
            n |= if i >= 4 - pad { 0 } else { val(c)? } << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod base64_tests {
    #[test]
    fn decodes_what_it_encodes_and_refuses_what_isnt_base64() {
        for input in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            assert_eq!(super::unbase64(&super::base64(input.as_bytes())).unwrap(), input.as_bytes());
        }
        assert!(super::unbase64("abc").is_none());
        assert!(super::unbase64("ab!d").is_none());
    }

    #[test]
    fn matches_the_rfc_examples() {
        for (input, want) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(super::base64(input.as_bytes()), want);
        }
    }
}
