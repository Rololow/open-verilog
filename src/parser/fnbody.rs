//! Bodies of functions: statements (`val`, `if`, `return`) lowered to one expression.
//!
//! A `val` is substituted into the statements after it, and an `if` whose branches both return becomes a
//! conditional expression, so a function of statements is an ordinary pure function (see `elab::fold::inline`).

use super::Parser;
use std::collections::HashMap;

use crate::ast::{DType, Expr, Item, ItemKind, Stmt};
use crate::elab::env::Env;
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
            // a field of a struct local (`cfg.XLEN`), assigned earlier in the body
            Expr::Member(base, field, _) => match &**base {
                Expr::Ident(v, _) => {
                    let name = format!("{v}.{field}");
                    env.iter().rev().find(|(n, _)| *n == name).map(|(_, value)| value.clone())
                }
                _ => None,
            },
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

/// A statement of a SystemVerilog function body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvStmt {
    Assign(String, Expr),
    If(Expr, Vec<SvStmt>, Vec<SvStmt>),
    /// `for (var = lo; var cmp bound; var = step) body`: unrolled when its bounds are constants.
    For { var: String, lo: Expr, cmp: String, bound: Expr, step: Expr, body: Vec<SvStmt> },
    /// `name[msb:lsb] = value`: some bits of a local; the others keep their value.
    PartAssign { name: String, msb: Expr, lsb: Expr, value: Expr },
    /// The width of a local, from its declaration (read by `PartAssign`).
    Width(String, Expr),
}

/// The globals that a function body folds with when it is read at its definition: none.
pub(crate) fn no_globals() -> Env {
    Env::with_enums(std::rc::Rc::new(std::collections::BTreeMap::new()))
}

/// Runs a function body on its environment (the parameters bound, at a call), checks that every bit of its locals is
/// assigned, and reads its result.
pub(crate) fn sv_build(stmts: &[SvStmt], name: &str, env: &mut Vec<(String, Expr)>, g: &Env) -> std::result::Result<Expr, String> {
    sv_run(stmts, env, g)?;
    if let Some((n, _)) = env.iter().find(|(n, v)| n.ends_with("#unset") && sv_const(v, g) != Some(0)) {
        let local = n.trim_end_matches("#unset");
        return Err(format!("some bits of '{local}' are never assigned in the function"));
    }
    sv_result(env, name)
}

/// The value of a function: its result, or the fields of a struct result (a pattern of its fields).
fn sv_result(env: &[(String, Expr)], name: &str) -> std::result::Result<Expr, String> {
    let mut body = match sv_get(env, name) {
        Some(e) => e.clone(),
        None => return Err("a function must assign its result on every path".into()),
    };
    // a struct result: `return cfg;` of a struct local, or fields assigned to the result, is a pattern of its fields
    let fields_of = |local: &str| -> Vec<Expr> {
        let prefix = format!("{local}.");
        env.iter()
            .filter_map(|(n, v)| n.strip_prefix(&prefix).map(|f| Expr::Keyed(f.to_string(), Box::new(v.clone()))))
            .collect()
    };
    if let Expr::Ident(local, _) = &body {
        let fields = fields_of(local);
        if !fields.is_empty() {
            body = Expr::Pattern(fields);
        }
    }
    let result_fields = fields_of(name);
    if !result_fields.is_empty() {
        body = Expr::Pattern(result_fields);
    }
    Ok(body)
}

/// A function body kept for its calls: the statements, the result name and the return type. It runs with the arguments
/// bound to the parameters, at each call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvBody {
    pub params: Vec<String>,
    pub result: String,
    pub ret: DType,
    pub stmts: Vec<SvStmt>,
}

/// Whether a statement list assigns anything (a check or a message does not).
fn sv_assigns(stmts: &[SvStmt]) -> bool {
    stmts.iter().any(|s| match s {
        SvStmt::Assign(..) | SvStmt::PartAssign { .. } => true,
        SvStmt::If(_, a, b) => sv_assigns(a) || sv_assigns(b),
        SvStmt::For { body, .. } => sv_assigns(body),
        SvStmt::Width(..) => false,
    })
}

