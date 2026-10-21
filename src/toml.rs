//! A small reader of the TOML subset that manifests use: `key = value` lines, `[table]` and `[[array of tables]]`
//! headers, strings, integers (decimal, `0x`, `0o`, `0b`, with `_`), booleans and inline arrays. A table becomes a
//! record (`Expr::Pattern` of keyed values), an array a list, a string a quoted literal.

use crate::ast::Expr;

#[derive(Debug, Clone)]
enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<Value>),
    Table(Vec<(String, Value)>),
}

impl Value {
    fn into_expr(self) -> Expr {
        match self {
            Value::Str(s) => Expr::Num(format!("\"{s}\"")),
            Value::Int(i) => Expr::Num(i.to_string()),
            Value::Bool(b) => Expr::Num(i32::from(b).to_string()),
            Value::Array(items) => Expr::List(items.into_iter().map(Value::into_expr).collect()),
            Value::Table(fields) => Expr::Pattern(fields.into_iter().map(|(k, v)| Expr::Keyed(k, Box::new(v.into_expr()))).collect()),
        }
    }
}

/// Removes a comment (a `#` outside a string) from a line.
fn strip_comment(line: &str) -> &str {
    let mut in_string: Option<char> = None;
    for (i, c) in line.char_indices() {
        match (in_string, c) {
            (None, '#') => return &line[..i],
            (None, '"' | '\'') => in_string = Some(c),
            (Some(q), c) if c == q => in_string = None,
            _ => {}
        }
    }
    line
}

fn parse_int(text: &str) -> Option<i64> {
    let t: String = text.chars().filter(|c| *c != '_').collect();
    let (neg, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, t.strip_prefix('+').unwrap_or(&t).to_string()),
    };
    let v = if let Some(h) = t.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok()?
    } else if let Some(o) = t.strip_prefix("0o") {
        i64::from_str_radix(o, 8).ok()?
    } else if let Some(b) = t.strip_prefix("0b") {
        i64::from_str_radix(b, 2).ok()?
    } else {
        t.parse().ok()?
    };
    Some(if neg { -v } else { v })
}

/// One value at the start of `s`; returns it and the rest of the text.
fn parse_value(s: &str) -> Result<(Value, &str), String> {
    let s = s.trim_start();
    let first = s.chars().next().ok_or("a value is missing")?;
    match first {
        '"' => {
            let mut out = String::new();
            let mut chars = s[1..].char_indices();
            while let Some((i, c)) = chars.next() {
                match c {
                    '"' => return Ok((Value::Str(out), &s[i + 2..])),
                    '\\' => match chars.next().map(|(_, e)| e) {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some(e) => out.push(e),
                        None => break,
                    },
                    c => out.push(c),
                }
            }
            Err("a string is not closed".into())
        }
        '\'' => {
            let end = s[1..].find('\'').ok_or("a string is not closed")?;
            Ok((Value::Str(s[1..=end].to_string()), &s[end + 2..]))
        }
        '[' => {
            let mut rest = s[1..].trim_start();
            let mut items = Vec::new();
            loop {
                if let Some(r) = rest.strip_prefix(']') {
                    return Ok((Value::Array(items), r));
                }
                let (v, r) = parse_value(rest)?;
                items.push(v);
                rest = r.trim_start();
                if let Some(r) = rest.strip_prefix(',') {
                    rest = r.trim_start();
                }
            }
        }
        _ => {
            let end = s.find([',', ']', ' ', '\t']).unwrap_or(s.len());
            let word = &s[..end];
            let v = match word {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                w => Value::Int(parse_int(w).ok_or_else(|| format!("'{w}' is not a value"))?),
            };
            Ok((v, &s[end..]))
        }
    }
}

/// The top-level entries of a document, as expressions.
pub fn parse(text: &str) -> Result<Vec<(String, Expr)>, String> {
    // where the `key = value` lines go: the document, a table, or the last record of an array of tables
    enum Target {
        Root,
        Table(String),
        Array(String),
    }
    let mut root: Vec<(String, Value)> = Vec::new();
    let mut target = Target::Root;
    let mut lines = text.lines().enumerate();
    while let Some((n, raw)) = lines.next() {
        let mut line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        let fail = |msg: &str| format!("line {}: {msg}", n + 1);
        if let Some(name) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
            let name = name.trim().to_string();
            match root.iter_mut().find(|(k, _)| *k == name) {
                Some((_, Value::Array(items))) => items.push(Value::Table(Vec::new())),
                Some(_) => return Err(fail(&format!("'{name}' is not an array of tables"))),
                None => root.push((name.clone(), Value::Array(vec![Value::Table(Vec::new())]))),
            }
            target = Target::Array(name);
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim().to_string();
            root.push((name.clone(), Value::Table(Vec::new())));
            target = Target::Table(name);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { return Err(fail("a line is 'key = value' or a table header")) };
        let key = key.trim().trim_matches('"').to_string();
        let mut value = value.trim().to_string();
        // an array may continue on the next lines
        while value.starts_with('[') && value.matches('[').count() > value.matches(']').count() {
            let Some((_, next)) = lines.next() else { return Err(fail("an array is not closed")) };
            value.push(' ');
            value += strip_comment(next).trim();
        }
        line.clear();
        let (v, _) = parse_value(&value).map_err(|e| fail(&e))?;
        let slot = match &target {
            Target::Root => &mut root,
            Target::Table(name) | Target::Array(name) => {
                let entry = root.iter_mut().rev().find(|(k, _)| k == name).map(|(_, v)| v);
                match entry {
                    Some(Value::Table(fields)) => fields,
                    Some(Value::Array(items)) => match items.last_mut() {
                        Some(Value::Table(fields)) => fields,
                        _ => return Err(fail("a record is missing")),
                    },
                    _ => return Err(fail("a table is missing")),
                }
            }
        };
        slot.push((key, v));
    }
    Ok(root.into_iter().map(|(k, v)| (k, v.into_expr())).collect())
}
