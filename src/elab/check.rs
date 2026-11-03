//! Diagnostics of an elaborated module: unknown names and members (with "did you mean" suggestions), a variable written by
//! an `always_ff` and an `initial`, selects past the width of their value, struct views of another width, and inputs
//! driven inside the module. They are errors of the elaboration, found after it.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::ast::{Dir, Expr, Item, ItemKind, Module, Pos, Stmt};
use crate::elab::env::Env;
use crate::elab::fold::{self, fold_expr, int_of};
use crate::elab::{expr_any, stmt_any_expr, width};
use crate::pipeline::arrays::rewrite_item;

/// The similarity of two names, as `difflib` computes it: twice the size of the matching blocks over the total length.
pub(super) fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    2.0 * matching(&a, &b) as f64 / (a.len() + b.len()) as f64
}

/// The number of characters in matching blocks, found the way `difflib` finds them (the longest block first, then the
/// blocks on each side of it).
pub(super) fn matching(a: &[char], b: &[char]) -> usize {
    let (mut bi, mut bj, mut bk) = (0, 0, 0);
    for i in 0..a.len() {
        for j in 0..b.len() {
            let k = (0..).take_while(|&k| i + k < a.len() && j + k < b.len() && a[i + k] == b[j + k]).count();
            if k > bk {
                (bi, bj, bk) = (i, j, k);
            }
        }
    }
    if bk == 0 {
        return 0;
    }
    bk + matching(&a[..bi], &b[..bj]) + matching(&a[bi + bk..], &b[bj + bk..])
}

/// ` (did you mean 'X'?)` for the candidate closest to `name` (similarity 0.6 or more), or nothing.
pub(crate) fn suggest(name: &str, candidates: impl IntoIterator<Item = String>) -> String {
    let best = candidates
        .into_iter()
        .map(|c| (similarity(name, &c), c))
        .filter(|(score, _)| *score >= 0.6)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.1.cmp(&b.1)));
    match best {
        Some((_, c)) => format!(" (did you mean '{c}'?)"),
        None => String::new(),
    }
}

/// The names a module declares or can name: its ports, signals, instances, tasks and the locals of its blocks.
pub(super) fn known_names(module: &Module) -> Vec<String> {
    let mut known: HashSet<String> = HashSet::new();
    known.insert(module.name.clone());
    known.extend(module.ports.iter().map(|p| p.name.clone()));
    for item in &module.items {
        match &item.kind {
            ItemKind::Decl(d) => known.extend(d.decls.iter().map(|x| x.name.clone())),
            ItemKind::Instance(i) => {
                known.insert(i.name.clone());
            }
            ItemKind::Task { name, .. } => {
                known.insert(name.clone());
            }
            ItemKind::Always { body, .. } | ItemKind::AlwaysComb(body) | ItemKind::Initial(body) => declared_in_stmt(body, &mut known),
            _ => {}
        }
    }
    known.into_iter().collect()
}