/// The value of a name in a function environment (its last assignment).
fn sv_get<'a>(env: &'a [(String, Expr)], name: &str) -> Option<&'a Expr> {
    env.iter().find(|(n, _)| n == name).map(|(_, e)| e)
}

fn sv_put(env: &mut Vec<(String, Expr)>, name: &str, value: Expr) {
    match env.iter_mut().find(|(n, _)| n == name) {
        Some(slot) => slot.1 = value,
        None => env.push((name.to_string(), value)),
    }
}

/// The integer value of an expression of constants (no parameter is known here), if it is one.
pub(crate) fn sv_const(e: &Expr, g: &Env) -> Option<i64> {
    // a sized literal (`4'd3`) is a value too
    match crate::elab::fold::fold_expr(e, g) {
        Expr::Num(text) => crate::lexer::number_value(&text),
        other => crate::elab::fold::int_of(&other),
    }
}

fn sv_num(v: i64) -> Expr {
    Expr::Num(v.to_string())
}

fn sv_remove(env: &mut Vec<(String, Expr)>, name: &str) {
    env.retain(|(n, _)| n != name);
}

/// The bits not yet assigned of a local, as a mask (bit 0 is the least significant bit of the local).
fn sv_unset(env: &[(String, Expr)], name: &str, g: &Env) -> i64 {
    sv_get(env, &format!("{name}#unset")).and_then(|v| sv_const(v, g)).unwrap_or(0)
}

/// The mask of the bits lo..=hi (at most 63 bits wide).
fn sv_mask(hi: i64, lo: i64) -> i64 {
    ((((1u64 << (hi - lo + 1)) - 1) << lo) & i64::MAX as u64) as i64
}

