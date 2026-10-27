//! Pipeline-level arrays are scalarized: `T v [N]` becomes `v_0 ... v_{N-1}`, and `v[k]` with a constant `k`
//! becomes `v_k`. Each element is then owned by the stage that drives it, so stages can chain through `v[i]`.

use std::collections::{BTreeSet, HashMap};

use super::rewrite::{map_expr, map_stmt, Failure};
use crate::ast::{Decl, Declarator, Expr, Item, ItemKind, Module};
use crate::elab::int_of;
use crate::error::{Error, Result};

/// The names a stage drives (assigned in a stage's statements or assigns, at any depth).
fn driven_in_stages(items: &[Item], in_stage: bool, out: &mut BTreeSet<String>) {
    for it in items {
        match &it.kind {
            ItemKind::Stage { items, .. } => driven_in_stages(items, true, out),
            ItemKind::Always { body, .. } | ItemKind::AlwaysComb(body) if in_stage => crate::elab::assigned_names(body, out),
            ItemKind::ContAssign { lhs, .. } if in_stage => {
                let mut base = lhs;
                while let Expr::Index(b, _) | Expr::Slice(b, _, _, _) | Expr::Member(b, _, _) = base {
                    base = b;
                }
                if let Expr::Ident(n, _) = base {
                    out.insert(n.clone());
                }
            }
            ItemKind::GenIf { then, els, .. } => {
                driven_in_stages(then, in_stage, out);
                if let Some(e) = els {
                    driven_in_stages(e, in_stage, out);
                }
            }
            ItemKind::GenFor { body, .. } => driven_in_stages(body, in_stage, out),
            _ => {}
        }
    }
}

/// The module with its pipeline-level arrays scalarized. An array is split per element when a stage drives it (so
/// each element belongs to one stage); the others (a register file written outside the stages) stay arrays.
pub fn scalarize(m: &Module, file: &str) -> Result<Module> {
    let mut in_stages = BTreeSet::new();
    driven_in_stages(&m.items, false, &mut in_stages);
    let mut sizes: HashMap<String, i64> = HashMap::new();
    let mut items = Vec::new();
    for it in &m.items {
        match &it.kind {
            ItemKind::Decl(d) if !d.is_const && d.decls.iter().any(|x| !x.dims.is_empty() && in_stages.contains(&x.name)) => {
                let mut elements = Vec::new();
                for x in &d.decls {
                    let [dim] = x.dims.as_slice() else {
                        return Err(Error::new("a pipeline-level array takes one constant size", file, it.pos.0, it.pos.1));
                    };
                    let n = int_of(dim).filter(|n| *n > 0).ok_or_else(|| {
                        Error::new(format!("the size of array '{}' must be a positive constant", x.name), file, it.pos.0, it.pos.1)
                    })?;
                    sizes.insert(x.name.clone(), n);
                    for k in 0..n {
                        elements.push(Declarator { name: format!("{}_{k}", x.name), dims: Vec::new(), init: None });
                    }
                }
                items.push(Item { pos: it.pos, kind: ItemKind::Decl(Decl { dtype: d.dtype.clone(), decls: elements, is_const: d.is_const, is_async: d.is_async }) });
            }
            _ => items.push(it.clone()),
        }
    }
    if sizes.is_empty() {
        return Ok(m.clone());
    }
    let rewrite = |e: &Expr| -> std::result::Result<Option<Expr>, Failure> {
        let Expr::Index(base, idx) = e else { return Ok(None) };
        let Expr::Ident(name, _) = &**base else { return Ok(None) };
        let Some(&n) = sizes.get(name) else { return Ok(None) };
        match int_of(idx) {
            Some(k) if (0..n).contains(&k) => Ok(Some(Expr::Ident(format!("{name}_{k}"), (0, 0)))),
            Some(k) => Err(((0, 0), format!("index {k} out of range (size {n})"))),
            _ => Err(((0, 0), format!("the index of array '{name}' must be a constant in range (the array is scalarized)"))),
        }
    };
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        out.push(rewrite_item(&it, &rewrite).map_err(|(at, msg)| {
            let pos = if at.0 == 0 { it.pos } else { at };
            Error::new(msg, file, pos.0, pos.1)
        })?);
    }
    Ok(Module { items: out, ..m.clone() })
}

/// Applies a resolver to every expression of an item (see `rewrite::map_expr`).
pub(crate) fn rewrite_item(it: &Item, f: &dyn Fn(&Expr) -> std::result::Result<Option<Expr>, Failure>) -> std::result::Result<Item, Failure> {
    let mut res = |e: &Expr| map_expr(e, &mut |x| f(x));
    let kind = match &it.kind {
        ItemKind::Decl(d) => ItemKind::Decl(Decl {
            dtype: d.dtype.clone(),
            decls: d
                .decls
                .iter()
                .map(|x| Ok(Declarator { name: x.name.clone(), dims: x.dims.iter().map(&mut res).collect::<std::result::Result<_, _>>()?, init: match &x.init { Some(e) => Some(res(e)?), None => None } }))
                .collect::<std::result::Result<_, Failure>>()?,
            is_const: d.is_const,
            is_async: d.is_async,
        }),
        ItemKind::ContAssign { lhs, rhs } => ItemKind::ContAssign { lhs: res(lhs)?, rhs: res(rhs)? },
        ItemKind::AlwaysComb(s) => ItemKind::AlwaysComb(map_stmt(s, &mut |x| f(x))?),
        ItemKind::Always { sens, body, keyword } => ItemKind::Always {
            sens: match sens {
                crate::ast::Sens::Edges(edges) => crate::ast::Sens::Edges(edges.iter().map(|(e, x)| Ok((e.clone(), res(x)?))).collect::<std::result::Result<_, Failure>>()?),
                crate::ast::Sens::Comb => crate::ast::Sens::Comb,
            },
            body: map_stmt(body, &mut |x| f(x))?,
            keyword: keyword.clone(),
        },
        ItemKind::Ctrl { kind, cond } => ItemKind::Ctrl { kind: kind.clone(), cond: res(cond)? },
        ItemKind::Instance(i) => ItemKind::Instance(crate::ast::Instance {
            conns: i
                .conns
                .iter()
                .map(|c| {
                    Ok(crate::ast::Conn {
                        port: c.port.clone(),
                        expr: match &c.expr {
                            Some(e) => Some(res(e)?),
                            None => None,
                        },
                    })
                })
                .collect::<std::result::Result<_, Failure>>()?,
            ..i.clone()
        }),
        ItemKind::Stage { name, items } => ItemKind::Stage {
            name: name.clone(),
            items: items.iter().map(|x| rewrite_item(x, f)).collect::<std::result::Result<_, Failure>>()?,
        },
        other => other.clone(),
    };
    Ok(Item { pos: it.pos, kind })
}
