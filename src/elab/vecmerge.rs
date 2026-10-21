//! Vectors of bundles, laid out as the reference does: each field of the bundle is one signal of the whole vector
//! (`port_valid [1:0]`, `port_address [2]`), and `port[k].field` is element `k` of it.
//!
//! Elaboration makes one signal per element first (`port_0_valid`, `port_1_valid`, ...), which is what the connections
//! and the bundle references are written with. This pass runs at the end of a module: it merges the elements of each
//! field into one signal, and rewrites every reference to an element as an index of that signal.

use std::collections::HashMap;

use crate::ast::{Arg, Declarator, Decl, DType, Expr, Item, ItemKind, Module, Port, Stmt};
use crate::pipeline::rewrite::{map_expr, map_stmt};

/// Element signal -> (the merged signal, the element's index).
pub type Elements = HashMap<String, (String, usize)>;

/// Merges the element signals of the vectors of bundles `groups` (name and count) of `module`, and returns the module
/// together with the element table (the tests and the connections of the instances refer to elements through it).
pub fn merge(module: Module, groups: &[(String, usize)]) -> (Module, Elements) {
    if groups.is_empty() {
        return (module, Elements::new());
    }
    let mut elements = Elements::new();
    // the fields of each vector: the signals named `base_0_field` (the element 0 of each)
    let mut fields: Vec<(String, usize, String)> = Vec::new();
    for (base, n) in groups {
        let prefix = format!("{base}_0_");
        let names = module.ports.iter().map(|p| p.name.clone()).chain(declared(&module.items));
        let mut leaves: Vec<String> = Vec::new();
        for leaf in names.filter_map(|name| name.strip_prefix(&prefix).map(str::to_string)) {
            if !leaves.contains(&leaf) {
                leaves.push(leaf);
            }
        }
        for leaf in leaves {
            for k in 0..*n {
                elements.insert(format!("{base}_{k}_{leaf}"), (format!("{base}_{leaf}"), k));
            }
            fields.push((base.clone(), *n, leaf));
        }
    }
    if fields.is_empty() {
        return (module, elements);
    }
    let mut module = module;
    module.ports = merge_ports(&module.ports, &elements, &fields);
    module.items = merge_items(&module.items, &elements, &fields);
    let rename = |e: &Expr| collapse(&rename_expr(e, &elements));
    module.items = map_items(&module.items, &rename);
    // the protocol checks are written as text: their element names are renamed as the expressions are
    module.items = module
        .items
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Raw(lines) => Item { pos: item.pos, kind: ItemKind::Raw(lines.iter().map(|l| rename_text(l, &elements)).collect()) },
            _ => item.clone(),
        })
        .collect();
    for p in &mut module.ports {
        if let Some(d) = &p.default {
            p.default = Some(rename(d));
        }
    }
    (module, elements)
}

/// The signals a module's declarations name.
fn declared(items: &[Item]) -> impl Iterator<Item = String> + '_ {
    items.iter().flat_map(|item| match &item.kind {
        ItemKind::Decl(d) => d.decls.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
        _ => Vec::new(),
    })
}

/// The width of a field is one bit when its element type is a plain `logic` with no dimensions.
fn one_bit(dtype: &DType, dims: &[Expr]) -> bool {
    dims.is_empty() && dtype.packed.is_empty() && (dtype.name.is_empty() || dtype.name == "logic")
}

/// The merged type of a field: a packed vector of the elements for one bit, else an unpacked array of them.
fn merged_type(dtype: &DType, dims: &[Expr], n: usize) -> (DType, Vec<Expr>) {
    if one_bit(dtype, dims) {
        let packed = vec![(Expr::Num((n - 1).to_string()), Expr::Num("0".into()))];
        (DType { packed, ..dtype.clone() }, Vec::new())
    } else {
        let mut all = vec![Expr::Num(n.to_string())];
        all.extend(dims.iter().cloned());
        (dtype.clone(), all)
    }
}

fn merge_ports(ports: &[Port], elements: &Elements, fields: &[(String, usize, String)]) -> Vec<Port> {
    let mut out = Vec::new();
    for p in ports {
        match elements.get(&p.name) {
            None => out.push(p.clone()),
            Some((merged, 0)) => {
                let (_, n, _) = fields.iter().find(|(base, _, leaf)| format!("{base}_{leaf}") == *merged).cloned().unwrap_or_default();
                let (dtype, dims) = merged_type(&p.dtype, &p.dims, n);
                // a default is the same on every element: `'0` or `'1` on a vector of bits
                let default = if one_bit(&p.dtype, &p.dims) { p.default.as_ref().and_then(|d| match d {
                    Expr::Num(v) if v == "0" => Some(Expr::Num("'0".into())),
                    Expr::Num(v) if v == "1" => Some(Expr::Num("'1".into())),
                    _ => None,
                }) } else { None };
                out.push(Port { name: merged.clone(), dtype, dims, default, ..p.clone() });
            }
            Some(_) => {}
        }
    }
    out
}