/// The segments of a local assigned in part, from its most significant bit down: (width, value).
fn sv_segments(env: &[(String, Expr)], name: &str, g: &Env) -> Option<Vec<(i64, Expr)>> {
    match sv_get(env, &format!("{name}#bits"))? {
        Expr::Concat(parts) => parts
            .iter()
            .map(|p| match p {
                Expr::Cast(w, e) => Some((sv_const(w, g)?, (**e).clone())),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// The segments of a concatenation whose parts have known widths (sized casts, or slices with constant bounds).
fn sv_concat_segments(e: &Expr, g: &Env) -> Option<Vec<(i64, Expr)>> {
    let Expr::Concat(parts) = e else { return None };
    parts
        .iter()
        .map(|p| match p {
            Expr::Cast(w, v) => Some((sv_const(w, g)?, (**v).clone())),
            Expr::Slice(_, msb, lsb, op) if op == ":" => Some((sv_const(msb, g)? - sv_const(lsb, g)? + 1, p.clone())),
            _ => None,
        })
        .collect()
}

/// The bits lo..=hi of segments covering a local (most significant first), each piece keeping its width. A segment cut
/// in two is sliced, which needs a name: an expression cannot be sliced in SystemVerilog.
fn sv_clip(segs: &[(i64, Expr)], lo: i64, hi: i64, g: &Env) -> std::result::Result<Vec<(i64, Expr)>, String> {
    let mut out = Vec::new();
    if lo > hi {
        return Ok(out);
    }
    let total: i64 = segs.iter().map(|(w, _)| w).sum();
    let mut top = total - 1;
    for (w, e) in segs {
        let bot = top - w + 1;
        let (a, b) = (bot.max(lo), top.min(hi));
        if a <= b {
            let piece = if a == bot && b == top { e.clone() } else { sv_piece(e, a - bot, b - bot, g)? };
            out.push((b - a + 1, piece));
        }
        top = bot - 1;
    }
    Ok(out)
}

/// The bits lo..=hi (relative to the value's bit 0) of a value, in SystemVerilog: a slice of a name, a zero, or the same
/// slice of both branches of a conditional, or of the parts of a concatenation.
fn sv_piece(e: &Expr, lo: i64, hi: i64, g: &Env) -> std::result::Result<Expr, String> {
    if sv_const(e, g) == Some(0) {
        return Ok(sv_num(0));
    }
    match e {
        Expr::Ident(..) => Ok(Expr::Slice(Box::new(e.clone()), Box::new(sv_num(hi)), Box::new(sv_num(lo)), ":".into())),
        Expr::Slice(base, _, l, op) if op == ":" => {
            let l0 = sv_const(l, g).ok_or("a part of a value is sliced at a bit that is not a constant")?;
            Ok(Expr::Slice(base.clone(), Box::new(sv_num(l0 + hi)), Box::new(sv_num(l0 + lo)), ":".into()))
        }
        Expr::Ternary(c, x, y) => Ok(Expr::Ternary(c.clone(), Box::new(sv_piece(x, lo, hi, g)?), Box::new(sv_piece(y, lo, hi, g)?))),
        Expr::Concat(_) => {
            let segs = sv_concat_segments(e, g).ok_or("a part of a concatenation has no known width")?;
            let pieces = sv_clip(&segs, lo, hi, g)?;
            Ok(Expr::Concat(pieces.into_iter().map(|(w, v)| Expr::Cast(Box::new(sv_num(w)), Box::new(v))).collect()))
        }
        _ => Err("a part of a value that is not a name is assigned in a function: not supported yet".into()),
    }
}

/// Runs the statements on the environment: an assignment sets its name, and an if runs both branches on copies and
/// keeps, for each name both branches set, a conditional expression. A name that only one branch sets is unknown after.
/// A for loop with constant bounds runs its body once per value, with the loop variable bound to it.
///
/// A local declared without a value has no bits assigned (`#unset` is its mask of unassigned bits); a part assignment
/// clears its bits and rebuilds the value from segments, so a bit is never read before it is assigned on the path.
pub(crate) fn sv_run(stmts: &[SvStmt], env: &mut Vec<(String, Expr)>, g: &Env) -> std::result::Result<(), String> {
    for s in stmts {
        match s {
            SvStmt::Width(name, w) => {
                let value = subst(w, env);
                if let Some(n) = sv_const(&value, g).filter(|n| (1..=63).contains(n)) {
                    sv_put(env, &format!("{name}#unset"), sv_num(sv_mask(n - 1, 0)));
                }
                sv_put(env, &format!("{name}#width"), value);
            }
            SvStmt::Assign(name, e) => {
                let value = subst(e, env);
                sv_put(env, name, value);
                sv_remove(env, &format!("{name}#unset"));
                sv_remove(env, &format!("{name}#bits"));
            }
            SvStmt::PartAssign { name, msb, lsb, value } => {
                let width = sv_get(env, &format!("{name}#width"))
                    .and_then(|v| sv_const(v, g))
                    .ok_or_else(|| format!("'{name}' is assigned in part but it is not a local of the function"))?;
                if width > 63 {
                    return Err(format!("a part of '{name}' is assigned, and it is wider than 63 bits: not supported yet"));
                }
                let msb = sv_const(&subst(msb, env), g).ok_or_else(|| format!("the bits of '{name}' are not constant"))?;
                let lsb = sv_const(&subst(lsb, env), g).ok_or_else(|| format!("the bits of '{name}' are not constant"))?;
                let (hi, lo) = (msb.max(lsb), msb.min(lsb));
                if lo < 0 || hi >= width {
                    return Err(format!("the bits of '{name}' are out of its width"));
                }
                let value = subst(value, env);
                let whole = sv_get(env, name).cloned().unwrap_or_else(|| sv_num(0));
                let segs = match sv_segments(env, name, g) {
                    Some(s) => s,
                    None => sv_concat_segments(&whole, g).unwrap_or_else(|| vec![(width, whole)]),
                };
                let mut new = sv_clip(&segs, hi + 1, width - 1, g)?;
                new.push((hi - lo + 1, value));
                new.extend(sv_clip(&segs, 0, lo - 1, g)?);
                let composite = Expr::Concat(
                    new.iter().map(|(w, e)| Expr::Cast(Box::new(sv_num(*w)), Box::new(e.clone()))).collect(),
                );
                sv_put(env, &format!("{name}#bits"), composite.clone());
                sv_put(env, name, composite);
                let unset = sv_unset(env, name, g) & !sv_mask(hi, lo);
                sv_put(env, &format!("{name}#unset"), sv_num(unset));
            }
            SvStmt::If(cond, then, els) => {
                let cond = subst(cond, env);
                let mut env_then = env.clone();
                sv_run(then, &mut env_then, g)?;
                let mut env_else = env.clone();
                sv_run(els, &mut env_else, g)?;
                let mut names: Vec<String> = Vec::new();
                for (n, _) in env_then.iter().chain(env_else.iter()) {
                    if !names.contains(n) {
                        names.push(n.clone());
                    }
                }
                for n in names {
                    if n.ends_with("#bits") {
                        // the segments of the branches differ: the value is the conditional one, rebuilt on a later part
                        sv_remove(env, &n);
                        continue;
                    }
                    if n.ends_with("#unset") {
                        // a bit is assigned after the if only if both branches assign it
                        let base = n.trim_end_matches("#unset");
                        let (a, b) = (sv_unset(&env_then, base, g), sv_unset(&env_else, base, g));
                        sv_put(env, &n, sv_num(a | b));
                        continue;
                    }
                    match (sv_get(&env_then, &n), sv_get(&env_else, &n)) {
                        (Some(a), Some(b)) if a == b => {
                            let value = a.clone();
                            sv_put(env, &n, value);
                        }
                        (Some(a), Some(b)) => {
                            let value = Expr::Ternary(Box::new(cond.clone()), Box::new(a.clone()), Box::new(b.clone()));
                            sv_put(env, &n, value);
                        }
                        _ => {}
                    }
                }
            }
            SvStmt::For { var, lo, cmp, bound, step, body } => {
                let mut v = sv_const(&subst(lo, env), g).ok_or_else(|| format!("the loop '{var}' does not start at a constant"))?;
                let mut turns = 0usize;
                loop {
                    sv_put(env, var, sv_num(v));
                    let limit = sv_const(&subst(bound, env), g).ok_or_else(|| format!("the bound of the loop '{var}' is not constant"))?;
                    let go = match cmp.as_str() {
                        "<" => v < limit,
                        "<=" => v <= limit,
                        ">" => v > limit,
                        ">=" => v >= limit,
                        "!=" => v != limit,
                        other => return Err(format!("the loop condition '{other}' is not supported")),
                    };
                    if !go {
                        break;
                    }
                    turns += 1;
                    if turns > 4096 {
                        return Err(format!("the loop '{var}' runs more than 4096 times"));
                    }
                    sv_run(body, env, g)?;
                    v = sv_const(&subst(step, env), g).ok_or_else(|| format!("the step of the loop '{var}' is not constant"))?;
                }
                // the loop variable is not a local after the loop: its declaration and unassigned bits go with it
                for suffix in ["", "#unset", "#width", "#bits"] {
                    sv_remove(env, &format!("{var}{suffix}"));
                }
            }
        }
    }
    Ok(())
}

impl Parser {
    /// A SystemVerilog function of a module: the header with its inputs in the parentheses, or with them declared after
    /// it, a body of blocking assignments and ifs, and endfunction. The body becomes one expression, which the calls
    /// inline. Anything else is refused, and its module part stays not analyzed.
    pub(super) fn parse_sv_function(&mut self) -> Result<ItemKind> {
        self.expect("function")?;
        if !self.accept("automatic") {
            self.accept("static");
        }
        // a return of dimensions alone (`function [15:0] f;`) is an implicit logic
        let ret = if self.at("[") { self.parse_dtype(true)? } else { self.parse_dtype(false)? };
        let name = self.ident()?;
        let typed: Vec<(DType, String)> = if self.accept("(") {
            let typed = self.comma_list(")", |p| {
                // a function has one result: an argument that is written back (output, inout, ref) is refused
                if p.at("output") || p.at("inout") || p.at("ref") {
                    return Err(p.fail("an output argument of a function is not supported: a function has one result"));
                }
                p.accept("input");
                let ty = p.parse_dtype(true)?;
                Ok((ty, p.ident()?))
            })?;
            self.expect(";")?;
            typed
        } else {
            self.expect(";")?;
            let mut typed = Vec::new();
            if self.at("output") || self.at("inout") || self.at("ref") {
                return Err(self.fail("an output argument of a function is not supported: a function has one result"));
            }
            while self.accept("input") {
                let ty = self.parse_dtype(true)?;
                loop {
                    typed.push((ty.clone(), self.ident()?));
                    if !self.accept(",") {
                        break;
                    }
                }
                self.expect(";")?;
            }
            typed
        };
        let (ptypes, params): (Vec<DType>, Vec<String>) = typed.into_iter().unzip();
        let mut stmts = Vec::new();
        while !self.at("endfunction") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'endfunction' missing"));
            }
            if self.accept("begin") {
                stmts.extend(self.parse_sv_block(&name)?);
            } else {
                stmts.extend(self.parse_sv_stmt(&name)?);
            }
        }
        self.expect("endfunction")?;
        // a void function has no value: when it only checks (assertions, messages), it changes nothing
        if ret.name == "void" {
            if sv_assigns(&stmts) {
                return Err(self.fail(format!("a void function '{name}' with assignments is not supported yet")));
            }
            return Ok(ItemKind::Comment(format!("function {name}: checks only")));
        }
        let ret = if ret.packed.is_empty() { None } else { Some(ret) };
        let globals = no_globals();
        let mut env = Vec::new();
        match sv_build(&stmts, &name, &mut env, &globals) {
            Ok(body) => Ok(ItemKind::Fn { name, params, ptypes, ret, body, override_at: None, sv: None }),
            // a bound or an index that is a parameter of the caller is known only at the call: the body is kept
            Err(m) if m.contains("not constant") => {
                let dtype = ret.clone().unwrap_or(DType { name: "logic".into(), targs: Vec::new(), signed: false, packed: Vec::new() });
                let sv = SvBody { params: params.clone(), result: name.clone(), ret: dtype, stmts };
                Ok(ItemKind::Fn { name, params, ptypes, ret, body: sv_num(0), override_at: None, sv: Some(sv) })
            }
            Err(m) => Err(self.fail(m)),
        }
    }

    /// The statements of a begin ... end block (the begin is consumed, and so is the end).
    fn parse_sv_block(&mut self, result: &str) -> Result<Vec<SvStmt>> {
        let mut stmts = Vec::new();
        while !self.accept("end") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'end' missing"));
            }
            if self.accept("begin") {
                stmts.extend(self.parse_sv_block(result)?);
            } else {
                stmts.extend(self.parse_sv_stmt(result)?);
            }
        }
        Ok(stmts)
    }

    /// The branch of an if: a block or one statement.
    fn parse_sv_branch(&mut self, result: &str) -> Result<Vec<SvStmt>> {
        if self.accept("begin") {
            return self.parse_sv_block(result);
        }
        self.parse_sv_stmt(result)
    }

    /// One statement: an assignment, a return (an assignment to the function name), an if, or a local declaration (which
    /// has no value until it is assigned).
    fn parse_sv_stmt(&mut self, result: &str) -> Result<Vec<SvStmt>> {
        // an empty statement (`default: ;`)
        if self.accept(";") {
            return Ok(Vec::new());
        }
        // a check of the simulation (`assert (...) [else $fatal(...)];`): it changes no value
        if self.at("assert") || self.at("assume") || self.at("cover") || self.tok().text.starts_with('$') {
            self.i += 1;
            let mut depth = 0i32;
            loop {
                if self.tok().kind == Kind::Eof {
                    return Err(self.fail("';' missing after an assertion"));
                }
                let text = self.tok().text.clone();
                self.i += 1;
                match text.as_str() {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    ";" if depth == 0 => break,
                    _ => {}
                }
            }
            return Ok(Vec::new());
        }
        // `unique case` / `priority case`: the same as `case`
        if (self.at("unique") || self.at("priority")) && self.peek(1).text == "case" {
            self.i += 1;
        }
        if self.accept("case") {
            return self.parse_sv_case(result);
        }
        if self.accept("for") {
            self.expect("(")?;
            while ["int", "integer", "genvar", "logic", "bit", "unsigned", "signed", "byte"].iter().any(|t| self.at(t)) {
                self.i += 1;
            }
            let var = self.ident()?;
            self.expect("=")?;
            let lo = self.parse_expr(0)?;
            self.expect(";")?;
            if self.ident()? != var {
                return Err(self.fail("the condition of a function's loop tests its variable"));
            }
            let cmp = self.tok().text.clone();
            self.i += 1;
            let bound = self.parse_expr(0)?;
            self.expect(";")?;
            if self.ident()? != var {
                return Err(self.fail("the step of a function's loop changes its variable"));
            }
            let step = if self.accept("++") {
                Expr::Binary("+".into(), Box::new(Expr::Ident(var.clone(), (0, 0))), Box::new(sv_num(1)), (0, 0))
            } else if self.accept("+=") {
                let k = self.parse_expr(0)?;
                Expr::Binary("+".into(), Box::new(Expr::Ident(var.clone(), (0, 0))), Box::new(k), (0, 0))
            } else {
                self.expect("=")?;
                self.parse_expr(0)?
            };
            self.expect(")")?;
            let body = if self.accept("begin") { self.parse_sv_block(result)? } else { self.parse_sv_stmt(result)? };
            return Ok(vec![SvStmt::For { var, lo, cmp, bound, step, body }]);
        }
        if self.accept("if") {
            self.expect("(")?;
            let cond = self.parse_expr(0)?;
            self.expect(")")?;
            let then = self.parse_sv_branch(result)?;
            let els = if self.accept("else") { self.parse_sv_branch(result)? } else { Vec::new() };
            return Ok(vec![SvStmt::If(cond, then, els)]);
        }
        if self.accept("return") {
            let e = self.parse_expr(0)?;
            self.expect(";")?;
            return Ok(vec![SvStmt::Assign(result.to_string(), e)]);
        }
        // a declaration: a type (a base type, or a package's type `pkg::t`) and names, each with a value or without one
        // `automatic` and `static` are lifetimes of a local: they change nothing here
        if self.at("automatic") || self.at("static") {
            self.i += 1;
        }
        let declares = self.at("logic") || self.at("reg") || self.at("int") || self.at("integer") || self.at("bit") || self.at("unsigned") || self.at("signed") || self.at("byte")
            || (self.tok().kind == Kind::Ident && (self.peek(1).kind == Kind::Ident || self.peek(1).text == "::"));
        if declares {
            let dtype = self.parse_dtype(true)?;
            let scalar = match dtype.name.as_str() {
                "logic" | "bit" | "reg" => Some(1),
                "byte" => Some(8),
                "shortint" => Some(16),
                "int" | "integer" => Some(32),
                "longint" => Some(64),
                _ => None,
            };
            let width = if dtype.packed.is_empty() && scalar.is_none() {
                None
            } else if dtype.packed.is_empty() {
                scalar.map(sv_num)
            } else {
              dtype.packed.iter().fold(None, |acc: Option<Expr>, (msb, lsb)| {
                let dim = Expr::Binary("+".into(), Box::new(Expr::Binary("-".into(), Box::new(msb.clone()), Box::new(lsb.clone()), (0, 0))), Box::new(sv_num(1)), (0, 0));
                Some(match acc {
                    None => dim,
                    Some(a) => Expr::Binary("*".into(), Box::new(a), Box::new(dim), (0, 0)),
                })
              })
            };
            let mut out = Vec::new();
            loop {
                let name = self.ident()?;
                if let Some(w) = &width {
                    out.push(SvStmt::Width(name.clone(), w.clone()));
                }
                if self.accept("=") {
                    let e = self.parse_expr(0)?;
                    out.push(SvStmt::Assign(name, e));
                }
                if !self.accept(",") {
                    break;
                }
            }
            self.expect(";")?;
            return Ok(out);
        }
        let mut target = self.ident()?;
        if self.accept("[") {
            let base = self.parse_expr(0)?;
            let (msb, lsb) = if self.accept("+:") {
                let w = self.parse_expr(0)?;
                let top = Expr::Binary("+".into(), Box::new(base.clone()), Box::new(Expr::Binary("-".into(), Box::new(w), Box::new(sv_num(1)), (0, 0))), (0, 0));
                (top, base)
            } else if self.accept(":") {
                (base, self.parse_expr(0)?)
            } else {
                (base.clone(), base)
            };
            self.expect("]")?;
            self.expect("=")?;
            let value = self.parse_expr(0)?;
            self.expect(";")?;
            return Ok(vec![SvStmt::PartAssign { name: target, msb, lsb, value }]);
        }
        if self.accept(".") {
            // a field of a struct local: `cfg.XLEN = e;`
            target = format!("{target}.{}", self.ident()?);
        }
        if !self.at("=") {
            return Err(self.fail(format!("a function statement is an assignment, an if or a declaration, not '{target}'")));
        }
        self.expect("=")?;
        let e = self.parse_expr(0)?;
        self.expect(";")?;
        Ok(vec![SvStmt::Assign(target, e)])
    }
}

