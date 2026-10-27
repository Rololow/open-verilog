//! Statements, case arms and `for` headers.

use super::{Parser, ASSIGN_OPS, BASE_TYPES};
use crate::ast::{Expr, ForHeader, Stmt};
use crate::error::Result;
use crate::lexer::Kind;

impl Parser {
    pub(super) fn parse_stmt(&mut self) -> Result<Stmt> {
        if self.at_block() && !self.is_concat_lvalue() {
            let closer = self.block_open()?;
            let mut stmts = Vec::new();
            while !self.at(closer) {
                if self.tok().kind == Kind::Eof {
                    return Err(self.fail(format!("'{closer}' missing")));
                }
                stmts.push(self.parse_stmt()?);
            }
            self.block_close(closer)?;
            return Ok(Stmt::Block(stmts));
        }
        match self.tok().text.as_str() {
            "if" => {
                self.i += 1;
                self.expect("(")?;
                let cond = self.parse_expr(0)?;
                self.expect(")")?;
                let then = Box::new(self.parse_stmt()?);
                let els = if self.accept("else") { Some(Box::new(self.parse_stmt()?)) } else { None };
                Ok(Stmt::If { cond, then, els })
            }
            "for" => {
                let header = self.parse_for_header()?;
                let pre = std::mem::take(&mut self.unpack);
                let body = self.parse_stmt()?;
                let body = if pre.is_empty() {
                    body
                } else {
                    let mut stmts: Vec<Stmt> = pre.into_iter().map(|(n, v)| Stmt::Val(n, v)).collect();
                    stmts.push(body);
                    Stmt::Block(stmts)
                };
                Ok(Stmt::For { header, body: Box::new(body) })
            }
            "case" | "casez" | "casex" => self.parse_case(),
            "goto" => {
                self.i += 1;
                let e = self.parse_expr(0)?;
                self.expect(";")?;
                Ok(Stmt::Goto(e))
            }
            "val" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                self.expect("=")?;
                let e = self.parse_expr(0)?;
                self.expect(";")?;
                Ok(Stmt::Val(name, e))
            }
            "val" if self.peek(1).text == "(" => {
                let pos = (self.tok().line, self.tok().col);
                self.i += 1;
                let names = self.tuple_names()?;
                self.expect("=")?;
                let e = self.parse_expr(0)?;
                self.expect(";")?;
                // a block of bindings: the statements after it see the names (see `fold::fold_block`)
                Ok(Stmt::Block(self.tuple_bindings(&names, e, pos).into_iter().map(|(n, v)| Stmt::Val(n, v)).collect()))
            }
            "return" => {
                self.i += 1;
                let e = self.parse_expr(0)?;
                self.expect(";")?;
                Ok(Stmt::Return(e))
            }
            // immediate assertions are checked by the tools, not emitted: dropped here
            "assert" | "assume" | "cover" if self.peek(1).text == "(" => {
                self.i += 1;
                self.expect("(")?;
                self.parse_expr(0)?;
                self.expect(")")?;
                if self.accept("else") {
                    self.parse_stmt()?;
                } else {
                    self.expect(";")?;
                }
                Ok(Stmt::Block(vec![]))
            }
            // synthesis hints: same behavior
            "unique" | "unique0" | "priority" if matches!(self.peek(1).text.as_str(), "case" | "casez" | "casex" | "if") => {
                self.i += 1;
                self.parse_stmt()
            }
            "automatic" | "static" if BASE_TYPES.contains(&self.peek(1).text.as_str()) => {
                self.i += 1;
                self.parse_stmt()
            }
            t if BASE_TYPES.contains(&t) && (self.peek(1).kind == Kind::Ident || matches!(self.peek(1).text.as_str(), "[" | "signed")) => {
                Ok(Stmt::Decl(self.parse_decl()?))
            }
            _ => self.parse_simple_stmt(),
        }
    }

    /// `lhs = rhs;`, `lhs <= rhs;`, `lhs op= rhs;`, `lhs++;`, or a call statement.
    fn parse_simple_stmt(&mut self) -> Result<Stmt> {
        let start = (self.tok().line, self.tok().col);
        let lhs = self.parse_postfix()?;
        if self.accept(";") {
            return Ok(Stmt::ExprStmt(lhs));
        }
        let (line, col) = (self.tok().line, self.tok().col);
        let stmt = if self.accept("<=") {
            Stmt::Assign { rhs: self.parse_expr(0)?, lhs, nonblocking: true, pos: start }
        } else if self.accept("=") {
            Stmt::Assign { rhs: self.parse_expr(0)?, lhs, nonblocking: false, pos: start }
        } else if let Some((_, op)) = ASSIGN_OPS.iter().find(|(text, _)| self.at(text)) {
            let op = op.to_string();
            self.i += 1;
            let rhs = Expr::Binary(op, Box::new(lhs.clone()), Box::new(self.parse_expr(0)?), (line, col));
            Stmt::Assign { lhs, rhs, nonblocking: self.in_ff, pos: start }
        } else if self.at("++") || self.at("--") {
            let op = if self.at("++") { "+" } else { "-" };
            self.i += 1;
            let rhs = Expr::Binary(op.into(), Box::new(lhs.clone()), Box::new(Expr::Num("1".into())), (line, col));
            Stmt::Assign { lhs, rhs, nonblocking: self.in_ff, pos: start }
        } else {
            return Err(self.fail(format!("'=' or '<=' expected, found '{}'", self.found())));
        };
        self.expect(";")?;
        Ok(stmt)
    }

    /// At `{`: is this `{a, b} = ...` (assignment to a concatenation) rather than a block?
    fn is_concat_lvalue(&self) -> bool {
        if !self.at("{") {
            return false;
        }
        let mut depth: i32 = 0;
        let mut n = 0;
        while self.peek(n).kind != Kind::Eof {
            let tx = self.peek(n).text.as_str();
            depth += (tx == "{") as i32 - (tx == "}") as i32;
            n += 1;
            if depth == 0 {
                return matches!(self.peek(n).text.as_str(), "=" | "<=");
            }
        }
        false
    }

    fn parse_case(&mut self) -> Result<Stmt> {
        let pos = (self.tok().line, self.tok().col);
        let kind = self.ident()?;
        self.expect("(")?;
        let subject = self.parse_expr(0)?;
        self.expect(")")?;
        let closer = if self.accept("{") { "}" } else { "endcase" };
        let mut arms = Vec::new();
        while !self.at(closer) {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail(format!("'{closer}' missing")));
            }
            let labels = if self.accept("default") {
                None
            } else {
                let mut labels = vec![self.parse_expr(0)?];
                while self.accept(",") {
                    labels.push(self.parse_expr(0)?);
                }
                Some(labels)
            };
            self.expect(":")?;
            arms.push((labels, self.parse_stmt()?));
        }
        self.expect(closer)?;
        Ok(Stmt::Case { subject, kind, arms, pos })
    }

    /// `for (int i = lo; i <cmp> bound; i++ | i += k | i = i + k)` or `for i in range`.
    /// `i < a && i < b` is taken as `i < min(a, b)`.
    /// `(a, b, ...)`: the names of a tuple, as in `val (a, b) = ...` and `for (a, b) in ...`.
    pub(super) fn tuple_names(&mut self) -> Result<Vec<String>> {
        self.expect("(")?;
        self.comma_list(")", |p| p.ident())
    }

    /// The bindings of a tuple value: a constant temporary holding the list, then one name per element.
    pub(super) fn tuple_bindings(&self, names: &[String], value: Expr, pos: (usize, usize)) -> Vec<(String, Expr)> {
        let tmp = format!("ov_tuple_{}_{}", pos.0, pos.1);
        let mut out = vec![(tmp.clone(), value)];
        for (k, n) in names.iter().enumerate() {
            out.push((n.clone(), Expr::Index(Box::new(Expr::Ident(tmp.clone(), pos)), Box::new(Expr::Num(k.to_string())))));
        }
        out
    }

    pub(super) fn parse_for_header(&mut self) -> Result<ForHeader> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("for")?;
        // `for (a, b) in list`: a loop over the rows; the names are bound to the elements of each row
        if self.at("(") && self.peek(1).kind == Kind::Ident && self.peek(2).text == "," {
            let names = self.tuple_names()?;
            self.expect("in")?;
            let list = self.parse_expr(1)?;
            let var = format!("ov_row_{}_{}", pos.0, pos.1);
            let len = Expr::Call(Box::new(Expr::Member(Box::new(list.clone()), "len".into(), pos)), Vec::new());
            let row = Expr::Index(Box::new(list), Box::new(Expr::Ident(var.clone(), pos)));
            self.unpack = names
                .iter()
                .enumerate()
                .map(|(k, n)| (n.clone(), Expr::Index(Box::new(row.clone()), Box::new(Expr::Num(k.to_string())))))
                .collect();
            return Ok(ForHeader { var, lo: Expr::Num("0".into()), bound: Some(len), cmp: "<".into(), step: None, pos });
        }
        if !self.accept("(") {
            let var = self.ident()?;
            self.expect("in")?;
            let lo = self.parse_expr(1)?;
            return Ok(ForHeader { var, lo, bound: None, cmp: "<".into(), step: None, pos });
        }
        if matches!(self.tok().text.as_str(), "int" | "integer" | "genvar" | "logic" | "bit" | "byte") {
            self.i += 1;
        }
        let var = self.ident()?;
        self.expect("=")?;
        let lo = self.parse_expr(0)?;
        self.expect(";")?;
        let mut cond = self.parse_expr(0)?;
        cond = self.merge_min_bound(&var, cond, pos);
        let (cmp, bound) = match &cond {
            Expr::Binary(op, l, r, _) if matches!(op.as_str(), "<" | "<=" | ">" | ">=" | "!=") && matches!(&**l, Expr::Ident(n, _) if *n == var) => {
                (op.clone(), (**r).clone())
            }
            _ => return Err(self.fail(format!("unsupported loop condition: expected '{var} < bound' (or <=, >, >=, !=)"))),
        };
        self.expect(";")?;
        let step = self.parse_for_step(&var)?;
        self.expect(")")?;
        Ok(ForHeader { var, lo, bound: Some(bound), cmp, step: Some(step), pos })
    }

    fn merge_min_bound(&self, var: &str, cond: Expr, pos: (usize, usize)) -> Expr {
        let bound_of = |e: &Expr| match e {
            Expr::Binary(op, l, r, _) if op == "<" && matches!(&**l, Expr::Ident(n, _) if n == var) => Some((**r).clone()),
            _ => None,
        };
        if let Expr::Binary(op, l, r, _) = &cond {
            if op == "&&" {
                if let (Some(a), Some(b)) = (bound_of(l), bound_of(r)) {
                    let min = Expr::Call(Box::new(Expr::Ident("min".into(), pos)), vec![a, b]);
                    return Expr::Binary("<".into(), Box::new(Expr::Ident(var.into(), pos)), Box::new(min), pos);
                }
            }
        }
        cond
    }

    fn parse_for_step(&mut self, var: &str) -> Result<Expr> {
        let one = || Expr::Num("1".into());
        if self.at("++") || self.at("--") {
            let neg = self.at("--");
            self.i += 1;
            if self.ident()? != var {
                return Err(self.fail(format!("the loop step must update '{var}'")));
            }
            return Ok(if neg { Expr::Unary("-".into(), Box::new(one())) } else { one() });
        }
        if self.ident()? != var {
            return Err(self.fail(format!("the loop step must update '{var}'")));
        }
        if self.accept("++") {
            Ok(one())
        } else if self.accept("--") {
            Ok(Expr::Unary("-".into(), Box::new(one())))
        } else if self.accept("+=") {
            self.parse_expr(0)
        } else if self.accept("-=") {
            Ok(Expr::Unary("-".into(), Box::new(self.parse_expr(0)?)))
        } else if self.accept("=") {
            if self.ident()? != var || !(self.at("+") || self.at("-")) {
                return Err(self.fail(format!("unsupported loop step: expected '{var} = {var} + k'")));
            }
            let neg = self.at("-");
            self.i += 1;
            let k = self.parse_expr(0)?;
            Ok(if neg { Expr::Unary("-".into(), Box::new(k)) } else { k })
        } else {
            Err(self.fail(format!("unsupported loop step: expected '{var}++', '{var} += k' or '{var}--'")))
        }
    }
}