/// A member that the elaboration did not resolve: a member of an enum that has no such state, a field that a struct or
/// a bundle does not have. The valid members are resolved already, so what is left is a mistake.
pub(super) fn member_error(module: &Module, env: &Env) -> Option<(String, Pos)> {
    let found: std::cell::RefCell<Option<(String, Pos)>> = std::cell::RefCell::new(None);
    let check = |e: &Expr| -> bool {
        if found.borrow().is_some() {
            return false;
        }
        let Expr::Member(base, field, pos) = e else { return false };
        let Expr::Ident(name, _) = &**base else { return false };
        if let Some(info) = env.enum_info(name) {
            if !info.members.iter().any(|(m, _)| m == field) {
                let hint = suggest(field, info.members.iter().map(|(m, _)| m.clone()));
                *found.borrow_mut() = Some((format!("enum '{name}' has no member '{field}'{hint}"), *pos));
            }
        } else if let Some(s) = env.struct_of(name) {
            if !s.fields.iter().any(|(f, _, _)| f == field) {
                let hint = suggest(field, s.fields.iter().map(|(f, _, _)| f.clone()));
                let sname = env.struct_name_of(name).unwrap_or_default();
                *found.borrow_mut() = Some((format!("struct '{sname}' has no field '{field}'{hint}"), *pos));
            }
        } else if let Some(b) = env.bundle_of(name) {
            if !b.fields.iter().any(|(f, _)| f == field) {
                let hint = suggest(field, b.fields.iter().map(|(f, _)| f.clone()));
                *found.borrow_mut() = Some((format!("bundle '{}' has no field '{field}'{hint}", b.bundle), *pos));
            }
        } else if let Some((_, isa, format)) = env.view(name) {
            // a view of an instruction: `opcode`, or a field of its format
            if let Some(info) = env.isa_info(&isa) {
                let fields: Vec<String> = info.formats.iter().find(|f| f.name == format).map(|f| f.fields.iter().map(|x| x.name.clone()).collect()).unwrap_or_default();
                if field != "opcode" && !fields.contains(field) {
                    let hint = suggest(field, fields.clone());
                    *found.borrow_mut() = Some((format!("format '{format}' has no field '{field}'{hint}"), *pos));
                }
            }
        }
        false
    };
    fn items_check(items: &[Item], check: &dyn Fn(&Expr) -> bool) {
        for it in items {
            match &it.kind {
                ItemKind::ContAssign { lhs, rhs } => {
                    expr_any(lhs, check);
                    expr_any(rhs, check);
                }
                ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                    stmt_any_expr(body, check);
                }
                ItemKind::Instance(i) => {
                    for c in &i.conns {
                        if let Some(e) = &c.expr {
                            expr_any(e, check);
                        }
                    }
                }
                ItemKind::GenFor { body, .. } => items_check(body, check),
                ItemKind::GenIf { then, els, .. } => {
                    items_check(then, check);
                    if let Some(e) = els {
                        items_check(e, check);
                    }
                }
                _ => {}
            }
        }
    }
    items_check(&module.items, &check);
    found.into_inner()
}

pub(super) fn unknown_identifier(module: &Module) -> Option<(String, Pos)> {
    let mut known: HashSet<String> = HashSet::new();
    known.insert(module.name.clone());
    known.extend(module.ports.iter().map(|p| p.name.clone()));
    let mut used: Vec<(String, Pos)> = Vec::new();
    for item in &module.items {
        match &item.kind {
            ItemKind::Decl(d) => known.extend(d.decls.iter().map(|x| x.name.clone())),
            ItemKind::Instance(i) => {
                known.insert(i.name.clone());
            }
            ItemKind::Task { name, .. } => {
                known.insert(name.clone());
            }
            _ => {}
        }
    }
    for item in &module.items {
        match &item.kind {
            ItemKind::Always { body, .. } | ItemKind::AlwaysComb(body) | ItemKind::Initial(body) => declared_in_stmt(body, &mut known),
            _ => {}
        }
    }
    for item in &module.items {
        match &item.kind {
            ItemKind::ContAssign { lhs, rhs } => {
                idents_in(lhs, &mut used);
                idents_in(rhs, &mut used);
            }
            ItemKind::Always { body, .. } | ItemKind::AlwaysComb(body) | ItemKind::Initial(body) => idents_in_stmt(body, &mut used),
            ItemKind::Decl(d) => {
                for x in &d.decls {
                    x.init.iter().for_each(|e| idents_in(e, &mut used));
                }
            }
            _ => {}
        }
    }
    // the instances of a loop are `m_0`, `m_1`, ...: the family `m` is a name too (`m[1].leaf` reaches into one)
    let family = |name: &str| module.items.iter().any(|item| matches!(&item.kind, ItemKind::Instance(i) if i.name.starts_with(&format!("{name}_"))));
    used.into_iter().find(|(name, _)| !name.starts_with('$') && !known.contains(name) && !family(name))
}

