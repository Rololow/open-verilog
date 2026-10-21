//! Rewriting of expressions and statements: a resolver sees each node first and may replace it.

use crate::ast::{Expr, ForHeader, Pos, SetItem, Stmt};

/// A failed rewrite: the position of the offending name (or `(0, 0)` when unknown) and the message.
pub type Failure = (Pos, String);

/// Returns `Some(replacement)` to replace a node, `None` to visit its children.
pub type Resolver<'r> = dyn FnMut(&Expr) -> Result<Option<Expr>, Failure> + 'r;

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

pub fn map_expr(e: &Expr, f: &mut Resolver) -> Result<Expr, Failure> {
    if let Some(replacement) = f(e)? {
        return Ok(replacement);
    }
    Ok(match e {
        Expr::Num(_) | Expr::Ident(..) | Expr::OpRef(_) | Expr::Type(_) => e.clone(),
        Expr::Unary(op, a) => Expr::Unary(op.clone(), boxed(map_expr(a, f)?)),
        Expr::Binary(op, l, r, pos) => Expr::Binary(op.clone(), boxed(map_expr(l, f)?), boxed(map_expr(r, f)?), *pos),
        Expr::Ternary(c, a, b) => Expr::Ternary(boxed(map_expr(c, f)?), boxed(map_expr(a, f)?), boxed(map_expr(b, f)?)),
        Expr::Index(base, idx) => Expr::Index(boxed(map_expr(base, f)?), boxed(map_expr(idx, f)?)),
        Expr::Slice(base, msb, lsb, op) => Expr::Slice(boxed(map_expr(base, f)?), boxed(map_expr(msb, f)?), boxed(map_expr(lsb, f)?), op.clone()),
        Expr::Member(base, name, pos) => Expr::Member(boxed(map_expr(base, f)?), name.clone(), *pos),
        Expr::Call(func, args) => Expr::Call(boxed(map_expr(func, f)?), map_all(args, f)?),
        Expr::Concat(parts) => Expr::Concat(map_all(parts, f)?),
        Expr::Repl(count, parts) => Expr::Repl(boxed(map_expr(count, f)?), map_all(parts, f)?),
        Expr::Cast(width, arg) => Expr::Cast(boxed(map_expr(width, f)?), boxed(map_expr(arg, f)?)),
        Expr::Lambda(params, body) => Expr::Lambda(params.clone(), boxed(map_expr(body, f)?)),
        Expr::List(items) => Expr::List(map_all(items, f)?),
        Expr::InSet(x, set) => Expr::InSet(boxed(map_expr(x, f)?), map_set(set, f)?),
        Expr::Pattern(items) => Expr::Pattern(map_all(items, f)?),
        Expr::Keyed(k, v) => Expr::Keyed(k.clone(), boxed(map_expr(v, f)?)),
        Expr::Match(subject, arms) => {
            let mut out = Vec::with_capacity(arms.len());
            for (labels, value) in arms {
                let labels = match labels {
                    Some(set) => Some(map_set(set, f)?),
                    None => None,
                };
                out.push((labels, map_expr(value, f)?));
            }
            Expr::Match(boxed(map_expr(subject, f)?), out)
        }
        Expr::With(base, overrides, pos) => {
            let mut out = Vec::with_capacity(overrides.len());
            for (n, v) in overrides {
                out.push((n.clone(), map_expr(v, f)?));
            }
            Expr::With(boxed(map_expr(base, f)?), out, *pos)
        }
    })
}

fn map_all(items: &[Expr], f: &mut Resolver) -> Result<Vec<Expr>, Failure> {
    let mut out = Vec::with_capacity(items.len());
    for x in items {
        out.push(map_expr(x, f)?);
    }
    Ok(out)
}

fn map_set(set: &[SetItem], f: &mut Resolver) -> Result<Vec<SetItem>, Failure> {
    let mut out = Vec::with_capacity(set.len());
    for s in set {
        out.push(SetItem {
            lo: map_expr(&s.lo, f)?,
            hi: match &s.hi {
                Some(h) => Some(map_expr(h, f)?),
                None => None,
            },
        });
    }
    Ok(out)
}

pub fn map_stmt(s: &Stmt, f: &mut Resolver) -> Result<Stmt, Failure> {
    Ok(match s {
        Stmt::Block(stmts) => Stmt::Block(stmts.iter().map(|x| map_stmt(x, f)).collect::<Result<_, _>>()?),
        Stmt::Assign { lhs, rhs, nonblocking } => Stmt::Assign { lhs: map_expr(lhs, f)?, rhs: map_expr(rhs, f)?, nonblocking: *nonblocking },
        Stmt::If { cond, then, els } => Stmt::If {
            cond: map_expr(cond, f)?,
            then: Box::new(map_stmt(then, f)?),
            els: match els {
                Some(e) => Some(Box::new(map_stmt(e, f)?)),
                None => None,
            },
        },
        Stmt::For { header, body } => Stmt::For { header: map_header(header, f)?, body: Box::new(map_stmt(body, f)?) },
        Stmt::Case { subject, kind, arms } => {
            let mut out = Vec::with_capacity(arms.len());
            for (labels, body) in arms {
                let labels = match labels {
                    Some(ls) => Some(map_all(ls, f)?),
                    None => None,
                };
                out.push((labels, map_stmt(body, f)?));
            }
            Stmt::Case { subject: map_expr(subject, f)?, kind: kind.clone(), arms: out }
        }
        Stmt::Decl(d) => Stmt::Decl(d.clone()),
        Stmt::Val(name, e) => Stmt::Val(name.clone(), map_expr(e, f)?),
        Stmt::Return(e) => Stmt::Return(map_expr(e, f)?),
        Stmt::Goto(e) => Stmt::Goto(map_expr(e, f)?),
        Stmt::ExprStmt(e) => Stmt::ExprStmt(map_expr(e, f)?),
    })
}

fn map_header(h: &ForHeader, f: &mut Resolver) -> Result<ForHeader, Failure> {
    Ok(ForHeader {
        var: h.var.clone(),
        lo: map_expr(&h.lo, f)?,
        bound: match &h.bound {
            Some(b) => Some(map_expr(b, f)?),
            None => None,
        },
        cmp: h.cmp.clone(),
        step: match &h.step {
            Some(s) => Some(map_expr(s, f)?),
            None => None,
        },
        pos: h.pos,
    })
}
