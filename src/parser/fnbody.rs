//! Bodies of functions: statements (`val`, `if`, `return`) lowered to one expression.
//!
//! A `val` is substituted into the statements after it, and an `if` whose branches both return becomes a
//! conditional expression, so a function of statements is an ordinary pure function (see `elab::fold::inline`).

use super::Parser;
use std::collections::HashMap;

use crate::ast::{Expr, Item, ItemKind, Stmt};
use crate::error::{Error, Result};
use crate::lexer::Kind;
use crate::pipeline::rewrite::map_expr;

/// A statement of a function body, before lowering.
#[derive(Debug, Clone)]
pub enum FnStmt {
    Val(String, Expr),
    Return(Expr),
    If(Expr, Vec<FnStmt>, Vec<FnStmt>),
}

/// `e` with the names bound by `env` replaced; later bindings shadow earlier ones.
fn subst(e: &Expr, env: &[(String, Expr)]) -> Expr {
    map_expr(e, &mut |x| {
        Ok(match x {
            Expr::Ident(n, _) => env.iter().rev().find(|(v, _)| v == n).map(|(_, value)| value.clone()),
            _ => None,
        })
    })
    .unwrap_or_else(|_| e.clone())
}

/// Lowers statements to the value they return. `env` is the bindings in scope.
pub fn lower(stmts: &[FnStmt], env: &mut Vec<(String, Expr)>, pos: (usize, usize), file: &str) -> Result<Option<Expr>> {
    for (k, s) in stmts.iter().enumerate() {
        match s {
            FnStmt::Val(name, e) => {
                let value = subst(e, env);
                env.push((name.clone(), value));
            }
            FnStmt::Return(e) => return Ok(Some(subst(e, env))),
            FnStmt::If(cond, then, els) => {
                let cond = subst(cond, env);
                // the statements after an `if` without `else` are its else branch
                let mut otherwise = els.clone();
                if otherwise.is_empty() {
                    otherwise.extend_from_slice(&stmts[k + 1..]);
                }
                let mut env_then = env.clone();
                let mut env_else = env.clone();
                let a = lower(then, &mut env_then, pos, file)?;
                let b = lower(&otherwise, &mut env_else, pos, file)?;
                return match (a, b) {
                    (Some(a), Some(b)) => Ok(Some(Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b)))),
                    _ => Err(Error::new("a conditional in a function must return a value on both branches", file, pos.0, pos.1)),
                };
            }
        }
    }
    Ok(None)
}

impl Parser {
    /// A function body in braces, or up to `endfunction` when `closer` is that.
    pub(super) fn parse_fn_body(&mut self, closer: &str) -> Result<Expr> {
        let pos = (self.tok().line, self.tok().col);
        let stmts = self.parse_fn_stmts(closer)?;
        let mut env = Vec::new();
        match lower(&stmts, &mut env, pos, &self.file)? {
            Some(e) => Ok(e),
            None => Err(Error::new("a function must return a value", &self.file, pos.0, pos.1)),
        }
    }

    /// Statements up to the closer (`}` or `endfunction`), which is consumed; a braced body opens with `{`.
    fn parse_fn_stmts(&mut self, closer: &str) -> Result<Vec<FnStmt>> {
        if closer == "}" {
            self.expect("{")?;
        }
        let mut stmts = Vec::new();
        while !self.at(closer) {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail(format!("'{closer}' missing")));
            }
            stmts.push(self.parse_fn_stmt()?);
        }
        self.expect(closer)?;
        Ok(stmts)
    }

    fn parse_fn_stmt(&mut self) -> Result<FnStmt> {
        if self.accept("return") {
            let e = self.parse_expr(0)?;
            self.expect(";")?;
            return Ok(FnStmt::Return(e));
        }
        if self.accept("val") {
            let name = self.ident()?;
            self.expect("=")?;
            let e = self.parse_expr(0)?;
            self.expect(";")?;
            return Ok(FnStmt::Val(name, e));
        }
        if self.accept("if") {
            self.expect("(")?;
            let cond = self.parse_expr(0)?;
            self.expect(")")?;
            let then = self.parse_fn_branch()?;
            let els = if self.accept("else") { self.parse_fn_branch()? } else { Vec::new() };
            return Ok(FnStmt::If(cond, then, els));
        }
        Err(self.fail(format!("a function statement is val, if or return, not '{}'", self.tok().text)))
    }

    /// One branch of an `if`: a braced block or a single statement.
    fn parse_fn_branch(&mut self) -> Result<Vec<FnStmt>> {
        if self.accept("{") {
            let mut stmts = Vec::new();
            while !self.accept("}") {
                if self.tok().kind == Kind::Eof {
                    return Err(self.fail("'}' missing"));
                }
                stmts.push(self.parse_fn_stmt()?);
            }
            return Ok(stmts);
        }
        Ok(vec![self.parse_fn_stmt()?])
    }
}