impl Parser {
    /// `case (sel) [inside] label, label: stmt ... [default: stmt] endcase`, the statement after `case`. Each item is an
    /// if on the selector being equal to one of its labels, and the default is the else at the end.
    fn parse_sv_case(&mut self, result: &str) -> Result<Vec<SvStmt>> {
        self.expect("(")?;
        let sel = self.parse_expr(0)?;
        self.expect(")")?;
        self.accept("inside");
        let mut arms: Vec<(Vec<Expr>, Vec<SvStmt>)> = Vec::new();
        let mut default: Vec<SvStmt> = Vec::new();
        while !self.accept("endcase") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'endcase' missing"));
            }
            if self.accept("default") {
                self.accept(":");
                default = self.parse_sv_branch(result)?;
                continue;
            }
            let mut labels = vec![self.parse_sv_case_label(&sel)?];
            while self.accept(",") {
                labels.push(self.parse_sv_case_label(&sel)?);
            }
            self.expect(":")?;
            let body = self.parse_sv_branch(result)?;
            arms.push((labels, body));
        }
        let mut rest = default;
        for (labels, body) in arms.into_iter().rev() {
            let cond = labels
                .into_iter()
                .reduce(|a, b| Expr::Binary("||".into(), Box::new(a), Box::new(b), (0, 0)))
                .expect("a case item has a label");
            rest = vec![SvStmt::If(cond, body, rest)];
        }
        Ok(rest)
    }
}

impl Parser {
    /// One label of a case item, as the condition that the selector matches it: `sel == value`, or `lo <= sel <= hi` for
    /// a range `[lo : hi]`.
    fn parse_sv_case_label(&mut self, sel: &Expr) -> Result<Expr> {
        if self.accept("[") {
            let lo = self.parse_expr(0)?;
            self.expect(":")?;
            let hi = self.parse_expr(0)?;
            self.expect("]")?;
            let ge = Expr::Binary(">=".into(), Box::new(sel.clone()), Box::new(lo), (0, 0));
            let le = Expr::Binary("<=".into(), Box::new(sel.clone()), Box::new(hi), (0, 0));
            return Ok(Expr::Binary("&&".into(), Box::new(ge), Box::new(le), (0, 0)));
        }
        let value = self.parse_expr(0)?;
        Ok(Expr::Binary("==".into(), Box::new(sel.clone()), Box::new(value), (0, 0)))
    }
}

