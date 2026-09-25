//! Tokenizer (ANALYSIS §4.2), with a deliberate clean-break simplification:
//! an atom is read whole and then classified. `-5` is a negative number; `-` (and `+ * /`,
//! `<=` etc.) are symbols. There is NO operator-number splitting and NO positional `-`
//! heuristic — write `(- 1 3)` for subtraction, `(+ 1 3)` for addition.
//!
//! Tokens carry their **char span** in the source. Nothing in evaluation needs it, but
//! `(source f)` does: to echo a definition the way it was written — comments, indentation and
//! all — something has to remember where in the text each top-level form began and ended.

use crate::value::LispError;

#[derive(Clone, PartialEq, Debug)]
pub enum Token {
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Quote,
    Quasi,
    Unquote,
    UnquoteSplice,
    Str(String),
    Num(f64),
    Kw(String),
    Sym(String),
}

/// A token plus the half-open char range `[start, end)` it occupies in the source.
#[derive(Clone, Debug)]
pub struct Spanned {
    pub tok: Token,
    pub start: usize,
    pub end: usize,
}

fn is_delim(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '"' | ';' | '\'' | '`' | ',')
}

/// `0x4000`, `0xff`, `-0x10` — hex integer literals. Anything else (`0x`, `0xg1`) stays a symbol.
/// Up to 13 hex digits, so the value is always an exact `f64` integer.
fn parse_hex(atom: &str) -> Option<f64> {
    let (neg, body) = match atom.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, atom),
    };
    let digits = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X"))?;
    if digits.is_empty() || digits.len() > 13 {
        return None;
    }
    let v = i64::from_str_radix(digits, 16).ok()? as f64;
    Some(if neg { -v } else { v })
}

/// Tokens only — the parser's fast path and every existing caller.
pub fn lex(src: &str) -> Result<Vec<Token>, LispError> {
    Ok(lex_spanned(src)?.into_iter().map(|s| s.tok).collect())
}

/// Tokens with their source spans. Comments and whitespace are skipped as usual; they are
/// recovered later from the raw text between spans (see `parser::top_forms`).
pub fn lex_spanned(src: &str) -> Result<Vec<Spanned>, LispError> {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut toks: Vec<Spanned> = Vec::new();

    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == ';' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        let tok = match c {
            '(' => {
                i += 1;
                Token::LParen
            }
            ')' => {
                i += 1;
                Token::RParen
            }
            '[' => {
                i += 1;
                Token::LBracket
            }
            ']' => {
                i += 1;
                Token::RBracket
            }
            '{' => {
                i += 1;
                Token::LBrace
            }
            '}' => {
                i += 1;
                Token::RBrace
            }
            '\'' => {
                i += 1;
                Token::Quote
            }
            '`' => {
                i += 1;
                Token::Quasi
            }
            ',' => {
                if i + 1 < n && chars[i + 1] == '@' {
                    i += 2;
                    Token::UnquoteSplice
                } else {
                    i += 1;
                    Token::Unquote
                }
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                while i < n && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < n {
                        let e = chars[i + 1];
                        match e {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            other => {
                                s.push('\\');
                                s.push(other);
                            }
                        }
                        i += 2;
                    } else {
                        s.push(chars[i]);
                        i += 1;
                    }
                }
                if i >= n {
                    return Err(LispError::Parse("unterminated string".into()));
                }
                i += 1; // closing quote
                Token::Str(s)
            }
            _ => {
                while i < n && !is_delim(chars[i]) {
                    i += 1;
                }
                let atom: String = chars[start..i].iter().collect();
                if let Some(rest) = atom.strip_prefix(':') {
                    Token::Kw(rest.to_string())
                } else if let Ok(num) = atom.parse::<f64>() {
                    Token::Num(num)
                } else if let Some(num) = parse_hex(&atom) {
                    Token::Num(num)
                } else {
                    Token::Sym(atom)
                }
            }
        };
        toks.push(Spanned { tok, start, end: i });
    }
    Ok(toks)
}
