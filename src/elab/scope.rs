//! Names local to a generate loop body: every unrolled iteration declares its own copy, named after the iteration
//! (`va` becomes `va_0`, `va_1`, ...), as the reference does.

use std::collections::HashMap;

use crate::ast::{Conn, Declarator, Expr, Item, ItemKind, Sens};
use crate::pipeline::rewrite::{map_expr, map_stmt};

/// The names a loop body declares (signals and values), including those of its conditional generate blocks. The
/// loops nested in it rename their own.
pub fn declared_names(items: &[Item]) -> Vec<String> {
    let mut out = Vec::new();
    for it in items {
        match &it.kind {
            ItemKind::Decl(d) => out.extend(d.decls.iter().map(|x| x.name.clone())),
            ItemKind::Val { name, .. } => out.push(name.clone()),
            ItemKind::GenIf { then, els, .. } => {
                out.extend(declared_names(then));
                if let Some(e) = els {
                    out.extend(declared_names(e));
                }
            }
            _ => {}
        }
    }
    out
}

/// `e` with the names of `map` replaced.
fn rename_expr(e: &Expr, map: &HashMap<String, String>) -> Expr {
    map_expr(e, &mut |x| {
        Ok(match x {
            Expr::Ident(n, pos) => map.get(n).map(|new| Expr::Ident(new.clone(), *pos)),
            _ => None,
        })
    })
    .unwrap_or_else(|_| e.clone())
}

/// The items with the names of `map` renamed, where they are declared and where they are used.
pub fn rename_items(items: &[Item], map: &HashMap<String, String>) -> Vec<Item> {
    items.iter().map(|it| Item { pos: it.pos, kind: rename_kind(&it.kind, map) }).collect()
}

fn rename_kind(kind: &ItemKind, map: &HashMap<String, String>) -> ItemKind {
    let ex = |e: &Expr| rename_expr(e, map);
    let stmt = |s: &crate::ast::Stmt| {
        map_stmt(s, &mut |x| {
            Ok(match x {
                Expr::Ident(n, pos) => map.get(n).map(|new| Expr::Ident(new.clone(), *pos)),
                _ => None,
            })
        })
        .unwrap_or_else(|_| s.clone())
    };
    match kind {
        ItemKind::Decl(d) => ItemKind::Decl(crate::ast::Decl {
            dtype: d.dtype.clone(),
            decls: d
                .decls
                .iter()
                .map(|x| Declarator {
                    name: map.get(&x.name).cloned().unwrap_or_else(|| x.name.clone()),
                    dims: x.dims.iter().map(ex).collect(),
                    init: x.init.as_ref().map(ex),
                })
                .collect(),
            is_const: d.is_const,
            is_async: d.is_async,
        }),
        ItemKind::Val { name, expr } => ItemKind::Val { name: map.get(name).cloned().unwrap_or_else(|| name.clone()), expr: ex(expr) },
        ItemKind::ContAssign { lhs, rhs } => ItemKind::ContAssign { lhs: ex(lhs), rhs: ex(rhs) },
        ItemKind::AlwaysComb(s) => ItemKind::AlwaysComb(stmt(s)),
        ItemKind::Initial(s) => ItemKind::Initial(stmt(s)),
        ItemKind::Always { sens, body, keyword } => ItemKind::Always {
            sens: match sens {
                Sens::Edges(edges) => Sens::Edges(edges.iter().map(|(e, x)| (e.clone(), ex(x))).collect()),
                Sens::Comb => Sens::Comb,
            },
            body: stmt(body),
            keyword: keyword.clone(),
        },
        ItemKind::Instance(i) => ItemKind::Instance(crate::ast::Instance {
            conns: i.conns.iter().map(|c| Conn { port: c.port.clone(), expr: c.expr.as_ref().map(ex) }).collect(),
            // the parameters of the instance may name a local value: `Lane<kind> l(...)`
            targs: i
                .targs
                .iter()
                .map(|(n, a)| (n.clone(), match a {
                    crate::ast::Arg::Expr(e) => crate::ast::Arg::Expr(ex(e)),
                    other => other.clone(),
                }))
                .collect(),
            ..i.clone()
        }),
        ItemKind::GenIf { cond, then, els } => ItemKind::GenIf {
            cond: ex(cond),
            then: rename_items(then, map),
            els: els.as_ref().map(|e| rename_items(e, map)),
        },
        ItemKind::GenFor { header, body } => {
            let mut header = header.clone();
            header.lo = ex(&header.lo);
            header.bound = header.bound.as_ref().map(ex);
            header.step = header.step.as_ref().map(ex);
            ItemKind::GenFor { header, body: rename_items(body, map) }
        }
        ItemKind::Stage { name, items } => ItemKind::Stage { name: name.clone(), items: rename_items(items, map) },
        other => other.clone(),
    }
}

