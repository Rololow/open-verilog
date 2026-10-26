//! The checks of `ovc lint` on the elaborated modules: latches, drivers, instance connections and assignment style (the
//! Python checks of the same names). They are warnings, and they run only in the lint: a design that is compiled is not
//! checked this way.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ast::{Dir, Expr, Item, ItemKind, Module, Pos, Stmt};
use crate::domains::reads_of;
use crate::elab::fold::int_of;
use crate::elab::width::width_of;
use crate::elab::{expr_any, stmt_any_expr, Env};

/// The warnings of the checks, as the compiler writes them: `file:line:col: warning: message`.
pub fn check(modules: &[Module], envs: &HashMap<String, Env>) -> Vec<String> {
    // the items of the generate loops and conditions count as if they were written at the top, as they are unrolled
    // in the reference (an instance in a loop drives its outputs all the same)
    let flat: Vec<Module> = modules.iter().map(|m| Module { items: flatten(&m.items), ..m.clone() }).collect();
    let modules = &flat;
    let by_name: HashMap<&str, &Module> = modules.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut out = Vec::new();
    for m in modules {
        let env = envs.get(&m.name);
        check_latches(m, env, &mut out);
        check_drivers(m, &by_name, &mut out);
        check_connections(m, &by_name, &mut out);
        check_process_style(m, &mut out);
    }
    out
}

/// The items with those of the generate loops and conditions in place of the loop.
fn flatten(items: &[Item]) -> Vec<Item> {
    let mut out = Vec::new();
    for it in items {
        match &it.kind {
            ItemKind::GenFor { body, .. } | ItemKind::GenIf { then: body, .. } => out.extend(flatten(body)),
            _ => out.push(it.clone()),
        }
    }
    out
}

fn warn(out: &mut Vec<String>, m: &Module, pos: Pos, msg: String) {
    let file = if m.origin.is_empty() { "<lint>" } else { m.origin.as_str() };
    let text = format!("{file}:{}:{}: warning: {msg}", pos.0, pos.1);
    if !out.contains(&text) {
        out.push(text);
    }
}

/// The base name of an assignment target (`x`, `x[i]`, `x[a:b]`, `x.f`).
fn base(e: &Expr) -> Option<String> {
    let mut e = e;
    loop {
        match e {
            Expr::Index(b, _) | Expr::Slice(b, _, _, _) | Expr::Member(b, _, _) => e = b,
            Expr::Ident(n, _) => return Some(n.clone()),
            _ => return None,
        }
    }
}

/// The lvalues of an assignment target: `{a, b[3:0]}` is `a` and `b[3:0]`.
fn parts(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::Concat(v) => v.iter().flat_map(parts).collect(),
        _ => vec![e],
    }
}

fn text(e: &Expr) -> String {
    crate::emit::expr::expr(e, -1, false).unwrap_or_else(|_| "?".into())
}

/// Every assignment of a statement: its target and whether it is non-blocking.
fn assignments<'a>(s: &'a Stmt, out: &mut Vec<(&'a Expr, bool)>) {
    match s {
        Stmt::Assign { lhs, nonblocking, .. } => out.push((lhs, *nonblocking)),
        Stmt::Block(v) => v.iter().for_each(|x| assignments(x, out)),
        Stmt::If { then, els, .. } => {
            assignments(then, out);
            if let Some(e) = els {
                assignments(e, out);
            }
        }
        Stmt::Case { arms, .. } => arms.iter().for_each(|(_, b)| assignments(b, out)),
        Stmt::For { body, .. } => assignments(body, out),
        _ => {}
    }
}

/// The signals assigned on every path through a statement.
fn def_assigned(s: &Stmt, env: Option<&Env>) -> BTreeSet<String> {
    match s {
        Stmt::Assign { lhs, .. } => parts(lhs).into_iter().filter_map(base).collect(),
        Stmt::Block(v) => v.iter().flat_map(|x| def_assigned(x, env)).collect(),
        Stmt::If { then, els: Some(e), .. } => def_assigned(then, env).intersection(&def_assigned(e, env)).cloned().collect(),
        Stmt::If { els: None, .. } => BTreeSet::new(),
        Stmt::Case { subject, kind, arms } => {
            let has_default = arms.iter().any(|(labels, _)| labels.is_none());
            if !has_default && !case_covers_all(subject, kind, arms, env) {
                return BTreeSet::new();
            }
            let mut sets = arms.iter().map(|(_, b)| def_assigned(b, env));
            match sets.next() {
                Some(first) => sets.fold(first, |acc, s| acc.intersection(&s).cloned().collect()),
                None => BTreeSet::new(),
            }
        }
        _ => BTreeSet::new(),
    }
}

