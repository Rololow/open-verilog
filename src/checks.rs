//! The checks of `ovc lint` on the elaborated modules (the Python lint checks of the same names): widths, latches,
//! drivers, instance connections and assignment style. They are warnings, and they run only in the lint: a design that
//! is compiled is not checked this way.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ast::{Dir, DType, Expr, File, Item, ItemKind, Module, Pos, Stmt};
use crate::domains::reads_of;
use crate::elab::fold::int_of;
use crate::elab::width::{width_of, Width};
use crate::elab::{expr_any, stmt_any_expr, Env};

/// The warnings of the checks, as the compiler writes them: `file:line:col: warning: message`.
///
/// `wrapped` lists the targets and the connections written with `.wrap()` in the source: the elaboration does not keep
/// the marker, and a truncation written on purpose is not a width problem.
pub fn check(modules: &[Module], envs: &HashMap<String, Env>, wrapped: &Wrapped) -> Vec<String> {
    // the items of the generate loops and conditions count as if they were written at the top, as they are unrolled
    // in the reference (an instance in a loop drives its outputs all the same)
    let flat: Vec<Module> = modules.iter().map(|m| Module { items: flatten(&m.items), ..m.clone() }).collect();
    let modules = &flat;
    let by_name: HashMap<&str, &Module> = modules.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut out = Vec::new();
    for m in modules {
        let env = envs.get(&m.name);
        check_widths(m, env, &by_name, wrapped, &mut out);
        check_signs(m, env, &mut out);
        check_latches(m, env, &mut out);
        check_drivers(m, &by_name, &mut out);
        check_connections(m, &by_name, &mut out);
        check_process_style(m, &mut out);
    }
    out
}

/// The truncations written on purpose in the source, by module (without its specialisation).
#[derive(Default)]
pub struct Wrapped {
    /// The targets of an assignment with `.wrap()`: (module, signal).
    signals: HashSet<(String, String)>,
    /// The connections with `.wrap()`: (module, instance, port).
    ports: HashSet<(String, String, String)>,
}

/// What the source writes with `.wrap()`.
pub fn wrapped_in(file: &File) -> Wrapped {
    let mut w = Wrapped::default();
    for m in &file.modules {
        scan_wrapped(&base_module(&m.name), &m.items, &mut w);
    }
    w
}

fn scan_wrapped(module: &str, items: &[Item], w: &mut Wrapped) {
    for it in items {
        match &it.kind {
            ItemKind::ContAssign { lhs, rhs } => mark(module, lhs, rhs, w),
            // `wire T x = e;` is an assignment of `e` to `x`
            ItemKind::Decl(d) => {
                for x in &d.decls {
                    if let Some(e) = &x.init {
                        mark(module, &Expr::Ident(x.name.clone(), (0, 0)), e, w);
                    }
                }
            }
            ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                for (lhs, rhs, _, _) in assignments(body) {
                    mark(module, lhs, rhs, w);
                }
            }
            ItemKind::Instance(i) => {
                for c in &i.conns {
                    if let (Some(p), Some(e)) = (&c.port, &c.expr) {
                        if has_wrap(e) {
                            w.ports.insert((module.to_string(), i.name.clone(), p.clone()));
                        }
                    }
                }
            }
            ItemKind::GenFor { body, .. } | ItemKind::Stage { items: body, .. } => scan_wrapped(module, body, w),
            ItemKind::GenIf { then, els, .. } => {
                scan_wrapped(module, then, w);
                if let Some(e) = els {
                    scan_wrapped(module, e, w);
                }
            }
            _ => {}
        }
    }
}

/// The name of a target as the elaborated design has it: a member of a bundle is `bus_field` (`move_write.row` is
/// `move_write_row`), an element of an array is its array.
fn flat_name(e: &Expr) -> Option<String> {
    match e {
        Expr::Ident(n, _) => Some(n.clone()),
        Expr::Index(b, _) | Expr::Slice(b, _, _, _) => flat_name(b),
        Expr::Member(b, f, _) => Some(format!("{}_{f}", flat_name(b)?)),
        _ => None,
    }
}

fn mark(module: &str, lhs: &Expr, rhs: &Expr, w: &mut Wrapped) {
    if has_wrap(rhs) {
        for p in parts(lhs) {
            if let Some(n) = flat_name(p) {
                w.signals.insert((module.to_string(), n));
            }
        }
    }
}

fn has_wrap(e: &Expr) -> bool {
    expr_any(e, &|x| matches!(x, Expr::Call(f, _) if matches!(&**f, Expr::Member(_, m, _) if m == "wrap")))
}

