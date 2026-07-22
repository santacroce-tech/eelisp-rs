//! Tokenizer (ANALYSIS §4.2), with a deliberate clean-break simplification:
//! an atom is read whole and then classified. `-5` is a negative number; `-` (and `+ * /`,
//! `<=` etc.) are symbols. There is NO operator-number splitting and NO positional `-`
//! heuristic — write `(- 1 3)` for subtraction, `(+ 1 3)` for addition.

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

fn is_delim(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '"' | ';' | '\'' | '`' | ',')
}

pub fn lex(src: &str) -> Result<Vec<Token>, LispError> {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut toks = Vec::new();

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
        match c {
            '(' => {
                toks.push(Token::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Token::RParen);
                i += 1;
            }
            '[' => {
                toks.push(Token::LBracket);
                i += 1;
            }
            ']' => {
                toks.push(Token::RBracket);
                i += 1;
            }
            '{' => {
                toks.push(Token::LBrace);
                i += 1;
            }
            '}' => {
                toks.push(Token::RBrace);
                i += 1;
            }
            '\'' => {
                toks.push(Token::Quote);
                i += 1;
            }
            '`' => {
                toks.push(Token::Quasi);
                i += 1;
            }
            ',' => {
                if i + 1 < n && chars[i + 1] == '@' {
                    toks.push(Token::UnquoteSplice);
                    i += 2;
                } else {
                    toks.push(Token::Unquote);
                    i += 1;
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
                toks.push(Token::Str(s));
            }
            _ => {
                let start = i;
                while i < n && !is_delim(chars[i]) {
                    i += 1;
                }
                let atom: String = chars[start..i].iter().collect();
                if let Some(rest) = atom.strip_prefix(':') {
                    toks.push(Token::Kw(rest.to_string()));
                } else if let Ok(num) = atom.parse::<f64>() {
                    toks.push(Token::Num(num));
                } else {
                    toks.push(Token::Sym(atom));
                }
            }
        }
    }
    Ok(toks)
}