/// The variable a statement assigns, when it is a name or an element of one.
pub(super) fn assigned_name(lhs: &Expr) -> Option<String> {
    match lhs {
        Expr::Ident(name, _) => Some(name.clone()),
        Expr::Index(base, _) => assigned_name(base),
        _ => None,
    }
}

/// The variables a statement assigns.
pub(super) fn assigned_in_stmt(s: &Stmt, out: &mut Vec<String>) {
    match s {
        Stmt::Assign { lhs, .. } => out.extend(assigned_name(lhs)),
        Stmt::Block(stmts) => stmts.iter().for_each(|x| assigned_in_stmt(x, out)),
        Stmt::If { then, els, .. } => {
            assigned_in_stmt(then, out);
            if let Some(e) = els {
                assigned_in_stmt(e, out);
            }
        }
        Stmt::For { body, .. } => assigned_in_stmt(body, out),
        Stmt::Case { arms, .. } => arms.iter().for_each(|(_, body)| assigned_in_stmt(body, out)),
        _ => {}
    }
}

/// A variable written by an `always_ff` and by an `initial` block: the simulator takes both, and Questa rejects it.
pub(super) fn written_by_ff_and_initial(module: &Module) -> Option<(String, Pos)> {
    let mut ff: Vec<(String, Pos)> = Vec::new();
    let mut init: Vec<String> = Vec::new();
    for item in &module.items {
        match &item.kind {
            ItemKind::Always { body, keyword, .. } if keyword == "always_ff" => {
                let mut names = Vec::new();
                assigned_in_stmt(body, &mut names);
                ff.extend(names.into_iter().map(|n| (n, item.pos)));
            }
            ItemKind::Initial(body) => assigned_in_stmt(body, &mut init),
            _ => {}
        }
    }
    ff.into_iter().find(|(n, _)| init.contains(n))
}

/// Visits every expression of an item: its statements and declarations, and the value of a module-level `val`.
pub(super) fn visit_item(it: &Item, visit: &dyn Fn(&Expr) -> std::result::Result<Option<Expr>, crate::pipeline::rewrite::Failure>) {
    let _ = rewrite_item(it, visit);
    if let ItemKind::Val { expr, .. } = &it.kind {
        let mut f = |e: &Expr| visit(e);
        let _ = crate::pipeline::rewrite::map_expr(expr, &mut f);
    }
}

/// A part-select past the width of an expression (`(a + b)[9:4]` of a 9-bit sum): SV would take the missing bits as
/// nothing, so it is an error. The first one, as a message.
pub(super) fn select_past_width(items: &[Item], env: &Env) -> Option<String> {
    let found: RefCell<Option<String>> = RefCell::new(None);
    for it in items {
        visit_item(it, &|e| {
            if let Expr::Slice(base, msb, lsb, op) = e {
                if op.as_str() == ":" && fold::is_expression(base) && found.borrow().is_none() {
                    if let (Some(m), Some(l), Some(w)) = (int_of(&fold_expr(msb, env)), int_of(&fold_expr(lsb, env)), width::width_of(base, env)) {
                        if m >= i64::from(w.width) {
                            *found.borrow_mut() = Some(format!("bits [{m}:{l}] are out of range for this {}-bit value", w.width));
                        }
                    }
                }
            }
            Ok(None)
        });
    }
    found.into_inner()
}

/// A value viewed as a struct (`a.as(S)`) of another width than the struct: the view would read bits that are not there.
pub(super) fn view_width_error(items: &[Item], env: &Env) -> Option<String> {
    let found: RefCell<Option<String>> = RefCell::new(None);
    for it in items {
        visit_item(it, &|e| {
            if let Expr::Call(f, args) = e {
                if let Expr::Member(base, method, _) = &**f {
                    if method == "as" && found.borrow().is_none() {
                        let key = match args.as_slice() {
                            [Expr::Ident(name, _)] => Some(name.clone()),
                            [Expr::Type(dt)] => fold::struct_key(dt, env),
                            _ => None,
                        };
                        if let (Some(s), Some(w)) = (key.as_ref().and_then(|k| env.struct_info(k)), width::width_of(base, env)) {
                            if i64::from(w.width) != i64::from(s.width) {
                                let name = key.unwrap_or_default();
                                *found.borrow_mut() = Some(format!(
                                    "a view as struct '{name}' needs the struct's width: it is {} bits but the struct is {} bits",
                                    w.width, s.width
                                ));
                            }
                        }
                    }
                }
            }
            Ok(None)
        });
    }
    found.into_inner()
}