/// The module a specialised name (`Ram__32_512`) comes from.
fn base_module(name: &str) -> String {
    name.split("__").next().unwrap_or(name).to_string()
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

/// Whether two expressions are the same as written (positions aside).
fn same(a: &Expr, b: &Expr) -> bool {
    text(a) == text(b)
}

fn text(e: &Expr) -> String {
    crate::emit::expr::expr(e, -1, false).unwrap_or_else(|_| "?".into())
}

/// Every assignment of a statement: its target, its value, whether it is non-blocking, and its position.
fn assignments(s: &Stmt) -> Vec<(&Expr, &Expr, bool, Pos)> {
    let mut out = Vec::new();
    collect(s, &mut out);
    out
}

fn collect<'a>(s: &'a Stmt, out: &mut Vec<(&'a Expr, &'a Expr, bool, Pos)>) {
    match s {
        Stmt::Assign { lhs, rhs, nonblocking, pos } => out.push((lhs, rhs, *nonblocking, *pos)),
        Stmt::Block(v) => v.iter().for_each(|x| collect(x, out)),
        Stmt::If { then, els, .. } => {
            collect(then, out);
            if let Some(e) = els {
                collect(e, out);
            }
        }
        Stmt::Case { arms, .. } => arms.iter().for_each(|(_, b)| collect(b, out)),
        Stmt::For { body, .. } => collect(body, out),
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
        Stmt::Case { subject, kind, arms, .. } => {
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

/// The width of an expression, when it is known and is not an unsized literal.
fn sized(e: &Expr, env: &Env) -> Option<u32> {
    width_of(e, env).filter(|w: &Width| !w.flex).map(|w| w.width)
}

/// The width of a port's type, when it is a plain vector (no unpacked dimensions).
fn port_width(dtype: &DType, dims: &[Expr]) -> Option<i64> {
    if !dims.is_empty() {
        return None;
    }
    if !(dtype.name.is_empty() || dtype.name == "logic") {
        return None;
    }
    if dtype.packed.is_empty() {
        return Some(1);
    }
    let mut w = 1;
    for (msb, lsb) in &dtype.packed {
        w *= (int_of(msb)? - int_of(lsb)?).abs() + 1;
    }
    Some(w)
}

/// `target = value`: the value must not be wider than the target, unless the bits dropped are the point.
fn lint_assign(m: &Module, env: &Env, lhs: &Expr, rhs: &Expr, pos: Pos, out: &mut Vec<String>) {
    let Some(lw) = width_of(lhs, env).map(|w| w.width) else { return };
    let name = base(lhs).unwrap_or_else(|| text(lhs));
    if let Expr::Num(t) = rhs {
        if !t.contains('\'') {
            if let Some(v) = crate::lexer::number_value(t) {
                if v >= 0 && (64 - (v as u64).leading_zeros()) > lw {
                    warn(out, m, pos, format!("literal {v} does not fit in {lw} bits ('{name}' is {lw} bits wide)"));
                }
            }
        }
        return;
    }
    let Some(rw) = sized(rhs, env) else { return };
    if rw <= lw {
        return;
    }
    if let Expr::Binary(op, a, b, _) = rhs {
        // a shift register (`x = x << n`) and an accumulator (`x = x + y`) drop their bits on purpose
        if (op == "<<" || op == "<<<") && same(a, lhs) {
            return;
        }
        if op == "+" || op == "-" || op == "*" {
            if op != "*" && (same(a, lhs) || same(b, lhs)) {
                return;
            }
            // the operands' own widths for the explanation: an unsized literal counts as its bit length, as in the reference
            let (wa, wb) = (width_of(a, env).map(|w| w.width), width_of(b, env).map(|w| w.width));
            let how = match (op.as_str(), wa, wb) {
                ("*", Some(wa), Some(wb)) => format!(" ({wa} + {wb})"),
                (_, Some(wa), Some(wb)) if wa + 1 == rw || wb + 1 == rw => format!(" (max({wa}, {wb}) + 1)"),
                _ => String::new(),
            };
            warn(out, m, pos, format!("'{name}' is {lw} bits but '{}' needs {rw} bits{how}: widen '{name}' or use .wrap() to truncate on purpose", text(rhs)));
            return;
        }
    }
    warn(
        out,
        m,
        pos,
        format!(
            "'{name}' is {lw} bits but the value is {rw} bits wide: the upper {} bits are dropped (use .wrap() or a slice to truncate on purpose)",
            rw - lw
        ),
    );
}

/// The width checks: assignments, and the connections of the instances to the ports they drive.
fn check_widths(m: &Module, env: Option<&Env>, by_name: &HashMap<&str, &Module>, wrapped: &Wrapped, out: &mut Vec<String>) {
    let Some(env) = env else { return };
    let key = base_module(&m.name);
    for it in &m.items {
        match &it.kind {
            ItemKind::ContAssign { lhs, rhs } => {
                if !target_wrapped(&key, lhs, wrapped) {
                    lint_assign(m, env, lhs, rhs, it.pos, out);
                }
            }
            ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                for (lhs, rhs, _, pos) in assignments(body) {
                    if !target_wrapped(&key, lhs, wrapped) {
                        lint_assign(m, env, lhs, rhs, pos, out);
                    }
                }
            }
            ItemKind::Instance(i) => {
                let Some(target) = by_name.get(i.module.as_str()) else { continue };
                for c in &i.conns {
                    let (Some(p), Some(e)) = (&c.port, &c.expr) else { continue };
                    if wrapped.ports.contains(&(key.clone(), i.name.clone(), p.clone())) {
                        continue;
                    }
                    let Some(q) = target.ports.iter().find(|q| &q.name == p) else { continue };
                    let Some(pw) = port_width(&q.dtype, &q.dims) else { continue };
                    if let Expr::Num(t) = e {
                        if let Some(v) = crate::lexer::number_value(t).filter(|v| *v >= 0 && t.chars().all(|c| c.is_ascii_digit())) {
                            if (v as u64) >> pw.min(63) != 0 {
                                warn(out, m, it.pos, format!("port '{p}' of '{}' is {pw} bits: literal {t} does not fit", i.module));
                            }
                        }
                        continue;
                    }
                    if let Some(ew) = sized(e, env) {
                        if i64::from(ew) != pw {
                            warn(out, m, it.pos, format!("port '{p}' of '{}' is {pw} bits but is connected to '{}' ({ew} bits)", i.module, text(e)));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Whether an expression is signed in SystemVerilog, when it can be told. A decimal literal is signed; a based one is not.
fn signed_of(e: &Expr, env: &Env) -> Option<bool> {
    match e {
        Expr::Ident(name, _) => env.signal(name).map(|(dt, _)| dt.signed),
        Expr::Num(t) => Some(!t.contains('\'')),
        Expr::Call(f, args) => match (&**f, args.as_slice()) {
            (Expr::Ident(n, _), [_]) if n == "$signed" => Some(true),
            (Expr::Ident(n, _), [_]) if n == "$unsigned" => Some(false),
            _ => None,
        },
        Expr::Binary(op, a, b, _) => match op.as_str() {
            "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||" => Some(false),
            "<<" | "<<<" | ">>" | ">>>" => signed_of(a, env),
            _ => Some(signed_of(a, env)? && signed_of(b, env)?),
        },
        // a reduction is unsigned; `-x` and `~x` keep the sign of `x`
        Expr::Unary(op, a) => match op.as_str() {
            "!" | "&" | "|" | "^" => Some(false),
            _ => signed_of(a, env),
        },
        Expr::Ternary(_, a, b) => Some(signed_of(a, env)? && signed_of(b, env)?),
        Expr::Cast(_, a) => signed_of(a, env),
        Expr::Index(..) | Expr::Slice(..) | Expr::Concat(_) | Expr::Repl(..) => Some(false),
        _ => None,
    }
}

/// The signals that a signed operand brings into an expression, with their widths (`$signed(a) * $signed(b)` gives
/// `a` and `b`, not the product).
fn signed_leaves(e: &Expr, env: &Env, out: &mut Vec<(String, u32)>) {
    match e {
        Expr::Ident(name, _) => {
            if env.signal(name).is_some_and(|(dt, _)| dt.signed) {
                out.extend(sized(e, env).map(|w| (name.clone(), w)));
            }
        }
        Expr::Call(f, args) if matches!(&**f, Expr::Ident(n, _) if n == "$signed") => {
            if let [a] = args.as_slice() {
                match a {
                    Expr::Ident(name, _) => out.extend(sized(a, env).map(|w| (name.clone(), w))),
                    _ => signed_leaves(a, env, out),
                }
            }
        }
        Expr::Binary(op, a, b, _) if matches!(op.as_str(), "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^") => {
            signed_leaves(a, env, out);
            signed_leaves(b, env, out);
        }
        Expr::Cast(_, a) | Expr::Unary(_, a) => signed_leaves(a, env, out),
        _ => {}
    }
}

/// A mixed operation: a signed operand and an unsigned one. SV makes the whole operation unsigned, so the signed
/// operand is extended with zeros to the width of the operation, and the result is not the signed one.
fn sign_expr(m: &Module, env: &Env, e: &Expr, pos: Pos, out: &mut Vec<String>) {
    match e {
        Expr::Binary(op, a, b, _) => {
            let mixed_op = matches!(op.as_str(), "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "==" | "!=" | "<" | "<=" | ">" | ">=");
            // a decimal literal is signed and does no harm beside an unsigned value (`count + 1`)
            let neutral = |x: &Expr| matches!(x, Expr::Num(t) if !t.contains('\''));
            if mixed_op && !neutral(a) && !neutral(b) {
                if let (Some(sa), Some(sb)) = (signed_of(a, env), signed_of(b, env)) {
                    if sa != sb {
                        let mut leaves = Vec::new();
                        signed_leaves(if sa { a } else { b }, env, &mut leaves);
                        if let Some((name, _)) = leaves.first() {
                            warn(out, m, pos, format!("'{name}' is signed but this expression is unsigned in SV: '{}' mixes it with an unsigned value", text(e)));
                        }
                        let width = [width_of(a, env), width_of(b, env)].into_iter().flatten().map(|w| w.width).max();
                        for (name, w) in &leaves {
                            if let Some(width) = width.filter(|width| width > w) {
                                warn(out, m, pos, format!("'{name}' is extended from {w} to {width} bits with zeros, not with its sign"));
                            }
                        }
                    }
                }
            }
            sign_expr(m, env, a, pos, out);
            sign_expr(m, env, b, pos, out);
        }
        Expr::Unary(_, a) | Expr::Cast(_, a) => sign_expr(m, env, a, pos, out),
        Expr::Ternary(c, a, b) => {
            sign_expr(m, env, c, pos, out);
            sign_expr(m, env, a, pos, out);
            sign_expr(m, env, b, pos, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|x| sign_expr(m, env, x, pos, out)),
        Expr::Index(a, i) => {
            sign_expr(m, env, a, pos, out);
            sign_expr(m, env, i, pos, out);
        }
        Expr::Slice(a, ..) => sign_expr(m, env, a, pos, out),
        Expr::Concat(parts) | Expr::Repl(_, parts) => parts.iter().for_each(|x| sign_expr(m, env, x, pos, out)),
        _ => {}
    }
}

/// The signedness of the operations: a signed operand mixed with an unsigned one (see `sign_expr`).
fn check_signs(m: &Module, env: Option<&Env>, out: &mut Vec<String>) {
    let Some(env) = env else { return };
    for it in &m.items {
        match &it.kind {
            ItemKind::ContAssign { rhs, .. } => sign_expr(m, env, rhs, it.pos, out),
            ItemKind::Decl(d) => {
                for x in &d.decls {
                    if let Some(init) = &x.init {
                        sign_expr(m, env, init, it.pos, out);
                    }
                }
            }
            ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                for (_, rhs, _, pos) in assignments(body) {
                    sign_expr(m, env, rhs, pos, out);
                }
            }
            _ => {}
        }
    }
}

fn target_wrapped(module: &str, lhs: &Expr, wrapped: &Wrapped) -> bool {
    parts(lhs).into_iter().filter_map(base).any(|n| wrapped.signals.contains(&(module.to_string(), n)))
}

/// A combinational block must assign each signal on every path, or a latch is inferred.
fn check_latches(m: &Module, env: Option<&Env>, out: &mut Vec<String>) {
    for it in &m.items {
        let ItemKind::AlwaysComb(body) = &it.kind else { continue };
        let assigned: BTreeSet<String> = assignments(body).iter().flat_map(|(lhs, _, _, _)| parts(lhs)).filter_map(base).collect();
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
                for (lhs, _, _, _) in assignments(body) {
                    add(&mut drivers, lhs, idx, "always_comb".into(), it.pos);
                }
            }
            ItemKind::Always { body, .. } => {
                for (lhs, _, _, _) in assignments(body) {
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
                for (_, _, _, pos) in assignments(body).into_iter().filter(|(_, _, nb, _)| *nb) {
                    warn(out, m, pos, "non-blocking assignment (<=) inside always_comb: use =".into());
                }
            }
            ItemKind::Always { body, .. } => {
                for (lhs, _, nonblocking, pos) in assignments(body) {
                    if nonblocking {
                        continue;
                    }
                    let names: BTreeSet<String> = parts(lhs).into_iter().filter_map(base).collect();
                    let shared = names.iter().find(|n| {
                        readers.get(n.as_str()).is_some_and(|r| r.iter().any(|&j| j != idx)) || outputs.contains(n.as_str())
                    });
                    if let Some(n) = shared {
                        warn(out, m, pos, format!("blocking assignment (=) to '{n}' inside a clocked block while another process reads it (a race): use <="));
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