/// A procedural function: its parameters and statements.
type Proc = (Vec<String>, Vec<Stmt>);

/// The most nested procedural calls expanded (a procedure that calls itself stops here, as an error at elaboration).
const MAX_CALL_DEPTH: usize = 16;

/// Removes the procedural functions of a module and expands their calls in the statements of its items.
pub(super) fn expand_procs(items: Vec<Item>) -> Vec<Item> {
    let mut procs: HashMap<String, Proc> = HashMap::new();
    let mut rest = Vec::new();
    for it in items {
        let pos = it.pos;
        match it.kind {
            // the definition stays, so that a use of it as a value can be named (its calls are expanded below)
            ItemKind::Proc { name, params, body } => {
                procs.insert(name.clone(), (params.clone(), body.clone()));
                rest.push(Item { pos, kind: ItemKind::Proc { name, params, body } });
            }
            kind => rest.push(Item { pos, kind }),
        }
    }
    if !procs.is_empty() {
        for it in &mut rest {
            expand_item(&mut it.kind, &procs, 0);
        }
    }
    // a pure function in a stage is compile-time: it belongs to the module, where its calls find it
    let mut hoisted = Vec::new();
    for it in &mut rest {
        if let ItemKind::Stage { items, .. } = &mut it.kind {
            let (fns, others): (Vec<Item>, Vec<Item>) = std::mem::take(items).into_iter().partition(|x| matches!(x.kind, ItemKind::Fn { .. }));
            *items = others;
            hoisted.extend(fns);
        }
    }
    rest.extend(hoisted);
    rest
}

fn expand_item(kind: &mut ItemKind, procs: &HashMap<String, Proc>, depth: usize) {
    match kind {
        ItemKind::Always { body, .. } | ItemKind::AlwaysComb(body) | ItemKind::Initial(body) => expand_stmt(body, procs, depth),
        ItemKind::GenIf { then, els, .. } => {
            for it in then.iter_mut().chain(els.iter_mut().flatten()) {
                expand_item(&mut it.kind, procs, depth);
            }
        }
        ItemKind::GenFor { body, .. } => {
            for it in body {
                expand_item(&mut it.kind, procs, depth);
            }
        }
        ItemKind::Stage { items, .. } => {
            for it in items {
                expand_item(&mut it.kind, procs, depth);
            }
        }
        _ => {}
    }
}

/// Replaces a statement call of a procedure by its statements, with the arguments for the parameters.
fn expand_stmt(s: &mut Stmt, procs: &HashMap<String, Proc>, depth: usize) {
    if let Stmt::ExprStmt(Expr::Call(f, args)) = s {
        if let Expr::Ident(name, _) = &**f {
            if let Some((params, body)) = procs.get(name) {
                if depth < MAX_CALL_DEPTH && params.len() == args.len() {
                    let bound: Vec<(String, Expr)> = params.iter().cloned().zip(args.iter().cloned()).collect();
                    let mut stmts: Vec<Stmt> = body.iter().map(|b| subst_stmt(b, &bound)).collect();
                    for x in &mut stmts {
                        expand_stmt(x, procs, depth + 1);
                    }
                    *s = Stmt::Block(stmts);
                    return;
                }
            }
        }
    }
    match s {
        Stmt::Block(stmts) => {
            for x in stmts {
                expand_stmt(x, procs, depth);
            }
        }
        Stmt::If { then, els, .. } => {
            expand_stmt(then, procs, depth);
            if let Some(e) = els {
                expand_stmt(e, procs, depth);
            }
        }
        Stmt::For { body, .. } => expand_stmt(body, procs, depth),
        Stmt::Case { arms, .. } => {
            for (_, body) in arms {
                expand_stmt(body, procs, depth);
            }
        }
        _ => {}
    }
}

/// `s` with the parameters replaced by their arguments.
fn subst_stmt(s: &Stmt, bound: &[(String, Expr)]) -> Stmt {
    crate::pipeline::rewrite::map_stmt(s, &mut |x| {
        Ok(match x {
            Expr::Ident(n, _) => bound.iter().find(|(p, _)| p == n).map(|(_, a)| a.clone()),
            _ => None,
        })
    })
    .unwrap_or_else(|_| s.clone())
}
