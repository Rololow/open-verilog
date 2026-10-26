//! Clock domains: the check that makes a crossing between two clock domains an error (port of `ovc/domains.py`).
//!
//! Every register belongs to the domain of its clock; a combinational signal belongs to the domains it reads. A
//! register that reads a signal of another domain, or an instance input fed from another domain than the one the
//! instance uses it in, is a clock-domain crossing: an error, unless the value enters through an `async` input port
//! (a synchronizer, an asynchronous FIFO) or the two clocks are declared related (`same_domain a, b;`).
//!
//! Nothing has to be annotated in the common case: the domain of an input port is inferred from the registers that
//! read it, and checked where the module is instantiated. Each module gets a summary in terms of its own ports, so the
//! check is modular: the modules are checked children first. The reset uses are checked the same way: a reset that is
//! asynchronous in one place and synchronous in another, or active high in one place and low in another, is a warning.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ast::{Conn, Dir, Expr, Instance, ItemKind, Module, Port, Pos, Sens, Stmt};
use crate::error::{Error, Result};

/// A clock token: a clock input port of the module, a clock generated inside it, the value of an input port (for a
/// combinational path), or any domain (`Async`: a value that enters through an `async` port).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Tok {
    Async,
    Port(String),
    Int(String),
    In(String),
}

/// A use of a reset: where it is (the module and the position, or a path below it through an instance).
#[derive(Debug, Clone)]
struct Where {
    /// The name of the module of a direct use, or the path of the instance that makes a use below.
    label: String,
    /// `None` for a direct use; the module of the use for one made below.
    module: Option<String>,
    pos: Pos,
}

impl Where {
    fn show(&self) -> String {
        match &self.module {
            Some(m) => format!("{} ({m})", self.label),
            None => self.label.clone(),
        }
    }
}

/// What a module says of its ports to the modules that instantiate it.
#[derive(Debug, Default, Clone)]
pub struct Summary {
    /// Input port -> the clock it is used in (`None`: unconstrained; `Async`: an `async` port).
    inputs: BTreeMap<String, Option<Tok>>,
    /// Output port -> the domains its value belongs to.
    outputs: BTreeMap<String, BTreeSet<Tok>>,
    /// Input ports used as clocks.
    clocks: BTreeSet<String>,
    /// Input port -> its reset uses: (kind `async` / `sync`, active `high` / `low`) -> where.
    resets: BTreeMap<String, BTreeMap<(String, String), Where>>,
}

/// Checks every module (children before parents) and returns the warnings, or the first error.
pub fn check(modules: &[Module], handshakes: &HashMap<String, Vec<(String, Dir, String)>>) -> Result<Vec<String>> {
    let mut summaries: HashMap<String, Summary> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();
    for m in modules {
        let empty = Vec::new();
        let summary = check_module(m, handshakes.get(&m.name).unwrap_or(&empty), &summaries, &mut warnings)?;
        summaries.insert(m.name.clone(), summary);
    }
    Ok(warnings)
}

fn token_name(t: &Tok) -> String {
    match t {
        Tok::Async => "async".into(),
        Tok::In(q) => format!("the domain of input '{q}'"),
        Tok::Port(n) | Tok::Int(n) => format!("'{n}'"),
    }
}

/// The name of a clock token: `clk` for `Port("clk")`.
fn clock_name(t: &Tok) -> String {
    match t {
        Tok::Port(n) | Tok::Int(n) | Tok::In(n) => n.clone(),
        Tok::Async => "async".into(),
    }
}

/// The name of the signal at the root of an lvalue: `x`, `x[i]`, `x[a:b]`, `x.f`.
fn base_name(e: &Expr) -> Option<String> {
    let mut e = e;
    loop {
        match e {
            Expr::Index(b, _) | Expr::Slice(b, _, _, _) | Expr::Member(b, _, _) => e = b,
            Expr::Ident(n, _) => return Some(n.clone()),
            _ => return None,
        }
    }
}

