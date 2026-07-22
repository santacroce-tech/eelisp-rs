//! Recursive-descent reader (ANALYSIS §4.2). `'` `` ` `` `,` `,@` desugar to
//! `(quote ..)`/`(quasiquote ..)`/`(unquote ..)`/`(unquote-splicing ..)`. `[..]` desugars to
//! `(list ..)`. `{:k v ..}` builds an ordered dict. `true`/`false`/`nil` become Bool/Null.

use std::rc::Rc;

use crate::lexer::{lex, Token};
use crate::value::{LispError, OrderedDict, Value};

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

fn list_val(v: Vec<Value>) -> Value {
    Value::List(Rc::new(v))
}
fn wrap(sym: &str, inner: Value) -> Value {
    list_val(vec![Value::Symbol(sym.into()), inner])
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
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
                _ => Value::Symbol(s),
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
}

pub fn parse(src: &str) -> Result<Vec<Value>, LispError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    let mut out = Vec::new();
    while p.pos < p.toks.len() {
        // Lenient top level: skip a stray closing delimiter instead of failing the whole parse.
        // Some library files (e.g. parts of the zzeelisp bundle) ship a hair unbalanced; recovering
        // here lets the rest of the file's definitions load rather than dropping the entire module.
        if matches!(p.peek(), Some(Token::RParen) | Some(Token::RBracket) | Some(Token::RBrace)) {
            p.pos += 1;
            continue;
        }
        out.push(p.parse_expr()?);
    }
    Ok(out)
}