/// A `case` without a default whose constant labels list every value of its selector (of at most 8 bits).
fn case_covers_all(subject: &Expr, kind: &str, arms: &[(Option<Vec<Expr>>, Stmt)], env: Option<&Env>) -> bool {
    let Some(w) = env.and_then(|e| width_of(subject, e)) else { return false };
    if kind != "case" || w.width > 8 {
        return false;
    }
    let mut values = BTreeSet::new();
    for labels in arms.iter().filter_map(|(l, _)| l.as_ref()) {
        for l in labels {
            match int_of(l) {
                Some(v) => {
                    values.insert(v);
                }
                None => return false,
            }
        }
    }
    values == (0..(1i64 << w.width)).collect()
}

/// A combinational block must assign each signal on every path, or a latch is inferred.
fn check_latches(m: &Module, env: Option<&Env>, out: &mut Vec<String>) {
    for it in &m.items {
        let ItemKind::AlwaysComb(body) = &it.kind else { continue };
        let mut all = Vec::new();
        assignments(body, &mut all);
        let assigned: BTreeSet<String> = all.iter().flat_map(|(lhs, _)| parts(lhs)).filter_map(base).collect();
        let defined = def_assigned(body, env);
        for n in assigned.difference(&defined) {
            // a block-local temporary (a hoisted declaration) is not a signal of the design
            if n.starts_with("ov_l") && n[4..].contains('_') {
                continue;
            }
            warn(out, m, it.pos, format!("'{n}' is not assigned on every path of this always_comb (a latch would be inferred): add a default value"));
        }
    }
}

/// One driver of a signal: the item it is in, the target as written, and how the item drives it.
struct Driver {
    item: usize,
    target: String,
    kind: String,
    pos: Pos,
}

fn check_drivers(m: &Module, by_name: &HashMap<&str, &Module>, out: &mut Vec<String>) {
    let mut drivers: BTreeMap<String, Vec<Driver>> = BTreeMap::new();
    let mut maybe_driven: BTreeSet<String> = BTreeSet::new();
    let add = |drivers: &mut BTreeMap<String, Vec<Driver>>, lhs: &Expr, item: usize, kind: String, pos: Pos| {
        for part in parts(lhs) {
            if let Some(n) = base(part) {
                drivers.entry(n).or_default().push(Driver { item, target: text(part), kind: kind.clone(), pos });
            }
        }
    };
    for (idx, it) in m.items.iter().enumerate() {
        match &it.kind {
            ItemKind::ContAssign { lhs, .. } => add(&mut drivers, lhs, idx, "assign".into(), it.pos),
            ItemKind::AlwaysComb(body) => {
                let mut all = Vec::new();
                assignments(body, &mut all);
                for (lhs, _) in all {
                    add(&mut drivers, lhs, idx, "always_comb".into(), it.pos);
                }
            }
            ItemKind::Always { body, .. } => {
                let mut all = Vec::new();
                assignments(body, &mut all);
                for (lhs, _) in all {
                    add(&mut drivers, lhs, idx, "always_ff".into(), it.pos);
                }
            }
            ItemKind::Instance(i) => match by_name.get(i.module.as_str()) {
                Some(target) => {
                    for c in &i.conns {
                        let (Some(p), Some(e)) = (&c.port, &c.expr) else { continue };
                        if target.ports.iter().any(|q| &q.name == p && q.dir == Dir::Output) {
                            add(&mut drivers, e, idx, format!("instance '{}'", i.name), it.pos);
                        }
                    }
                }
                None => {
                    for c in &i.conns {
                        if let Some(e) = &c.expr {
                            maybe_driven.extend(parts(e).into_iter().filter_map(base));
                        }
                    }
                }
            },
            _ => {}
        }
    }
    let inputs: BTreeSet<&str> = m.ports.iter().filter(|p| p.dir == Dir::Input).map(|p| p.name.as_str()).collect();
    for (n, list) in &drivers {
        if inputs.contains(n.as_str()) {
            continue;
        }
        for (i, a) in list.iter().enumerate() {
            for b in &list[i + 1..] {
                if a.item != b.item && (a.target == b.target || a.target == *n || b.target == *n) {
                    warn(out, m, b.pos, format!("'{n}' is driven from several places ({} and {})", a.kind, b.kind));
                }
            }
        }
    }
    for p in m.ports.iter().filter(|p| p.dir == Dir::Output) {
        if !drivers.contains_key(&p.name) && !maybe_driven.contains(&p.name) {
            warn(out, m, m.pos, format!("output port '{}' of '{}' is never driven", p.name, m.name));
        }
    }
}