/// The lvalues inside a (possibly concatenated) assignment target: `{a, b[3:0]}` is `a` and `b[3:0]`.
fn lvalue_parts(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::Concat(parts) => parts.iter().flat_map(lvalue_parts).collect(),
        _ => vec![e],
    }
}

/// The names an expression reads (identifiers, including those in indices).
fn reads(e: &Expr, out: &mut BTreeSet<String>) {
    match e {
        Expr::Ident(n, _) => {
            out.insert(n.clone());
        }
        Expr::Num(_) | Expr::OpRef(_) | Expr::Type(_) => {}
        Expr::Unary(_, a) | Expr::Member(a, _, _) | Expr::Keyed(_, a) => reads(a, out),
        Expr::Binary(_, a, b, _) | Expr::Index(a, b) | Expr::Cast(a, b) => {
            reads(a, out);
            reads(b, out);
        }
        Expr::Ternary(c, a, b) => {
            reads(c, out);
            reads(a, out);
            reads(b, out);
        }
        Expr::Slice(a, b, c, _) => {
            reads(a, out);
            reads(b, out);
            reads(c, out);
        }
        Expr::Call(f, args) => {
            reads(f, out);
            args.iter().for_each(|x| reads(x, out));
        }
        Expr::Concat(v) | Expr::List(v) | Expr::Pattern(v) => v.iter().for_each(|x| reads(x, out)),
        Expr::Repl(n, v) => {
            reads(n, out);
            v.iter().for_each(|x| reads(x, out));
        }
        Expr::Lambda(_, body) => reads(body, out),
        Expr::InSet(e, items) => {
            reads(e, out);
            items.iter().for_each(|i| {
                reads(&i.lo, out);
                if let Some(hi) = &i.hi {
                    reads(hi, out);
                }
            });
        }
        Expr::Match(e, arms) => {
            reads(e, out);
            for (labels, value) in arms {
                for i in labels.iter().flat_map(|items| items.iter()) {
                    reads(&i.lo, out);
                    if let Some(hi) = &i.hi {
                        reads(hi, out);
                    }
                }
                reads(value, out);
            }
        }
        Expr::With(base, fields, _) => {
            reads(base, out);
            fields.iter().for_each(|(_, v)| reads(v, out));
        }
    }
}

fn reads_of(e: &Expr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    reads(e, &mut out);
    out
}

/// The (targets, names read) of every assignment of a statement. The conditions around an assignment are read too.
fn assignments(s: &Stmt, conds: &BTreeSet<String>, out: &mut Vec<(BTreeSet<String>, BTreeSet<String>)>) {
    match s {
        Stmt::Assign { lhs, rhs, .. } => {
            let targets: BTreeSet<String> = lvalue_parts(lhs).into_iter().filter_map(base_name).collect();
            let mut index_reads = BTreeSet::new();
            for part in lvalue_parts(lhs) {
                let mut p = part;
                loop {
                    match p {
                        Expr::Index(b, idx) => {
                            reads(idx, &mut index_reads);
                            p = b;
                        }
                        Expr::Slice(b, msb, lsb, _) => {
                            reads(msb, &mut index_reads);
                            reads(lsb, &mut index_reads);
                            p = b;
                        }
                        Expr::Member(b, _, _) => p = b,
                        _ => break,
                    }
                }
            }
            let mut r = reads_of(rhs);
            r.extend(index_reads);
            r.extend(conds.iter().cloned());
            out.push((targets, r));
        }
        Stmt::Block(stmts) => stmts.iter().for_each(|x| assignments(x, conds, out)),
        Stmt::If { cond, then, els } => {
            let mut c = conds.clone();
            reads(cond, &mut c);
            assignments(then, &c, out);
            if let Some(e) = els {
                assignments(e, &c, out);
            }
        }
        Stmt::Case { subject, arms, .. } => {
            let mut c = conds.clone();
            reads(subject, &mut c);
            for (labels, _) in arms {
                for label in labels.iter().flatten() {
                    reads(label, &mut c);
                }
            }
            for (_, body) in arms {
                assignments(body, &c, out);
            }
        }
        Stmt::For { header, body } => {
            let mut c = conds.clone();
            reads(&header.lo, &mut c);
            if let Some(bound) = &header.bound {
                reads(bound, &mut c);
            }
            assignments(body, &c, out);
        }
        _ => {}
    }
}