/// The names a statement declares: its local declarations and the variables of its loops.
pub(super) fn declared_in_stmt(s: &Stmt, known: &mut HashSet<String>) {
    match s {
        Stmt::Decl(d) => known.extend(d.decls.iter().map(|x| x.name.clone())),
        Stmt::Block(stmts) => stmts.iter().for_each(|x| declared_in_stmt(x, known)),
        Stmt::If { then, els, .. } => {
            declared_in_stmt(then, known);
            if let Some(e) = els {
                declared_in_stmt(e, known);
            }
        }
        Stmt::For { header, body } => {
            known.insert(header.var.clone());
            declared_in_stmt(body, known);
        }
        Stmt::Case { arms, .. } => arms.iter().for_each(|(_, body)| declared_in_stmt(body, known)),
        _ => {}
    }
}

/// The identifiers an expression reads, with their positions.
pub(super) fn idents_in(e: &Expr, out: &mut Vec<(String, Pos)>) {
    let _ = crate::pipeline::rewrite::map_expr(e, &mut |x| {
        if let Expr::Ident(name, pos) = x {
            out.push((name.clone(), *pos));
        }
        Ok(None)
    });
}

pub(super) fn idents_in_stmt(s: &Stmt, out: &mut Vec<(String, Pos)>) {
    let _ = crate::pipeline::rewrite::map_stmt(s, &mut |x| {
        if let Expr::Ident(name, pos) = x {
            out.push((name.clone(), *pos));
        }
        Ok(None)
    });
}

/// The first input port that the module drives: an assignment whose target starts from it (`a`, `a[3]`, `a.b`).
pub(super) fn driven_input(module: &Module) -> Option<(String, Pos)> {
    let inputs: Vec<&str> = module.ports.iter().filter(|p| p.dir == Dir::Input).map(|p| p.name.as_str()).collect();
    let mut found = Vec::new();
    for item in &module.items {
        match &item.kind {
            ItemKind::ContAssign { lhs, .. } => assigns_in_expr(lhs, &mut found),
            ItemKind::Always { body, .. } => assigns_in_stmt(body, &mut found),
            ItemKind::AlwaysComb(body) => assigns_in_stmt(body, &mut found),
            _ => {}
        }
    }
    found.into_iter().find(|(name, _)| inputs.contains(&name.as_str()))
}

/// The base name and position of an assignment target.
pub(super) fn assigns_in_expr(target: &Expr, out: &mut Vec<(String, Pos)>) {
    match target {
        Expr::Ident(name, pos) => out.push((name.clone(), *pos)),
        Expr::Index(base, _) | Expr::Slice(base, _, _, _) | Expr::Member(base, _, _) => assigns_in_expr(base, out),
        _ => {}
    }
}

pub(super) fn assigns_in_stmt(s: &Stmt, out: &mut Vec<(String, Pos)>) {
    match s {
        Stmt::Block(stmts) => stmts.iter().for_each(|x| assigns_in_stmt(x, out)),
        Stmt::Assign { lhs, .. } => assigns_in_expr(lhs, out),
        Stmt::If { then, els, .. } => {
            assigns_in_stmt(then, out);
            if let Some(e) = els {
                assigns_in_stmt(e, out);
            }
        }
        Stmt::For { body, .. } => assigns_in_stmt(body, out),
        Stmt::Case { arms, .. } => arms.iter().for_each(|(_, body)| assigns_in_stmt(body, out)),
        _ => {}
    }
}
