//! Recursive-descent reader (ANALYSIS §4.2). `'` `` ` `` `,` `,@` desugar to
//! `(quote ..)`/`(quasiquote ..)`/`(unquote ..)`/`(unquote-splicing ..)`. `[..]` desugars to
//! `(list ..)`. `{:k v ..}` builds an ordered dict. `true`/`false`/`nil` become Bool/Null.
//!
//! `top_forms` is the same parse, but it also hands back each top-level form's original text and
//! the comment block written directly above it — the raw material `(source f)` echoes back.

use std::rc::Rc;

use crate::lexer::{lex_spanned, Spanned, Token};
use crate::value::{LispError, OrderedDict, Value};

struct Parser {
    toks: Vec<Spanned>,
    pos: usize,
}

/// A top-level form together with how it was written.
pub struct TopForm {
    pub value: Value,
    /// The form's own text, exactly as it appears in the source.
    pub source: String,
    /// The run of `;` comment lines sitting immediately above it, or empty.
    pub comments: String,
}

fn list_val(v: Vec<Value>) -> Value {
    Value::List(Rc::new(v))
}
fn wrap(sym: &str, inner: Value) -> Value {
    list_val(vec![Value::Symbol(sym.into()), inner])
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos).map(|s| &s.tok)
    }
    fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).map(|s| s.tok.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<Value, LispError> {
        let t = self.next().ok_or_else(|| LispError::Parse("unexpected EOF".into()))?;
        match t {
            Token::LParen => self.parse_seq(&Token::RParen).map(list_val),
            Token::LBracket => {
                let mut v = self.parse_seq(&Token::RBracket)?;
                let mut out = vec![Value::Symbol("list".into())];
                out.append(&mut v);
                Ok(list_val(out))
            }
            Token::LBrace => self.parse_dict(),
            Token::Quote => Ok(wrap("quote", self.parse_expr()?)),
            Token::Quasi => Ok(wrap("quasiquote", self.parse_expr()?)),
            Token::Unquote => Ok(wrap("unquote", self.parse_expr()?)),
            Token::UnquoteSplice => Ok(wrap("unquote-splicing", self.parse_expr()?)),
            Token::Str(s) => Ok(Value::Str(s)),
            Token::Num(nu) => Ok(Value::Number(nu)),
            Token::Kw(k) => Ok(Value::Keyword(k)),
            Token::Sym(s) => Ok(match s.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                "nil" => Value::Null,
                _ => Value::Symbol(s.into()),
            }),
            Token::RParen | Token::RBracket | Token::RBrace => {
                Err(LispError::Parse("unexpected closing delimiter".into()))
            }
        }
    }

    fn parse_seq(&mut self, close: &Token) -> Result<Vec<Value>, LispError> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => return Err(LispError::Parse("unclosed list".into())),
                Some(t) if t == close => {
                    self.pos += 1;
                    break;
                }
                _ => out.push(self.parse_expr()?),
            }
        }
        Ok(out)
    }

    fn parse_dict(&mut self) -> Result<Value, LispError> {
        let mut d = OrderedDict::default();
        loop {
            match self.next() {
                None => return Err(LispError::Parse("unclosed dict".into())),
                Some(Token::RBrace) => break,
                // keys may be keywords ({:a 1}) or strings ({"a" 1}, e.g. JSON-shaped literals)
                Some(Token::Kw(k)) => {
                    let v = self.parse_expr()?;
                    d.insert(k, v);
                }
                Some(Token::Str(s)) => {
                    let v = self.parse_expr()?;
                    d.insert(s, v);
                }
                Some(_) => return Err(LispError::Parse("dict keys must be keywords or strings".into())),
            }
        }
        Ok(Value::Dict(Rc::new(d)))
    }

    /// True when the next token is a stray closer at the top level. Lenient top level: skip it
    /// instead of failing the whole parse. Some library files (e.g. parts of the zzeelisp bundle)
    /// ship a hair unbalanced; recovering here lets the rest of the file's definitions load
    /// rather than dropping the entire module.
    fn at_stray_closer(&self) -> bool {
        matches!(self.peek(), Some(Token::RParen) | Some(Token::RBracket) | Some(Token::RBrace))
    }
}

pub fn parse(src: &str) -> Result<Vec<Value>, LispError> {
    Ok(top_forms(src)?.into_iter().map(|f| f.value).collect())
}

pub fn top_forms(src: &str) -> Result<Vec<TopForm>, LispError> {
    let chars: Vec<char> = src.chars().collect();
    let toks = lex_spanned(src)?;
    let mut p = Parser { toks, pos: 0 };
    let mut out = Vec::new();
    while p.pos < p.toks.len() {
        if p.at_stray_closer() {
            p.pos += 1;
            continue;
        }
        let start = p.toks[p.pos].start;
        let value = p.parse_expr()?;
        let end = with_trailing_comment(&chars, p.toks[p.pos - 1].end);
        out.push(TopForm {
            value,
            source: chars[start..end].iter().collect(),
            comments: comment_block_above(&chars, start),
        });
    }
    Ok(out)
}

/// Extend a form's end past a comment that follows it on the same line — the `;; toast` in
/// `(defn ed-message (text) …)  ;; toast` documents that function and nothing else, so it belongs
/// to it. Anything but whitespace between the form and the `;` means the comment is someone else's.
fn with_trailing_comment(chars: &[char], end: usize) -> usize {
    let mut i = end;
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    if i >= chars.len() || chars[i] != ';' {
        return end;
    }
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    i
}

/// The contiguous run of comment lines directly above the char at `start`. A blank line or any
/// line with code on it ends the block, so a form only ever picks up the comment written for it.
fn comment_block_above(chars: &[char], start: usize) -> String {
    // Back up to the first char of the line the form starts on.
    let mut line_start = start;
    while line_start > 0 && chars[line_start - 1] != '\n' {
        line_start -= 1;
    }
    // Anything but whitespace before the form on its own line means no comment block above.
    if chars[line_start..start].iter().any(|c| !c.is_whitespace()) {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut cursor = line_start;
    while cursor > 0 {
        let end = cursor - 1; // the '\n' that ends the previous line
        let mut begin = end;
        while begin > 0 && chars[begin - 1] != '\n' {
            begin -= 1;
        }
        let line: String = chars[begin..end].iter().collect();
        if !line.trim_start().starts_with(';') {
            break;
        }
        lines.push(line);
        cursor = begin;
    }
    lines.reverse();
    lines.join("\n")
}