/// `(name, active)` of the reset tested first in a clocked block: `if (rst)`, `if (!rst)`, `if (rst == 0)`, ...
fn reset_condition(body: &Stmt) -> Option<(String, String)> {
    let mut s = body;
    while let Stmt::Block(stmts) = s {
        match stmts.first() {
            Some(first) => s = first,
            None => break,
        }
    }
    let Stmt::If { cond, .. } = s else { return None };
    let (mut c, mut low) = (cond, false);
    while let Expr::Unary(op, arg) = c {
        if op != "!" && op != "~" {
            break;
        }
        c = arg.as_ref();
        low = !low;
    }
    if let Expr::Binary(op, lhs, rhs, _) = c {
        if op == "==" || op == "!=" {
            if let Expr::Num(text) = &**rhs {
                if ["0", "1'b0", "1", "1'b1"].contains(&text.as_str()) {
                    if !low {
                        low = (text == "0" || text == "1'b0") != (op == "!=");
                    }
                    c = lhs.as_ref();
                }
            }
        }
    }
    match c {
        Expr::Ident(name, _) => Some((name.clone(), if low { "low".into() } else { "high".into() })),
        _ => None,
    }
}

/// An instance of a module that has a summary: its connections, and the clocks of the child mapped to ours.
struct InstMap<'s> {
    inst: &'s Instance,
    summary: &'s Summary,
    conns: HashMap<String, &'s Expr>,
    cmap: HashMap<String, Tok>,
}

/// The state of the check of one module.
struct Checker<'m> {
    module: &'m Module,
    signals: BTreeSet<String>,
    dom: BTreeMap<String, BTreeSet<Tok>>,
    related: HashMap<Tok, Tok>,
    /// Input port -> the clock tokens it is used in, with where (in the order they were found).
    need: BTreeMap<String, Vec<(Tok, String)>>,
}

impl Checker<'_> {
    /// The representative of a token in its `same_domain` class.
    fn find(&self, t: &Tok) -> Tok {
        let mut t = t.clone();
        while let Some(next) = self.related.get(&t) {
            if *next == t {
                break;
            }
            t = next.clone();
        }
        t
    }

    /// The domains of the signals named (only the signals of the module count).
    fn domain_of<'a>(&self, names: impl IntoIterator<Item = &'a String>) -> BTreeSet<Tok> {
        let mut out = BTreeSet::new();
        for n in names {
            if self.signals.contains(n) {
                if let Some(d) = self.dom.get(n) {
                    out.extend(d.iter().cloned());
                }
            }
        }
        out.into_iter().map(|t| if matches!(t, Tok::Port(_) | Tok::Int(_)) { self.find(&t) } else { t }).collect()
    }

    /// The tokens of a child's token, seen from here through the connections of its instance.
    fn map_token(&self, t: &Tok, im: &InstMap) -> BTreeSet<Tok> {
        match t {
            Tok::Async => BTreeSet::from([Tok::Async]),
            Tok::Port(p) => BTreeSet::from([im.cmap.get(p).cloned().unwrap_or_else(|| Tok::Int(format!("{}.{p}", im.inst.name)))]),
            Tok::In(q) => match im.conns.get(q) {
                Some(e) => self.domain_of(&reads_of(e)),
                None => BTreeSet::new(),
            },
            Tok::Int(n) => BTreeSet::from([Tok::Int(format!("{}.{n}", im.inst.name))]),
        }
    }

    /// A value read in the domain `tok`: a crossing unless it is in that domain, or is an `async` value.
    fn use_in(&mut self, names: &BTreeSet<String>, tok: &Tok, place: &str, pos: Pos) -> Result<()> {
        for n in names {
            if !self.signals.contains(n) {
                continue;
            }
            let Some(doms) = self.dom.get(n).cloned() else { continue };
            for d in doms {
                let d = if matches!(d, Tok::Port(_) | Tok::Int(_)) { self.find(&d) } else { d };
                if d == *tok || d == Tok::Async {
                    continue;
                }
                if let Tok::In(q) = &d {
                    let uses = self.need.entry(q.clone()).or_default();
                    if !uses.iter().any(|(t, _)| t == tok) {
                        uses.push((tok.clone(), place.to_string()));
                    }
                    continue;
                }
                return Err(self.error(
                    format!(
                        "clock-domain crossing in module '{}': '{n}' (domain {}) is read by {place} (domain {}). Pass it \
                         through a synchronizer (an `async` input, e.g. Sync or an asynchronous FIFO) or, if the clocks \
                         are synchronous, declare `same_domain {}, {};`",
                        self.module.name,
                        token_name(&d),
                        token_name(tok),
                        clock_name(&d),
                        clock_name(tok)
                    ),
                    pos,
                ));
            }
        }
        Ok(())
    }

    fn error(&self, message: String, pos: Pos) -> Error {
        Error::new(message, &self.module.origin, pos.0, pos.1)
    }
}