/// The declarations of the elements: each field is declared once, where its element 0 was.
fn merge_items(items: &[Item], elements: &Elements, fields: &[(String, usize, String)]) -> Vec<Item> {
    let mut out = Vec::new();
    let mut done: Vec<String> = Vec::new();
    for item in items {
        let ItemKind::Decl(d) = &item.kind else {
            out.push(item.clone());
            continue;
        };
        let mut kept = Vec::new();
        for x in &d.decls {
            match elements.get(&x.name) {
                None => kept.push(x.clone()),
                Some((merged, 0)) if !done.contains(merged) => {
                    done.push(merged.clone());
                    let (_, n, _) = fields.iter().find(|(base, _, leaf)| format!("{base}_{leaf}") == *merged).cloned().unwrap_or_default();
                    let (dtype, dims) = merged_type(&d.dtype, &x.dims, n);
                    let decl = Decl { dtype, decls: vec![Declarator { name: merged.clone(), dims: Vec::new(), init: None }], is_const: d.is_const, is_async: d.is_async };
                    let decl = Decl { decls: vec![Declarator { dims, ..decl.decls[0].clone() }], ..decl };
                    out.push(Item { pos: item.pos, kind: ItemKind::Decl(decl) });
                }
                Some(_) => {}
            }
        }
        if !kept.is_empty() {
            out.push(Item { pos: item.pos, kind: ItemKind::Decl(Decl { decls: kept, ..d.clone() }) });
        }
    }
    out
}

/// An element reference `name` as the field's index: `port_valid[1]`.
fn rename_expr(e: &Expr, elements: &Elements) -> Expr {
    map_expr(e, &mut |x| {
        Ok(match x {
            Expr::Ident(name, pos) => elements.get(name).map(|(merged, k)| {
                Expr::Index(Box::new(Expr::Ident(merged.clone(), *pos)), Box::new(Expr::Num(k.to_string())))
            }),
            _ => None,
        })
    })
    .unwrap_or_else(|_| e.clone())
}

/// `{v[n-1], ..., v[0]}` and `'{v[0], ..., v[n-1]}` are `v`: the whole vector, as one name.
fn collapse(e: &Expr) -> Expr {
    map_expr(e, &mut |x| {
        Ok(match x {
            Expr::Concat(parts) => whole_vector(parts.iter().rev()).map(|name| Expr::Ident(name, (0, 0))),
            Expr::Pattern(items) => whole_vector(items.iter()).map(|name| Expr::Ident(name, (0, 0))),
            _ => None,
        })
    })
    .unwrap_or_else(|_| e.clone())
}

/// The name of the vector when the items are its elements `0, 1, ...` in order.
fn whole_vector<'a>(items: impl Iterator<Item = &'a Expr>) -> Option<String> {
    let mut name: Option<String> = None;
    for (k, item) in items.enumerate() {
        let Expr::Index(base, idx) = item else { return None };
        let Expr::Ident(vec, _) = &**base else { return None };
        let Expr::Num(i) = &**idx else { return None };
        if i.parse::<usize>().ok()? != k || name.as_ref().is_some_and(|n| n != vec) {
            return None;
        }
        name = Some(vec.clone());
    }
    name
}

/// The items with a rewrite applied to each of their expressions and statements.
fn map_items(items: &[Item], f: &dyn Fn(&Expr) -> Expr) -> Vec<Item> {
    items.iter().map(|item| Item { pos: item.pos, kind: map_kind(&item.kind, f) }).collect()
}

fn map_kind(kind: &ItemKind, f: &dyn Fn(&Expr) -> Expr) -> ItemKind {
    let stmt = |s: &Stmt| map_stmt_with(s, f);
    match kind {
        ItemKind::ContAssign { lhs, rhs } => ItemKind::ContAssign { lhs: f(lhs), rhs: f(rhs) },
        ItemKind::AlwaysComb(body) => ItemKind::AlwaysComb(stmt(body)),
        ItemKind::Always { sens, body, keyword } => ItemKind::Always { sens: sens.clone(), body: stmt(body), keyword: keyword.clone() },
        ItemKind::Initial(body) => ItemKind::Initial(stmt(body)),
        ItemKind::Decl(d) => ItemKind::Decl(Decl {
            decls: d
                .decls
                .iter()
                .map(|x| Declarator { name: x.name.clone(), dims: x.dims.iter().map(f).collect(), init: x.init.as_ref().map(f) })
                .collect(),
            ..d.clone()
        }),
        ItemKind::Instance(i) => ItemKind::Instance(crate::ast::Instance {
            conns: i.conns.iter().map(|c| crate::ast::Conn { port: c.port.clone(), expr: c.expr.as_ref().map(f) }).collect(),
            targs: i.targs.iter().map(|(n, a)| (n.clone(), match a {
                Arg::Expr(e) => Arg::Expr(f(e)),
                other => other.clone(),
            })).collect(),
            ..i.clone()
        }),
        ItemKind::TieOff(e) => ItemKind::TieOff(f(e)),
        ItemKind::Val { name, expr } => ItemKind::Val { name: name.clone(), expr: f(expr) },
        ItemKind::Stage { name, items } => ItemKind::Stage { name: name.clone(), items: map_items(items, f) },
        other => other.clone(),
    }
}

/// A statement with `f` applied to its expressions.
fn map_stmt_with(s: &Stmt, f: &dyn Fn(&Expr) -> Expr) -> Stmt {
    map_stmt(s, &mut |x| Ok(Some(f(x)))).unwrap_or_else(|_| s.clone())
}

/// Text with each identifier that is an element name replaced by the element's index of the merged signal.
fn rename_text(text: &str, elements: &Elements) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        match elements.get(word.as_str()) {
            Some((merged, k)) => out.push_str(&format!("{merged}[{k}]")),
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}
