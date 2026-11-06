//! The checks of `ovc lint` on the elaborated modules (the Python lint checks of the same names): widths, latches,
//! drivers, instance connections and assignment style. They are warnings, and they run only in the lint, except the
//! undriven outputs, which the compiler refuses (`undriven_outputs`).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ast::{Dir, DType, Expr, File, Item, ItemKind, Module, Pos, Stmt};
use crate::error::Error;
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
        check_signs(m, env, wrapped, &mut out);
        check_sizes(m, env, &mut out);
        check_latches(m, env, &mut out);
        let mut undriven = Vec::new();
        check_drivers(m, &by_name, &mut out, &mut undriven);
        for e in undriven {
            let text = format!("{}:{}:{}: warning: {}", e.file, e.line, e.col, e.message);
            if !out.contains(&text) {
                out.push(text);
            }
        }
        check_connections(m, &by_name, &mut out);
        check_process_style(m, &mut out);
    }
    out
}

/// The output ports that nothing drives, in every module of the design: an undriven output is an error of the compiler
/// (its value is X in simulation, and the synthesis tools drop it).
pub fn undriven_outputs(modules: &[Module]) -> Vec<Error> {
    let flat: Vec<Module> = modules.iter().map(|m| Module { items: flatten(&m.items), ..m.clone() }).collect();
    let by_name: HashMap<&str, &Module> = flat.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut errors = Vec::new();
    for m in &flat {
        check_drivers(m, &by_name, &mut Vec::new(), &mut errors);
    }
    errors
}

/// The truncations written on purpose in the source, by module (without its specialisation).
#[derive(Default)]
pub struct Wrapped {
    /// The assignments with `.wrap()`: (module, signal, position of the assignment). The exemption is for that
    /// assignment only: another assignment to the same signal still needs its own `.wrap()`.
    signals: HashSet<(String, String, Pos)>,
    /// The targets among `signals` that are declared or assigned inside a generate loop: the elaboration names each
    /// of their copies with the loop's indices (`penalty` is `penalty_0`, `penalty_1`...). Their copies share the
    /// position of the assignment in the source.
    loop_signals: HashSet<(String, String, Pos)>,
    /// The connections with `.wrap()`: (module, instance, port).
    ports: HashSet<(String, String, String)>,
}

/// What the source writes with `.wrap()`.
pub fn wrapped_in(file: &File) -> Wrapped {
    let mut w = Wrapped::default();
    for m in &file.modules {
        scan_wrapped(&base_module(&m.name), &m.items, false, &mut w);
    }
    w
}