/// The domain summary of one module, after its checks. Errors stop at the first one; warnings are collected.
fn check_module(
    m: &Module,
    handshakes: &[(String, Dir, String)],
    summaries: &HashMap<String, Summary>,
    warnings: &mut Vec<String>,
) -> Result<Summary> {
    let ports: BTreeMap<&str, &Port> = m.ports.iter().map(|p| (p.name.as_str(), p)).collect();
    let mut signals: BTreeSet<String> = m.ports.iter().map(|p| p.name.clone()).collect();
    for it in &m.items {
        if let ItemKind::Decl(d) = &it.kind {
            if !d.is_const {
                signals.extend(d.decls.iter().map(|x| x.name.clone()));
            }
        }
    }
    // `assign clk2 = clk1;` makes two names one clock
    let mut alias: HashMap<String, String> = HashMap::new();
    for it in &m.items {
        if let ItemKind::ContAssign { lhs: Expr::Ident(l, _), rhs: Expr::Ident(r, _) } = &it.kind {
            alias.insert(l.clone(), r.clone());
        }
    }
    let mut ck = Checker { module: m, signals, dom: BTreeMap::new(), related: HashMap::new(), need: BTreeMap::new() };
    let clock_token = |ck: &Checker, sig: &str| -> Tok {
        let mut sig = sig.to_string();
        let mut seen = BTreeSet::new();
        while let Some(next) = alias.get(&sig) {
            if !seen.insert(sig.clone()) {
                break;
            }
            sig = next.clone();
        }
        match ports.get(sig.as_str()) {
            Some(p) if p.dir == Dir::Input => ck.find(&Tok::Port(sig.clone())),
            _ => ck.find(&Tok::Int(sig.clone())),
        }
    };
    // `same_domain a, b;`: synchronous clocks, one class
    for it in &m.items {
        if let ItemKind::SameDomain(names) = &it.kind {
            let toks: Vec<Tok> = names.iter().map(|n| clock_token(&ck, n)).collect();
            for t in toks.iter().skip(1) {
                let root = ck.find(t);
                let target = ck.find(&toks[0]);
                ck.related.insert(root, target);
            }
        }
    }
    // the domains of the signals that are given one: async declarations, and the input ports
    for it in &m.items {
        if let ItemKind::Decl(d) = &it.kind {
            if d.is_async {
                for x in &d.decls {
                    ck.dom.insert(x.name.clone(), BTreeSet::from([Tok::Async]));
                }
            }
        }
    }
    for p in &m.ports {
        if p.dir == Dir::Input {
            let t = if p.is_async { Tok::Async } else { Tok::In(p.name.clone()) };
            ck.dom.insert(p.name.clone(), BTreeSet::from([t]));
        }
    }
    let mut summary = Summary::default();
    // sources: combinational (targets, reads); registers (targets, reads, clock, position)
    let mut comb: Vec<(BTreeSet<String>, BTreeSet<String>)> = Vec::new();
    let mut regs: Vec<(BTreeSet<String>, BTreeSet<String>, Tok, Pos)> = Vec::new();
    let mut insts: Vec<&Instance> = Vec::new();
    // reset uses of each signal: signal -> (kind, active) -> where (the first one found wins)
    let mut uses: BTreeMap<String, BTreeMap<(String, String), Where>> = BTreeMap::new();
    // a specialised module (`Ram__32_512`) is named as its source (`Ram`), as the reset messages show it
    let source_name = m.name.split("__").next().unwrap_or(&m.name).to_string();
    let here = |pos: Pos| Where { label: source_name.clone(), module: None, pos };
    for it in &m.items {
        match &it.kind {
            ItemKind::ContAssign { lhs, rhs } => {
                let mut out = Vec::new();
                assignments(&Stmt::Assign { lhs: lhs.clone(), rhs: rhs.clone(), nonblocking: false }, &BTreeSet::new(), &mut out);
                comb.extend(out);
            }
            ItemKind::Decl(d) => {
                for x in &d.decls {
                    if let (Some(init), false) = (&x.init, d.is_const) {
                        comb.push((BTreeSet::from([x.name.clone()]), reads_of(init)));
                    }
                }
            }
            ItemKind::AlwaysComb(body) => assignments(body, &BTreeSet::new(), &mut comb),
            ItemKind::Always { sens: Sens::Comb, body, .. } => assignments(body, &BTreeSet::new(), &mut comb),
            ItemKind::Always { sens: Sens::Edges(edges), body, .. } if !edges.is_empty() => {
                let Some(clk) = base_name(&edges[0].1) else { continue };
                let tok = clock_token(&ck, &clk);
                if let Tok::Port(p) = &tok {
                    summary.clocks.insert(p.clone());
                }
                let skip: BTreeSet<String> = edges.iter().filter_map(|(_, e)| base_name(e)).collect();
                let mut out = Vec::new();
                assignments(body, &BTreeSet::new(), &mut out);
                for (targets, r) in out {
                    let r: BTreeSet<String> = r.difference(&skip).cloned().collect();
                    regs.push((targets, r, tok.clone(), it.pos));
                }
                // asynchronous resets: the other edges of the sensitivity list
                for (edge, e) in edges.iter().skip(1) {
                    if let Some(n) = base_name(e) {
                        let active = if edge == "negedge" { "low" } else { "high" };
                        uses.entry(n).or_default().entry(("async".into(), active.into())).or_insert_with(|| here(it.pos));
                    }
                }
                // a synchronous reset: the first test of the block
                if edges.len() == 1 {
                    if let Some((name, active)) = reset_condition(body) {
                        if ck.signals.contains(&name) && name != clk {
                            uses.entry(name).or_default().entry(("sync".into(), active)).or_insert_with(|| here(it.pos));
                        }
                    }
                }
            }
            ItemKind::Instance(inst) => insts.push(inst),
            _ => {}
        }
    }
    // the children that have a summary, with their clocks mapped to ours
    let mut inst_maps: Vec<InstMap> = Vec::new();
    for inst in insts {
        let Some(s) = summaries.get(&inst.module) else { continue };
        let conns: HashMap<String, &Expr> = inst
            .conns
            .iter()
            .filter_map(|Conn { port, expr }| match (port, expr) {
                (Some(p), Some(e)) if p != "*" => Some((p.clone(), e)),
                _ => None,
            })
            .collect();
        let mut cmap = HashMap::new();
        for c in &s.clocks {
            if let Some(Expr::Ident(name, _)) = conns.get(c).copied() {
                let tok = clock_token(&ck, name);
                if let Tok::Port(p) = &tok {
                    summary.clocks.insert(p.clone());
                }
                cmap.insert(c.clone(), tok);
            }
        }
        inst_maps.push(InstMap { inst, summary: s, conns, cmap });
    }
    // the domains of the signals driven by combinational paths and registers, through the children, to a fixpoint
    let fixed: BTreeSet<String> = ck.dom.iter().filter(|(_, d)| d.len() == 1 && d.contains(&Tok::Async)).map(|(n, _)| n.clone()).collect();
    for _ in 0..64 {
        let mut changed = false;
        let add = |ck: &mut Checker, targets: &BTreeSet<String>, ds: BTreeSet<Tok>, changed: &mut bool| {
            for t in targets.difference(&fixed) {
                let cur = ck.dom.entry(t.clone()).or_default();
                if !ds.is_subset(cur) {
                    cur.extend(ds.iter().cloned());
                    *changed = true;
                }
            }
        };
        for (targets, r) in &comb {
            let ds = ck.domain_of(r.iter());
            add(&mut ck, targets, ds, &mut changed);
        }
        for (targets, _, tok, _) in &regs {
            add(&mut ck, targets, BTreeSet::from([tok.clone()]), &mut changed);
        }
        for im in &inst_maps {
            for (p, toks) in &im.summary.outputs {
                if let Some(e) = im.conns.get(p) {
                    let mut ds = BTreeSet::new();
                    for t in toks {
                        ds.extend(ck.map_token(t, im));
                    }
                    let targets: BTreeSet<String> = lvalue_parts(e).into_iter().filter_map(base_name).collect();
                    add(&mut ck, &targets, ds, &mut changed);
                }
            }
        }
        if !changed {
            break;
        }
    }
    // the uses: a value read in a domain other than its own is a crossing, unless it is `async`
    for (targets, r, tok, pos) in &regs {
        let place = format!(
            "register{} {}",
            if targets.len() > 1 { "s" } else { "" },
            targets.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        ck.use_in(r, tok, &place, *pos)?;
    }
    for im in &inst_maps {
        for (q, req) in &im.summary.inputs {
            let Some(req) = req else { continue };
            if *req == Tok::Async || !im.conns.contains_key(q) {
                continue;
            }
            let Some(tok) = ck.map_token(req, im).into_iter().next() else { continue };
            let names = reads_of(im.conns[q]);
            let place = format!("input '{q}' of instance '{}'", im.inst.name);
            ck.use_in(&names, &tok, &place, im.inst.pos)?;
        }
    }
    // a producer offers its value without waiting for ready: otherwise two such modules wait for each other forever
    for (port, role, shown) in handshakes {
        let valid = format!("{port}_valid");
        if *role == Dir::Master && ck.dom.get(&valid).is_some_and(|d| d.contains(&Tok::In(format!("{port}_ready")))) {
            return Err(ck.error(
                format!(
                    "module '{}': {shown}.valid depends combinationally on {shown}.ready. A producer must offer its value \
                     without waiting for ready (otherwise two such modules wait for each other forever); compute valid \
                     from registers and inputs other than ready",
                    m.name
                ),
                m.pos,
            ));
        }
    }
    // the summary of the ports
    for p in &m.ports {
        if p.dir == Dir::Input {
            if p.is_async {
                summary.inputs.insert(p.name.clone(), Some(Tok::Async));
                continue;
            }
            let req = ck.need.get(&p.name).cloned().unwrap_or_default();
            if req.len() > 1 {
                let ((t1, w1), (t2, w2)) = (&req[0], &req[1]);
                return Err(ck.error(
                    format!(
                        "clock-domain crossing in module '{}': input '{}' is used in domain {} ({w1}) and in domain {} \
                         ({w2}). Declare it `input async` and synchronize it, or split it",
                        m.name,
                        p.name,
                        token_name(t1),
                        token_name(t2)
                    ),
                    m.pos,
                ));
            }
            summary.inputs.insert(p.name.clone(), req.first().map(|(t, _)| t.clone()));
        } else {
            summary.outputs.insert(p.name.clone(), ck.dom.get(&p.name).cloned().unwrap_or_default());
        }
    }
    check_resets(m, &ports, &uses, &inst_maps, &mut summary, warnings);
    Ok(summary)
}

/// The reset uses of a module and of its children (through the connections): a reset that is asynchronous in one place
/// and synchronous in another, or active high in one place and low in another, is reported once, where the uses meet.
fn check_resets(
    m: &Module,
    ports: &BTreeMap<&str, &Port>,
    uses: &BTreeMap<String, BTreeMap<(String, String), Where>>,
    inst_maps: &[InstMap],
    summary: &mut Summary,
    warnings: &mut Vec<String>,
) {
    let mut uses = uses.clone();
    let mut inherited: BTreeSet<String> = BTreeSet::new();
    for im in inst_maps {
        for (q, kinds) in &im.summary.resets {
            let Some(e) = im.conns.get(q) else { continue };
            let flip = matches!(e, Expr::Unary(op, _) if op == "!" || op == "~");
            for n in reads_of(e) {
                let kinds_set: BTreeSet<&String> = kinds.keys().map(|(k, _)| k).collect();
                let actives: BTreeSet<&String> = kinds.keys().map(|(_, a)| a).collect();
                if kinds_set.len() > 1 || actives.len() > 1 {
                    inherited.insert(n.clone());
                }
                for ((kind, active), w) in kinds {
                    let active = if flip { if active == "high" { "low" } else { "high" } } else { active.as_str() };
                    let label = if w.label.is_empty() { im.inst.name.clone() } else { format!("{}.{}", im.inst.name, w.label) };
                    uses.entry(n.clone()).or_default().entry((kind.clone(), active.to_string())).or_insert_with(|| Where {
                        label,
                        module: w.module.clone(),
                        pos: im.inst.pos,
                    });
                }
            }
        }
    }
    let mut report = |msg: String, pos: Pos| {
        let text = format!("{}:{}:{}: warning: {msg}", m.origin, pos.0, pos.1);
        if !warnings.contains(&text) {
            warnings.push(text);
        }
    };
    for (n, kinds) in &uses {
        // the node a warning is reported at: the first direct use, else the module
        let pos = kinds.values().find(|w| w.module.is_none()).map(|w| w.pos).unwrap_or(m.pos);
        if !inherited.contains(n) {
            let has_async = kinds.keys().any(|(k, _)| k == "async");
            let has_sync = kinds.keys().any(|(k, _)| k == "sync");
            if has_async && has_sync {
                let a = kinds.iter().find(|((k, _), _)| k == "async").map(|(_, w)| w.show()).unwrap_or_default();
                let b = kinds.iter().find(|((k, _), _)| k == "sync").map(|(_, w)| w.show()).unwrap_or_default();
                report(
                    format!(
                        "reset '{n}' is asynchronous in {a} but synchronous in {b}: its release is not timed the same way \
                         everywhere; release it through a reset synchronizer and use one style"
                    ),
                    pos,
                );
            }
            let actives: BTreeSet<&String> = kinds.keys().map(|(_, a)| a).collect();
            if actives.len() == 2 {
                let hi = kinds.iter().find(|((_, a), _)| a == "high").map(|(_, w)| w.show()).unwrap_or_default();
                let lo = kinds.iter().find(|((_, a), _)| a == "low").map(|(_, w)| w.show()).unwrap_or_default();
                if hi != lo {
                    report(format!("reset '{n}' is active high in {hi} but active low in {lo}"), pos);
                } else {
                    report(format!("reset '{n}' is tested both active high and active low in {hi}"), pos);
                }
            }
        }
        if ports.get(n.as_str()).is_some_and(|p| p.dir == Dir::Input) {
            // what the parents see: a direct use is `("", module)`, a use below keeps its path
            let seen = kinds
                .iter()
                .map(|(k, w)| {
                    let w = match &w.module {
                        None => Where { label: String::new(), module: Some(w.label.clone()), pos: w.pos },
                        Some(_) => w.clone(),
                    };
                    (k.clone(), w)
                })
                .collect();
            summary.resets.insert(n.clone(), seen);
        }
    }
}