/// Declarations inside the statements of a block are module-level signals in the output (a block-local variable
/// of an always block, unrolled loops included): `m` becomes `ov_l0_m`. Returns the statements without the
/// declarations (an initial value becomes an assignment), the new declarations, and renames the uses.
pub fn hoist_locals(body: &crate::ast::Stmt, counter: &mut usize) -> (crate::ast::Stmt, Vec<crate::ast::Decl>) {
    let mut decls = Vec::new();
    let out = hoist(body, &HashMap::new(), &mut decls, counter);
    (out, decls)
}

fn hoist(s: &crate::ast::Stmt, scope: &HashMap<String, String>, decls: &mut Vec<crate::ast::Decl>, counter: &mut usize) -> crate::ast::Stmt {
    use crate::ast::Stmt;
    let ex = |e: &Expr| rename_expr(e, scope);
    match s {
        Stmt::Block(stmts) => {
            let mut local = scope.clone();
            let mut out = Vec::new();
            for st in stmts {
                if let Stmt::Decl(d) = st {
                    for x in &d.decls {
                        let fresh = format!("ov_l{}_{}", *counter, x.name);
                        *counter += 1;
                        decls.push(crate::ast::Decl {
                            dtype: d.dtype.clone(),
                            decls: vec![Declarator { name: fresh.clone(), dims: x.dims.iter().map(|e| rename_expr(e, &local)).collect(), init: None }],
                            is_const: false,
                            is_async: false,
                        });
                        if let Some(init) = &x.init {
                            out.push(Stmt::Assign { lhs: Expr::Ident(fresh.clone(), (0, 0)), rhs: rename_expr(init, &local), nonblocking: false, pos: (0, 0) });
                        }
                        local.insert(x.name.clone(), fresh);
                    }
                } else {
                    out.push(hoist(st, &local, decls, counter));
                }
            }
            Stmt::Block(out)
        }
        Stmt::Assign { lhs, rhs, nonblocking, pos } => Stmt::Assign { lhs: ex(lhs), rhs: ex(rhs), nonblocking: *nonblocking, pos: *pos },
        Stmt::If { cond, then, els } => Stmt::If {
            cond: ex(cond),
            then: Box::new(hoist(then, scope, decls, counter)),
            els: els.as_ref().map(|e| Box::new(hoist(e, scope, decls, counter))),
        },
        Stmt::For { header, body } => {
            let mut header = header.clone();
            header.lo = ex(&header.lo);
            header.bound = header.bound.as_ref().map(ex);
            header.step = header.step.as_ref().map(ex);
            Stmt::For { header, body: Box::new(hoist(body, scope, decls, counter)) }
        }
        Stmt::Case { subject, kind, arms, pos } => Stmt::Case {
            subject: ex(subject),
            kind: kind.clone(),
            arms: arms
                .iter()
                .map(|(labels, body)| (labels.as_ref().map(|ls| ls.iter().map(ex).collect()), hoist(body, scope, decls, counter)))
                .collect(),
            pos: *pos,
        },
        Stmt::ExprStmt(e) => Stmt::ExprStmt(ex(e)),
        Stmt::Val(name, e) => Stmt::Val(name.clone(), ex(e)),
        Stmt::Return(e) => Stmt::Return(ex(e)),
        Stmt::Goto(e) => Stmt::Goto(ex(e)),
        Stmt::Decl(d) => Stmt::Decl(d.clone()),
    }
}
