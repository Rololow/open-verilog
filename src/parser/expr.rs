//! Expressions: precedence climbing over binary operators, then unary, postfix and primary forms.

use super::{binop_prec, Parser, REL_PREC, UNOPS};
use crate::ast::{Expr, SetItem};
use crate::error::Result;
use crate::lexer::Kind;

impl Parser {
    pub(super) fn parse_expr(&mut self, min_prec: u8) -> Result<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let t = self.tok().clone();
            if t.kind == Kind::Op && t.text == "?" && min_prec == 0 {
                self.i += 1;
                let a = self.parse_expr(0)?;
                self.expect(":")?;
                let b = self.parse_expr(0)?;
                return Ok(Expr::Ternary(Box::new(lhs), Box::new(a), Box::new(b)));
            }
            if t.kind == Kind::Ident && t.text == "in" && min_prec.max(1) <= REL_PREC {
                self.i += 1;
                lhs = Expr::InSet(Box::new(lhs), self.parse_set()?);
                continue;
            }
            let mut prec = if t.kind == Kind::Op { binop_prec(&t.text) } else { None };
            if self.in_generic && (t.text == ">>" || t.text == ">>>") {
                prec = None; // A<B<N>>: closes two generics; write (a >> b) for a shift
            }
            match prec {
                Some(p) if p >= min_prec.max(1) => {
                    self.i += 1;
                    let rhs = self.parse_expr(p + 1)?;
                    lhs = Expr::Binary(t.text, Box::new(lhs), Box::new(rhs), (t.line, t.col));
                }
                _ => return Ok(lhs),
            }
        }
    }

    fn parse_set_item(&mut self) -> Result<SetItem> {
        let lo = self.parse_expr(2)?;
        let hi = if self.accept("..") { Some(self.parse_expr(2)?) } else { None };
        Ok(SetItem { lo, hi })
    }

    /// `{a, lo..hi}` or a single `lo..hi`.
    pub(super) fn parse_set(&mut self) -> Result<Vec<SetItem>> {
        if self.accept("{") {
            return self.comma_list("}", |p| p.parse_set_item());
        }
        Ok(vec![self.parse_set_item()?])
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        let t = self.tok().clone();
        if t.kind == Kind::Op && binop_prec(&t.text).is_some() && matches!(self.peek(1).text.as_str(), ")" | ",") {
            self.i += 1; // reduce(+)
            return Ok(Expr::OpRef(t.text));
        }
        if t.kind == Kind::Op && UNOPS.contains(&t.text.as_str()) {
            self.i += 1;
            return Ok(Expr::Unary(t.text, Box::new(self.parse_unary()?)));
        }
        self.parse_postfix()
    }

    pub(super) fn parse_postfix(&mut self) -> Result<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            let t = self.tok().clone();
            if self.accept(".") {
                let (line, col) = (self.tok().line, self.tok().col);
                e = Expr::Member(Box::new(e), self.ident()?, (line, col));
            } else if !self.no_index && self.accept("[") {
                let first = self.parse_expr(0)?;
                if matches!(self.tok().text.as_str(), ":" | "+:" | "-:") {
                    let op = self.tok().text.clone();
                    self.i += 1;
                    let width = self.parse_expr(0)?;
                    e = Expr::Slice(Box::new(e), Box::new(first), Box::new(width), op);
                } else {
                    e = Expr::Index(Box::new(e), Box::new(first));
                }
                self.expect("]")?;
            } else if matches!(&e, Expr::Member(_, name, _) if name == "as") && self.at("(") && self.peek(1).kind == Kind::Ident && self.peek(2).text == "<" {
                // `.as(Struct<args>)`: the argument is a type
                self.i += 1;
                let dt = self.parse_dtype(false)?;
                self.expect(")")?;
                e = Expr::Call(Box::new(e), vec![Expr::Type(dt)]);
            } else if self.accept("(") {
                e = Expr::Call(Box::new(e), self.comma_list(")", |p| p.parse_expr(0))?);
            } else if t.text == "with" && self.peek(1).text == "{" {
                // `base with { field = value; ... }`: the separator is `;` or `,` (as the reference accepts both)
                let pos = (t.line, t.col);
                self.i += 2;
                let mut overrides = Vec::new();
                while !self.at("}") {
                    let name = self.ident()?;
                    self.expect("=")?;
                    overrides.push((name, self.parse_expr(0)?));
                    if !(self.accept(";") || self.accept(",")) {
                        break;
                    }
                }
                self.expect("}")?;
                e = Expr::With(Box::new(e), overrides, pos);
            } else if t.text == "'" && self.peek(1).text == "(" {
                self.i += 2; // SV cast: 8'(x)
                let arg = self.parse_expr(0)?;
                self.expect(")")?;
                e = Expr::Cast(Box::new(e), Box::new(arg));
            } else {
                return Ok(e);
            }
        }
    }

    /// At `(`: is this `(a, b) => body`?
    fn is_lambda_params(&self) -> bool {
        let mut n = 1;
        while self.peek(n).kind == Kind::Ident {
            n += 1;
            if self.peek(n).text != "," {
                break;
            }
            n += 1;
        }
        n > 1 && self.peek(n).text == ")" && self.peek(n + 1).text == "=>"
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let t = self.tok().clone();
        if t.text == "'" && self.peek(1).text == "{" {
            // '{a, b, key: c}: the values of an array, by position or by key
            self.i += 2;
            let items = self.comma_list("}", |p| {
                let key = match (p.tok().kind, p.peek(1).text.as_str()) {
                    (Kind::Ident | Kind::Num, ":") => {
                        let k = p.tok().text.clone();
                        p.i += 2;
                        Some(k)
                    }
                    _ => None,
                };
                let value = p.parse_expr(0)?;
                Ok(match key {
                    Some(k) => Expr::Keyed(k, Box::new(value)),
                    None => value,
                })
            })?;
            return Ok(Expr::Pattern(items));
        }
        if t.kind == Kind::Str {
            // adjacent literals join: "a" "b" is "ab"
            let mut text = t.text.clone();
            self.i += 1;
            while self.tok().kind == Kind::Str {
                text = format!("{}{}", &text[..text.len() - 1], &self.tok().text[1..]);
                self.i += 1;
            }
            return Ok(Expr::Num(text));
        }
        if t.kind == Kind::Num {
            self.i += 1;
            return Ok(Expr::Num(t.text));
        }
        if t.kind == Kind::Ident {
            if t.text == "match" && self.peek(1).text == "(" {
                self.i += 2;
                let subject = self.parse_expr(0)?;
                self.expect(")")?;
                self.expect("{")?;
                let mut arms = Vec::new();
                while !self.accept("}") {
                    if self.tok().kind == Kind::Eof {
                        return Err(self.fail("'}' missing"));
                    }
                    let labels = if self.accept("default") {
                        None
                    } else {
                        let mut set = vec![self.parse_set_item()?];
                        while self.accept(",") {
                            set.push(self.parse_set_item()?);
                        }
                        Some(set)
                    };
                    self.expect(":")?;
                    arms.push((labels, self.parse_expr(0)?));
                    self.accept(";");
                }
                return Ok(Expr::Match(Box::new(subject), arms));
            }
            if self.peek(1).text == "=>" {
                self.i += 2;
                return Ok(Expr::Lambda(vec![t.text], Box::new(self.parse_expr(0)?)));
            }
            self.i += 1;
            if self.accept("::") {
                // Pkg::name: packages are flattened
                let name = self.tok().clone();
                self.i += 1;
                return Ok(Expr::Ident(name.text, (name.line, name.col)));
            }
            return Ok(Expr::Ident(t.text, (t.line, t.col)));
        }
        if self.at("(") && self.is_lambda_params() {
            self.i += 1;
            let mut params = Vec::new();
            while !self.accept(")") {
                params.push(self.ident()?);
                self.accept(",");
            }
            self.expect("=>")?;
            return Ok(Expr::Lambda(params, Box::new(self.parse_expr(0)?)));
        }
        if self.accept("(") {
            let saved = std::mem::replace(&mut self.in_generic, false);
            let e = self.parse_expr(0);
            self.in_generic = saved;
            let e = e?;
            self.expect(")")?;
            return Ok(e);
        }
        if self.accept("[") {
            return Ok(Expr::List(self.comma_list("]", |p| p.parse_expr(0))?));
        }
        if self.accept("{") {
            let first = self.parse_expr(0)?;
            if self.accept("{") {
                // replication {n{a, b}}: the inner `}` is closed by comma_list, the outer one here
                let parts = self.comma_list("}", |p| p.parse_expr(0))?;
                self.expect("}")?;
                return Ok(Expr::Repl(Box::new(first), parts));
            }
            let mut parts = vec![first];
            while self.accept(",") {
                parts.push(self.parse_expr(0)?);
            }
            self.expect("}")?;
            return Ok(Expr::Concat(parts));
        }
        Err(self.fail(format!("expression expected, found '{}'", self.found())))
    }
}
