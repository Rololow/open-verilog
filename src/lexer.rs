//! Tokens of .ov. Mirrors the reference lexer (ovc/lexer.py) token for token.

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ident,
    Num,
    Op,
    Str,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub text: String,
    pub line: usize,
    pub col: usize,
}

/// Multi-character operators, longest first within each prefix class.
const OPS: &[&str] = &[
    "<<<", ">>>", "===", "!==", "::", "+:", "-:", "<<", ">>", "==", "!=", "<=", ">=", "&&", "||", "**", "++", "--",
    "..", "=>", "->", "+=", "-=", "*=", "/=", "|=", "&=", "^=",
];
const SINGLE: &str = "-+*/%&|^~!<>=?:;,.(){}[]@#'";

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Length in bytes of a sized or based literal starting at `i` (`8'hff`, `'sb1010`, `'1`), if one starts there.
fn based_literal(src: &[u8], mut i: usize) -> Option<usize> {
    let start = i;
    let digits = |i: &mut usize| {
        let s = *i;
        while *i < src.len() && (src[*i].is_ascii_digit() || src[*i] == b'_') {
            *i += 1;
        }
        *i > s
    };
    if i < src.len() && src[i].is_ascii_digit() {
        digits(&mut i);
        while i < src.len() && (src[i] == b' ' || src[i] == b'\t') {
            i += 1;
        }
    }
    if i >= src.len() || src[i] != b'\'' {
        return None;
    }
    i += 1;
    if i < src.len() && (src[i] == b's' || src[i] == b'S') {
        i += 1;
    }
    if i >= src.len() || !b"bBoOdDhH".contains(&src[i]) {
        // `'0`, `'1`, `'x`, `'z`: an unsized single bit
        return (start + 1 < src.len() && b"01xXzZ".contains(&src[start + 1]) && i == start + 1).then_some(2);
    }
    i += 1;
    while i < src.len() && (src[i] == b' ' || src[i] == b'\t') {
        i += 1;
    }
    let body = i;
    while i < src.len() && (src[i].is_ascii_hexdigit() || b"xXzZ?_".contains(&src[i])) {
        i += 1;
    }
    (i > body).then_some(i - start)
}

pub fn lex(src: &str, file: &str) -> Result<Vec<Token>> {
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let (mut i, mut line, mut line_start) = (0usize, 1usize, 0usize);
    let col_of = |at: usize, line_start: usize| src[line_start..at].chars().count() + 1;

    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            if c == '\n' {
                line += 1;
                line_start = i + 1;
            }
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            if let Some(end) = src[i + 2..].find("*/") {
                let body = &src[i..i + 2 + end + 2];
                line += body.matches('\n').count();
                if let Some(nl) = body.rfind('\n') {
                    line_start = i + nl + 1;
                }
                i += body.len();
                continue;
            }
            // an unclosed comment is the operator `/` followed by `*`, as in the reference lexer
        }
        let col = col_of(i, line_start);
        if c == '"' {
            let mut j = i + 1;
            loop {
                match b.get(j) {
                    None | Some(b'\n') => return Err(Error::new("unterminated string", file, line, col)),
                    Some(&b'\x5c') => j += 2,
                    Some(b'"') => break,
                    Some(_) => j += 1,
                }
            }
            toks.push(Token { kind: Kind::Str, text: src[i..=j].to_string(), line, col });
            i = j + 1;
            continue;
        }
        if let Some(len) = based_literal(b, i) {
            let text: String = src[i..i + len].chars().filter(|&c| c != ' ' && c != '\t').collect();
            toks.push(Token { kind: Kind::Num, text, line, col });
            i += len;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'_') {
                j += 1;
            }
            toks.push(Token { kind: Kind::Num, text: src[i..j].to_string(), line, col });
            i = j;
            continue;
        }
        let ident_start = if c == '$' { b.get(i + 1).is_some_and(|&x| is_ident_start(x as char)) } else { is_ident_start(c) };
        if ident_start {
            let mut j = i + 1;
            while j < b.len() && is_ident_char(b[j] as char) {
                j += 1;
            }
            toks.push(Token { kind: Kind::Ident, text: src[i..j].to_string(), line, col });
            i = j;
            continue;
        }
        if let Some(op) = OPS.iter().find(|op| b[i..].starts_with(op.as_bytes())) {
            toks.push(Token { kind: Kind::Op, text: op.to_string(), line, col });
            i += op.len();
            continue;
        }
        if SINGLE.contains(c) {
            toks.push(Token { kind: Kind::Op, text: c.to_string(), line, col });
            i += 1;
            continue;
        }
        return Err(Error::new(format!("unexpected character {c:?}"), file, line, col));
    }
    let col = col_of(i, line_start);
    toks.push(Token { kind: Kind::Eof, text: String::new(), line, col });
    Ok(toks)
}

/// The value of a number literal: decimal, or based with an optional size (`'h1f`, `8'hff`, `'b1010`), underscores
/// allowed. `None` for a literal whose value is not a plain integer (`x`, `z`).
pub fn number_value(text: &str) -> Option<i64> {
    let text: String = text.chars().filter(|c| *c != '_').collect();
    let Some((size, based)) = text.split_once('\'') else {
        return text.parse().ok();
    };
    if !size.is_empty() && size.parse::<u32>().is_err() {
        return None;
    }
    let radix = match based.chars().next()?.to_ascii_lowercase() {
        'h' => 16,
        'd' => 10,
        'o' => 8,
        'b' => 2,
        _ => return None,
    };
    i64::from_str_radix(&based[1..], radix).ok()
}