/// An input port of an instanced module that has no default must be connected (when every connection is named).
fn check_connections(m: &Module, by_name: &HashMap<&str, &Module>, out: &mut Vec<String>) {
    for it in &m.items {
        let ItemKind::Instance(i) = &it.kind else { continue };
        let Some(target) = by_name.get(i.module.as_str()) else { continue };
        if i.conns.iter().any(|c| c.port.is_none()) {
            continue;
        }
        let connected: BTreeSet<&str> = i.conns.iter().filter_map(|c| c.port.as_deref()).collect();
        for q in &target.ports {
            if q.dir == Dir::Input && q.default.is_none() && !q.name.starts_with("ov_") && !connected.contains(q.name.as_str()) {
                warn(out, m, it.pos, format!("input port '{}' of '{}' is not connected", q.name, i.module));
            }
        }
    }
}

/// Assignment style: `<=` in a combinational block, and `=` on a signal that another process reads in a clocked block.
fn check_process_style(m: &Module, out: &mut Vec<String>) {
    let outputs: BTreeSet<&str> = m.ports.iter().filter(|p| p.dir != Dir::Input).map(|p| p.name.as_str()).collect();
    let mut readers: HashMap<String, BTreeSet<usize>> = HashMap::new();
    for (idx, it) in m.items.iter().enumerate() {
        for name in mentioned(it) {
            readers.entry(name).or_default().insert(idx);
        }
    }
    for (idx, it) in m.items.iter().enumerate() {
        match &it.kind {
            ItemKind::AlwaysComb(body) => {
                let mut all = Vec::new();
                assignments(body, &mut all);
                if all.iter().any(|(_, nb)| *nb) {
                    warn(out, m, it.pos, "non-blocking assignment (<=) inside always_comb: use =".into());
                }
            }
            ItemKind::Always { body, .. } => {
                let mut all = Vec::new();
                assignments(body, &mut all);
                for (lhs, nonblocking) in all {
                    if nonblocking {
                        continue;
                    }
                    let names: BTreeSet<String> = parts(lhs).into_iter().filter_map(base).collect();
                    let shared = names.iter().find(|n| {
                        readers.get(n.as_str()).is_some_and(|r| r.iter().any(|&j| j != idx)) || outputs.contains(n.as_str())
                    });
                    if let Some(n) = shared {
                        warn(out, m, it.pos, format!("blocking assignment (=) to '{n}' inside a clocked block while another process reads it (a race): use <="));
                    }
                }
            }
            _ => {}
        }
    }
}

/// The names an item mentions (read or written).
fn mentioned(it: &Item) -> BTreeSet<String> {
    let names = RefCell::new(BTreeSet::new());
    let collect = |e: &Expr| {
        expr_any(e, &|x| {
            if let Expr::Ident(n, _) = x {
                names.borrow_mut().insert(n.clone());
            }
            false
        })
    };
    match &it.kind {
        ItemKind::ContAssign { lhs, rhs } => {
            collect(lhs);
            collect(rhs);
        }
        ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
            stmt_any_expr(body, &|x| {
                if let Expr::Ident(n, _) = x {
                    names.borrow_mut().insert(n.clone());
                }
                false
            });
        }
        ItemKind::Instance(i) => {
            for c in &i.conns {
                if let Some(e) = &c.expr {
                    collect(e);
                }
            }
        }
        _ => {}
    }
    let mut out = names.into_inner();
    if let ItemKind::Decl(d) = &it.kind {
        for x in &d.decls {
            if let Some(e) = &x.init {
                out.extend(reads_of(e));
            }
        }
    }
    out
}