fn scan_wrapped(module: &str, items: &[Item], in_loop: bool, w: &mut Wrapped) {
    for it in items {
        match &it.kind {
            ItemKind::ContAssign { lhs, rhs } => mark(module, lhs, rhs, in_loop, it.pos, w),
            // `wire T x = e;` is an assignment of `e` to `x`
            ItemKind::Decl(d) => {
                for x in &d.decls {
                    if let Some(e) = &x.init {
                        mark(module, &Expr::Ident(x.name.clone(), (0, 0)), e, in_loop, it.pos, w);
                    }
                }
            }
            ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                for (lhs, rhs, _, pos) in assignments(body) {
                    mark(module, lhs, rhs, in_loop, pos, w);
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
            ItemKind::GenFor { body, .. } => scan_wrapped(module, body, true, w),
            ItemKind::Stage { items: body, .. } => scan_wrapped(module, body, in_loop, w),
            ItemKind::GenIf { then, els, .. } => {
                scan_wrapped(module, then, in_loop, w);
                if let Some(e) = els {
                    scan_wrapped(module, e, in_loop, w);
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

fn mark(module: &str, lhs: &Expr, rhs: &Expr, in_loop: bool, pos: Pos, w: &mut Wrapped) {
    if has_wrap(rhs) {
        for p in parts(lhs) {
            if let Some(n) = flat_name(p) {
                if in_loop {
                    w.loop_signals.insert((module.to_string(), n.clone(), pos));
                }
                w.signals.insert((module.to_string(), n, pos));
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
            // the statements of a pipeline stage are the module's own: they are checked like the others
            ItemKind::GenFor { body, .. } | ItemKind::GenIf { then: body, .. } | ItemKind::Stage { items: body, .. } => {
                out.extend(flatten(body))
            }
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
                if !target_wrapped(&key, lhs, it.pos, wrapped) {
                    lint_assign(m, env, lhs, rhs, it.pos, out);
                }
            }
            ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                for (lhs, rhs, _, pos) in assignments(body) {
                    if !target_wrapped(&key, lhs, pos, wrapped) {
                        lint_assign(m, env, lhs, rhs, pos, out);
                    }
                }
            }
            // `logic [7:0] a = d + 1;` is an assignment of `d + 1` to `a`
            ItemKind::Decl(d) => {
                for x in &d.decls {
                    if let Some(init) = &x.init {
                        let lhs = Expr::Ident(x.name.clone(), (0, 0));
                        if !target_wrapped(&key, &lhs, it.pos, wrapped) {
                            lint_assign(m, env, &lhs, init, it.pos, out);
                        }
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
/// The operators whose bits below the width of the target do not depend on the bits above the operands' width: a sum,
/// a product or a bitwise operation, truncated to the target.
fn is_modular_op(op: &str) -> bool {
    matches!(op, "+" | "-" | "*" | "&" | "|" | "^")
}

/// `modular` is the width of the target when the value is truncated to it with `.wrap()`. Then the bits of the target do
/// not depend on the signedness of an operand that is as wide as the target: it is not extended, and the extension of
/// the other operands (with zeros, unsigned) is the one the sum means. A signed operand narrower than the target would
/// be extended with zeros, so the operation is still reported.
fn agree_below_target(op: &str, a: &Expr, b: &Expr, modular: Option<u32>, env: &Env) -> bool {
    let Some(w) = modular.filter(|_| is_modular_op(op)) else { return false };
    [a, b].into_iter().all(|x| match signed_of(x, env) {
        Some(false) => true,
        Some(true) => width_of(x, env).is_some_and(|x| x.width >= w),
        None => false,
    })
}

fn sign_expr(m: &Module, env: &Env, e: &Expr, modular: Option<u32>, pos: Pos, out: &mut Vec<String>) {
    match e {
        Expr::Binary(op, a, b, _) => {
            let mixed_op = matches!(op.as_str(), "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "==" | "!=" | "<" | "<=" | ">" | ">=");
            // a decimal literal is signed and does no harm beside an unsigned value (`count + 1`)
            let neutral = |x: &Expr| matches!(x, Expr::Num(t) if !t.contains('\''));
            let kept = agree_below_target(op, a, b, modular, env);
            if mixed_op && !neutral(a) && !neutral(b) && !kept {
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
            let inner = if is_modular_op(op) { modular } else { None };
            sign_expr(m, env, a, inner, pos, out);
            sign_expr(m, env, b, inner, pos, out);
        }
        Expr::Unary(op, a) => sign_expr(m, env, a, if matches!(op.as_str(), "-" | "~") { modular } else { None }, pos, out),
        Expr::Cast(_, a) => sign_expr(m, env, a, None, pos, out),
        Expr::Ternary(c, a, b) => {
            sign_expr(m, env, c, None, pos, out);
            sign_expr(m, env, a, modular, pos, out);
            sign_expr(m, env, b, modular, pos, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|x| sign_expr(m, env, x, None, pos, out)),
        Expr::Index(a, i) => {
            sign_expr(m, env, a, None, pos, out);
            sign_expr(m, env, i, None, pos, out);
        }
        Expr::Slice(a, ..) => sign_expr(m, env, a, None, pos, out),
        Expr::Concat(parts) | Expr::Repl(_, parts) => parts.iter().for_each(|x| sign_expr(m, env, x, None, pos, out)),
        _ => {}
    }
}

/// The signedness of the operations: a signed operand mixed with an unsigned one (see `sign_expr`). A value written with
/// `.wrap()` is truncated to its target, so the operations that agree below that width are not reported.
fn check_signs(m: &Module, env: Option<&Env>, wrapped: &Wrapped, out: &mut Vec<String>) {
    let Some(env) = env else { return };
    let key = base_module(&m.name);
    let modular = |lhs: &Expr, pos: Pos| if target_wrapped(&key, lhs, pos, wrapped) { sized(lhs, env) } else { None };
    for it in &m.items {
        match &it.kind {
            ItemKind::ContAssign { lhs, rhs } => sign_expr(m, env, rhs, modular(lhs, it.pos), it.pos, out),
            ItemKind::Decl(d) => {
                for x in &d.decls {
                    if let Some(init) = &x.init {
                        sign_expr(m, env, init, modular(&Expr::Ident(x.name.clone(), (0, 0)), it.pos), it.pos, out);
                    }
                }
            }
            ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                for (lhs, rhs, _, pos) in assignments(body) {
                    sign_expr(m, env, rhs, modular(lhs, pos), pos, out);
                }
            }
            _ => {}
        }
    }
}

/// A shift of a sum or a product: SV evaluates the sum at the widest of the target and the operands, so the bits above
/// that width are gone before the shift (`y = (a * b) >> 8` on 8 bits). A signed operand is not checked here (`sign`).
fn check_sizes(m: &Module, env: Option<&Env>, out: &mut Vec<String>) {
    let Some(env) = env else { return };
    fn walk(m: &Module, env: &Env, e: &Expr, target: u32, pos: Pos, out: &mut Vec<String>) {
        match e {
            Expr::Binary(op, l, r, _) if matches!(op.as_str(), ">>" | ">>>") => {
                if let Expr::Binary(inner, a, b, _) = &**l {
                    if matches!(inner.as_str(), "+" | "-" | "*") && !signed_operand(a, env) && !signed_operand(b, env) {
                        let exact = width_of(l, env).map(|w| w.width);
                        let operands = [width_of(a, env), width_of(b, env)].into_iter().flatten().map(|w| w.width).max();
                        if let (Some(exact), Some(operands)) = (exact, operands) {
                            let evaluated = operands.max(target);
                            if exact > evaluated {
                                warn(out, m, pos, format!("'{}' is evaluated on {evaluated} bits before '>> {}': the bits above {evaluated} are lost (widen the operands, or cast the sum to {exact} bits)", text(l), text(r)));
                            }
                        }
                    }
                }
                walk(m, env, l, target, pos, out);
                walk(m, env, r, target, pos, out);
            }
            Expr::Binary(_, a, b, _) => {
                walk(m, env, a, target, pos, out);
                walk(m, env, b, target, pos, out);
            }
            Expr::Ternary(c, a, b) => {
                walk(m, env, c, target, pos, out);
                walk(m, env, a, target, pos, out);
                walk(m, env, b, target, pos, out);
            }
            Expr::Unary(_, a) => walk(m, env, a, target, pos, out),
            _ => {}
        }
    }
    // the operands of a sum are signed when they are declared `int` (the `sign` checks own those)
    fn signed_operand(e: &Expr, env: &Env) -> bool {
        matches!(signed_of(e, env), Some(true))
            || matches!(e, Expr::Ident(name, _) if env.signal(name).is_some_and(|(dt, _)| dt.name == "int" || dt.name == "integer"))
    }
    for it in &m.items {
        match &it.kind {
            ItemKind::ContAssign { lhs, rhs } => {
                if let Some(target) = sized(lhs, env) {
                    walk(m, env, rhs, target, it.pos, out);
                }
            }
            ItemKind::AlwaysComb(body) | ItemKind::Always { body, .. } => {
                for (lhs, rhs, _, pos) in assignments(body) {
                    if let Some(target) = sized(lhs, env) {
                        walk(m, env, rhs, target, pos, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn target_wrapped(module: &str, lhs: &Expr, pos: Pos, wrapped: &Wrapped) -> bool {
    parts(lhs).into_iter().filter_map(base).any(|n| is_wrapped(module, &n, pos, wrapped))
}

/// Whether the elaborated signal `name` is a target written with `.wrap()`. A signal of a generate loop is named with
/// the loop's indices (`penalty_0`, or `x_1_2` in nested loops): it matches its source name followed by indices.
fn is_wrapped(module: &str, name: &str, pos: Pos, wrapped: &Wrapped) -> bool {
    if wrapped.signals.contains(&(module.to_string(), name.to_string(), pos)) {
        return true;
    }
    wrapped.loop_signals.iter().any(|(m, source, at)| {
        m == module
            && *at == pos
            && name
                .strip_prefix(source.as_str())
                .and_then(|rest| rest.strip_prefix('_'))
                .is_some_and(|idx| idx.split('_').all(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit())))
    })
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

/// One driver of a signal: the item it is in, the target as written, how the item drives it, and the flattened name
/// of the target (`m[i].aw.valid` is `m_aw_valid`, the port of a vector of bundles).
struct Driver {
    item: usize,
    target: String,
    kind: String,
    pos: Pos,
    flat: String,
}

/// `undriven` gets the output ports that nothing drives (an error of the compiler, a warning of the lint).
fn check_drivers(m: &Module, by_name: &HashMap<&str, &Module>, out: &mut Vec<String>, undriven: &mut Vec<Error>) {
    let mut drivers: BTreeMap<String, Vec<Driver>> = BTreeMap::new();
    let mut maybe_driven: BTreeSet<String> = BTreeSet::new();
    let add = |drivers: &mut BTreeMap<String, Vec<Driver>>, lhs: &Expr, item: usize, kind: String, pos: Pos| {
        for part in parts(lhs) {
            if let Some(n) = base(part) {
                let flat = flat_name(part).unwrap_or_else(|| n.clone());
                drivers.entry(n).or_default().push(Driver { item, target: text(part), kind: kind.clone(), pos, flat });
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
                        // a bundle port of the target is connected whole: its output signals (`s_aw_ready`) drive the
                        // bundle (`s`), with the same suffix
                        let whole_bundle = !target.ports.iter().any(|q| &q.name == p);
                        if let (true, Some(root)) = (whole_bundle, base(e)) {
                            let flat = flat_name(e).unwrap_or_else(|| root.clone());
                            for q in target.ports.iter().filter(|q| q.dir == Dir::Output) {
                                if let Some(suffix) = q.name.strip_prefix(p.as_str()).and_then(|r| r.strip_prefix('_')) {
                                    let kind = format!("instance '{}'", i.name);
                                    drivers.entry(root.clone()).or_default().push(Driver { item: idx, target: text(e), kind, pos: it.pos, flat: format!("{flat}_{suffix}") });
                                }
                            }
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
    // a port is driven when one of its drivers is the port, or a field of it (a struct port driven field by field)
    let driven = |port: &str| {
        drivers.values().flatten().any(|d| d.flat == port || d.flat.strip_prefix(port).is_some_and(|rest| rest.starts_with('_')))
    };
    for p in m.ports.iter().filter(|p| p.dir == Dir::Output) {
        if !driven(&p.name) && !maybe_driven.contains(&p.name) {
            let file = if m.origin.is_empty() { "<lint>" } else { m.origin.as_str() };
            undriven.push(Error::new(format!("output port '{}' of '{}' is never driven", p.name, m.name), file, m.pos.0, m.pos.1));
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
        // an input the child never reads (a port of a feature it does not have, `WRITE_ENABLE(0)`) cannot be X in use
        let read: BTreeSet<String> = target.items.iter().flat_map(mentioned).collect();
        for q in &target.ports {
            if q.dir == Dir::Input && q.default.is_none() && !q.name.starts_with("ov_") && !connected.contains(q.name.as_str()) && read.contains(&q.name) {
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
