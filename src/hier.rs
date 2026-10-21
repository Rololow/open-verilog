//! Connections across the hierarchy. `inst.sub.sig` reads a signal inside an instance at any depth, and
//! `assign inst.sub.in = v;` drives an input port. Real ports are threaded through every module on the path, so
//! intermediate modules are not edited by hand and the result is plain SystemVerilog:
//!
//! - reading: each module on the path gets an output `ov_tap_<path>` that carries the signal, each instance is
//!   connected to it, and the reference becomes a local wire;
//! - driving: the target must be an input port with a default (`input x = 0`). Every intermediate module gets an
//!   input `ov_drv_<path>` with the same default, so the other instances keep the default.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::ast::{Conn, Decl, Declarator, DType, Dir, Expr, Item, ItemKind, Module, Pos, Port};
use crate::elab::int_of;
use crate::error::{Error, Result};
use crate::pipeline::arrays::rewrite_item;
use crate::pipeline::rewrite::{map_expr, Failure};

/// One name of a path, with the indices applied to it: `c[2]` is `Seg { name: "c", idx: [2] }`.
#[derive(Clone, Debug)]
struct Seg {
    name: String,
    idx: Vec<i64>,
    pos: Pos,
}

impl Seg {
    /// The name as the elaborated instance or wire names it: `c[2]` is `c_2`.
    fn ident(&self) -> String {
        self.idx.iter().fold(self.name.clone(), |s, i| format!("{s}_{i}"))
    }
}

/// `a.b[2].c` as segments. `None` for anything else.
fn flatten(e: &Expr) -> Option<Vec<Seg>> {
    match e {
        Expr::Ident(name, pos) => Some(vec![Seg { name: name.clone(), idx: Vec::new(), pos: *pos }]),
        Expr::Member(base, name, pos) => {
            let mut segs = flatten(base)?;
            segs.push(Seg { name: name.clone(), idx: Vec::new(), pos: *pos });
            Some(segs)
        }
        Expr::Index(base, i) => {
            let mut segs = flatten(base)?;
            segs.last_mut()?.idx.push(int_of(i)?);
            Some(segs)
        }
        _ => None,
    }
}

fn key(segs: &[Seg]) -> String {
    segs.iter().map(|s| format!("{}{:?}", s.name, s.idx)).collect::<Vec<_>>().join(".")
}

fn logic(dt: &DType) -> DType {
    let mut dt = dt.clone();
    if dt.name == "wire" || dt.name == "reg" {
        dt.name = "logic".into();
    }
    dt
}

fn instances(items: &[Item]) -> HashMap<String, usize> {
    items.iter().enumerate().filter_map(|(k, it)| match &it.kind {
        ItemKind::Instance(i) => Some((i.name.clone(), k)),
        _ => None,
    }).collect()
}

/// Threads the hierarchical references of every module. Modules are changed in place.
pub fn thread(modules: &mut [Module], file: &str) -> Result<()> {
    let mut t = Threader { file, index: HashMap::new(), taps: HashMap::new(), wires: HashSet::new(), method_sites: HashMap::new() };
    for (k, m) in modules.iter().enumerate() {
        t.index.insert(m.name.clone(), k);
    }
    for m in 0..modules.len() {
        t.module(modules, m)?;
    }
    // the functions have been given their ports where they are called: they are not emitted
    for m in modules.iter_mut() {
        m.items.retain(|it| !matches!(it.kind, ItemKind::Fn { .. }));
    }
    Ok(())
}

struct Threader<'a> {
    file: &'a str,
    /// Module name -> index.
    index: HashMap<String, usize>,
    /// (module, path) -> the output port that carries it.
    taps: HashMap<(usize, String), String>,
    /// (module, wire) already declared for a read.
    wires: HashSet<(usize, String)>,
    /// (callee, method) -> the number of call sites given ports so far.
    method_sites: HashMap<(usize, String), usize>,
}

/// A call of a method of an instance: `inst.method(args)`.
fn method_call(e: &Expr, names: &HashMap<String, usize>) -> Option<(String, String, Vec<Expr>)> {
    let Expr::Call(f, args) = e else { return None };
    let Expr::Member(base, method, _) = &**f else { return None };
    let Expr::Ident(inst, _) = &**base else { return None };
    names.contains_key(inst).then(|| (inst.clone(), method.clone(), args.clone()))
}

impl Threader<'_> {
    fn err(&self, pos: Pos, msg: impl Into<String>) -> Error {
        Error::new(msg, self.file, pos.0, pos.1)
    }

    /// The instance named `seg` in module `m`: its position among the items, and the module it instantiates.
    fn instance(&self, ms: &[Module], m: usize, seg: &Seg) -> Result<(usize, usize)> {
        let name = seg.ident();
        let Some(&at) = instances(&ms[m].items).get(&name) else {
            return Err(self.err(seg.pos, format!("module '{}' has no instance '{name}'", ms[m].name)));
        };
        let ItemKind::Instance(inst) = &ms[m].items[at].kind else { unreachable!() };
        let Some(&child) = self.index.get(&inst.module) else {
            return Err(self.err(seg.pos, format!("'{name}' in '{}' is an external module: its signals cannot be reached", ms[m].name)));
        };
        Ok((at, child))
    }

    /// The declared type, dimensions and port (if a port) of a signal of module `m`.
    fn signal(&self, ms: &[Module], m: usize, name: &str, pos: Pos) -> Result<(DType, Vec<Expr>, Option<Port>)> {
        if let Some(p) = ms[m].ports.iter().find(|p| p.name == name) {
            return Ok((p.dtype.clone(), p.dims.clone(), Some(p.clone())));
        }
        for it in &ms[m].items {
            if let ItemKind::Decl(d) = &it.kind {
                if !d.is_const {
                    if let Some(x) = d.decls.iter().find(|x| x.name == name) {
                        return Ok((d.dtype.clone(), x.dims.clone(), None));
                    }
                }
            }
        }
        Err(self.err(pos, format!("module '{}' has no signal '{name}'", ms[m].name)))
    }

    /// Connects port `port` of the instance at item `at` of module `m` to `expr`.
    fn connect(&self, ms: &mut [Module], m: usize, at: usize, port: &str, expr: Expr, pos: Pos) -> Result<()> {
        let ItemKind::Instance(inst) = &mut ms[m].items[at].kind else { unreachable!() };
        if inst.conns.iter().any(|c| c.port.as_deref() == Some(port) && c.expr.is_some()) {
            return Err(self.err(pos, format!("port '{port}' of instance '{}' is already connected", inst.name)));
        }
        inst.conns.retain(|c| c.port.as_deref() != Some(port));
        inst.conns.push(Conn { port: Some(port.to_string()), expr: Some(expr) });
        Ok(())
    }

    fn existing(ms: &[Module], m: usize, at: usize, port: &str) -> Option<Expr> {
        let ItemKind::Instance(inst) = &ms[m].items[at].kind else { return None };
        inst.conns.iter().find(|c| c.port.as_deref() == Some(port)).and_then(|c| c.expr.clone())
    }

    fn port_on(ms: &mut [Module], m: usize, port: Port) {
        ms[m].ports.push(port);
    }

    /// The output port of module `m` that carries `path` (instances, then a signal). Adds it when needed.
    fn tap(&mut self, ms: &mut [Module], m: usize, path: &[Seg]) -> Result<String> {
        let k = (m, key(path));
        if let Some(name) = self.taps.get(&k) {
            return Ok(name.clone());
        }
        let name = if path.len() == 1 {
            let leaf = &path[0];
            let (dtype, dims, port) = self.signal(ms, m, &leaf.name, leaf.pos)?;
            if port.is_some_and(|p| p.dir == Dir::Output) {
                leaf.name.clone()
            } else {
                let name = format!("ov_tap_{}", leaf.name);
                Self::port_on(ms, m, Port { dir: Dir::Output, dtype: logic(&dtype), name: name.clone(), dims, cond: None, is_async: false, default: None, pos: leaf.pos });
                ms[m].items.push(Item { pos: leaf.pos, kind: ItemKind::ContAssign { lhs: Expr::Ident(name.clone(), leaf.pos), rhs: Expr::Ident(leaf.name.clone(), leaf.pos) } });
                name
            }
        } else {
            let (at, child) = self.instance(ms, m, &path[0])?;
            let inner = self.tap(ms, child, &path[1..])?;
            let port = ms[child].ports.iter().find(|p| p.name == inner).cloned().expect("tapped port exists");
            let name = format!("ov_tap_{}", path.iter().map(Seg::ident).collect::<Vec<_>>().join("_"));
            Self::port_on(ms, m, Port { dir: Dir::Output, dtype: logic(&port.dtype), name: name.clone(), dims: port.dims.clone(), cond: None, is_async: false, default: None, pos: path[0].pos });
            match Self::existing(ms, m, at, &inner) {
                Some(got) => ms[m].items.push(Item { pos: path[0].pos, kind: ItemKind::ContAssign { lhs: Expr::Ident(name.clone(), path[0].pos), rhs: got } }),
                None => self.connect(ms, m, at, &inner, Expr::Ident(name.clone(), path[0].pos), path[0].pos)?,
            }
            name
        };
        self.taps.insert(k, name.clone());
        Ok(name)
    }

    /// The expression that carries `segs` (a signal inside an instance) in module `m`.
    fn read(&mut self, ms: &mut [Module], m: usize, segs: &[Seg]) -> Result<Expr> {
        let (leaf, through) = segs.split_last().expect("a path has a signal");
        let (at, child) = self.instance(ms, m, &through[0])?;
        let mut inner_path: Vec<Seg> = through[1..].to_vec();
        inner_path.push(Seg { name: leaf.name.clone(), idx: Vec::new(), pos: leaf.pos });
        let inner = self.tap(ms, child, &inner_path)?;
        let wire = format!("ov_tap_{}", through.iter().map(Seg::ident).chain([leaf.name.clone()]).collect::<Vec<_>>().join("_"));
        let mut out = match Self::existing(ms, m, at, &inner) {
            Some(got) if !self.wires.contains(&(m, wire.clone())) => got,
            _ => {
                if self.wires.insert((m, wire.clone())) {
                    let port = ms[child].ports.iter().find(|p| p.name == inner).cloned().expect("tapped port exists");
                    let decl = crate::ast::Decl {
                        dtype: logic(&port.dtype),
                        decls: vec![crate::ast::Declarator { name: wire.clone(), dims: port.dims.clone(), init: None }],
                        is_const: false,
                        is_async: false,
                    };
                    ms[m].items.push(Item { pos: leaf.pos, kind: ItemKind::Decl(decl) });
                    self.connect(ms, m, at, &inner, Expr::Ident(wire.clone(), leaf.pos), leaf.pos)?;
                }
                Expr::Ident(wire, leaf.pos)
            }
        };
        for i in &leaf.idx {
            out = Expr::Index(Box::new(out), Box::new(Expr::Num(i.to_string())));
        }
        Ok(out)
    }

    /// `assign inst.sub.in = value;` in module `m`: the input is threaded down from `m`.
    fn drive(&mut self, ms: &mut [Module], m: usize, segs: &[Seg], value: Expr, pos: Pos) -> Result<()> {
        let (leaf, through) = segs.split_last().expect("a path has a signal");
        if !leaf.idx.is_empty() {
            return Err(self.err(pos, format!("'{}': drive a whole input port, not an element", leaf.name)));
        }
        // (module holding the instance, item of the instance) for each level, top first
        let mut chain = Vec::new();
        let mut cur = m;
        for seg in through {
            let (at, child) = self.instance(ms, cur, seg)?;
            chain.push((cur, at));
            cur = child;
        }
        let port = ms[cur].ports.iter().find(|p| p.name == leaf.name).cloned();
        let port = match port {
            Some(p) if p.dir == Dir::Input => p,
            _ => {
                return Err(self.err(leaf.pos, format!("'{}' can only be driven if it is an input port of '{}' (declare `input {} = <default>`)", leaf.name, ms[cur].name, leaf.name)));
            }
        };
        if port.default.is_none() && segs.len() > 2 {
            return Err(self.err(leaf.pos, format!("input '{}' of '{}' needs a default value to be driven from '{}' (`input {} = 0`): the other instances on the path keep it", leaf.name, ms[cur].name, ms[m].name, leaf.name)));
        }
        let mut target = leaf.name.clone();
        for depth in (1..chain.len()).rev() {
            let (parent, at) = chain[depth];
            let name = format!("ov_drv_{}", segs[depth..].iter().map(Seg::ident).collect::<Vec<_>>().join("_"));
            Self::port_on(ms, parent, Port { dir: Dir::Input, dtype: logic(&port.dtype), name: name.clone(), dims: port.dims.clone(), cond: None, is_async: false, default: port.default.clone(), pos });
            self.connect(ms, parent, at, &target, Expr::Ident(name.clone(), pos), pos)?;
            target = name;
        }
        let (top_module, top_at) = chain[0];
        debug_assert_eq!(top_module, m);
        self.connect(ms, m, top_at, &target, value, pos)
    }

    /// Threads the references of module `m`: reads become wires, drives become connections.
    fn module(&mut self, ms: &mut [Module], m: usize) -> Result<()> {
        // reads first, in the order they appear, so the ports they add come before the drives' ports
        let mut reads: Vec<(String, Vec<Seg>)> = Vec::new();
        let mut drives: Vec<(Vec<Seg>, Expr, Pos)> = Vec::new();
        let items = std::mem::take(&mut ms[m].items);
        let names = instances(&items);
        let is_hier = |segs: &[Seg]| segs.len() >= 2 && names.contains_key(&segs[0].ident());
        let found: RefCell<Vec<Vec<Seg>>> = RefCell::new(Vec::new());
        // a hierarchical chain is recorded and not searched further
        let collect_ref = |e: &Expr| -> std::result::Result<Option<Expr>, Failure> {
            match flatten(e) {
                Some(segs) if segs.len() >= 2 && names.contains_key(&segs[0].ident()) => {
                    found.borrow_mut().push(segs);
                    Ok(Some(e.clone()))
                }
                _ => Ok(None),
            }
        };
        let collect = |e: &Expr| -> std::result::Result<Option<Expr>, Failure> {
            // a method call (`rf.read(a)`) is not a signal: only its arguments can hold references
            if let Expr::Call(f, args) = e {
                if matches!(**f, Expr::Member(..)) {
                    for a in args {
                        let _ = map_expr(a, &mut |x| collect_ref(x));
                    }
                    return Ok(Some(e.clone()));
                }
            }
            collect_ref(e)
        };
        for it in &items {
            if let ItemKind::ContAssign { lhs, rhs } = &it.kind {
                if flatten(lhs).is_some_and(|s| is_hier(&s)) {
                    let _ = map_expr(rhs, &mut |e| collect(e));
                    continue;
                }
            }
            let _ = rewrite_item(it, &collect);
        }
        for segs in found.into_inner() {
            let k = key(&segs);
            if !reads.iter().any(|(rk, _)| *rk == k) {
                reads.push((k, segs));
            }
        }
        let mut mapping: HashMap<String, Expr> = HashMap::new();
        ms[m].items = items;
        for (k, segs) in &reads {
            let value = self.read(ms, m, segs)?;
            mapping.insert(k.clone(), value);
        }
        // method calls on instances: each call site gets ports on the callee, and a wire here for its result
        let names = instances(&ms[m].items);
        let sites: RefCell<Vec<(String, String, Vec<Expr>)>> = RefCell::new(Vec::new());
        for it in &ms[m].items {
            let _ = rewrite_item(it, &|e| {
                if let Some(call) = method_call(e, &names) {
                    sites.borrow_mut().push(call);
                }
                Ok(None)
            });
        }
        let mut new_decls = Vec::new();
        let mut site_wires: Vec<Expr> = Vec::new();
        for (inst, method, args) in sites.into_inner() {
            let seg = Seg { name: inst.clone(), idx: Vec::new(), pos: (0, 0) };
            let (at, child) = self.instance(ms, m, &seg)?;
            let pos = ms[m].items[at].pos;
            let callee = ms[child].items.iter().find_map(|it| match &it.kind {
                ItemKind::Fn { name, params, ptypes, ret, body } if *name == method => {
                    Some((params.clone(), ptypes.clone(), ret.clone(), body.clone()))
                }
                _ => None,
            });
            let Some((params, ptypes, ret, body)) = callee else {
                return Err(self.err(pos, format!("module '{}' has no method '{method}'", ms[child].name)));
            };
            if args.len() != params.len() {
                return Err(self.err(pos, format!("method '{method}' takes {} arguments, not {}", params.len(), args.len())));
            }
            let k = {
                let count = self.method_sites.entry((child, method.clone())).or_insert(0);
                *count += 1;
                *count - 1
            };
            let (stem, wire) = if params.is_empty() {
                (format!("ov_{method}"), format!("{inst}__{method}"))
            } else {
                (format!("ov_{method}_{k}"), format!("{inst}__{method}_{k}"))
            };
            let result = DType { name: "logic".into(), packed: ret.map(|r| r.packed).unwrap_or_default(), ..DType::default() };
            // the callee: an input for each argument and an output for the result, which the body drives
            // a method without arguments has one output, shared by every call site (the first one creates it)
            let inputs: Vec<String> = params.iter().map(|p| format!("{stem}_{p}")).collect();
            let shared = params.is_empty() && ms[child].ports.iter().any(|p| p.name == stem);
            if !shared {
            for (name, ptype) in inputs.iter().zip(&ptypes) {
                ms[child].ports.push(Port { dir: Dir::Input, dtype: logic(ptype), name: name.clone(), dims: Vec::new(), cond: None, is_async: false, default: None, pos });
            }
            ms[child].ports.push(Port { dir: Dir::Output, dtype: result.clone(), name: stem.clone(), dims: Vec::new(), cond: None, is_async: false, default: None, pos });
            let as_args: Vec<Expr> = inputs.iter().map(|n| Expr::Ident(n.clone(), pos)).collect();
            let value = crate::elab::substitute(&body, &params, &as_args);
            ms[child].items.push(Item { pos, kind: ItemKind::ContAssign { lhs: Expr::Ident(stem.clone(), pos), rhs: value } });
            }
            // the caller: a wire carries the result; the arguments connect to the inputs
            new_decls.push(Item {
                pos,
                kind: ItemKind::Decl(Decl { dtype: result, decls: vec![Declarator { name: wire.clone(), dims: Vec::new(), init: None }], is_const: false, is_async: false }),
            });
            for (name, arg) in inputs.iter().zip(&args) {
                self.connect(ms, m, at, name, replace(arg, &mapping), pos)?;
            }
            self.connect(ms, m, at, &stem, Expr::Ident(wire.clone(), pos), pos)?;
            site_wires.push(Expr::Ident(wire, pos));
        }
        ms[m].items.splice(0..0, new_decls);
        // the rewrite visits the call sites in the order they were collected
        let site = RefCell::new(0usize);
        let rewrite_one = |e: &Expr| -> Option<Expr> {
            if let Some(r) = replacement(e, &mapping) {
                return Some(r);
            }
            if method_call(e, &names).is_some() {
                let i = *site.borrow();
                *site.borrow_mut() += 1;
                return site_wires.get(i).cloned();
            }
            None
        };
        // rewrite the items: hierarchical reads are replaced, hierarchical drives are taken out for later
        let items = std::mem::take(&mut ms[m].items);
        let mut kept = Vec::with_capacity(items.len());
        for it in items {
            if let ItemKind::ContAssign { lhs, rhs } = &it.kind {
                if let Some(segs) = flatten(lhs).filter(|s| is_hier(s)) {
                    let rhs = map_expr(rhs, &mut |e| Ok(rewrite_one(e))).unwrap_or_else(|_| rhs.clone());
                    drives.push((segs, rhs, it.pos));
                    continue;
                }
            }
            let rewritten = rewrite_item(&it, &|e| Ok(rewrite_one(e))).map_err(|(at, msg)| self.err(if at.0 == 0 { it.pos } else { at }, msg))?;
            kept.push(rewritten);
        }
        ms[m].items = kept;
        for (segs, value, pos) in drives {
            self.drive(ms, m, &segs, value, pos)?;
        }
        // the declarations the threading added go with the others, before the logic
        let (decls, rest): (Vec<Item>, Vec<Item>) = std::mem::take(&mut ms[m].items).into_iter().partition(|i| matches!(i.kind, ItemKind::Decl(_)));
        ms[m].items = decls.into_iter().chain(rest).collect();
        Ok(())
    }
}

fn replacement(e: &Expr, mapping: &HashMap<String, Expr>) -> Option<Expr> {
    let segs = flatten(e)?;
    mapping.get(&key(&segs)).cloned()
}

/// `e` with the hierarchical chains of `mapping` replaced. The resolver never fails.
fn replace(e: &Expr, mapping: &HashMap<String, Expr>) -> Expr {
    map_expr(e, &mut |x| Ok(replacement(x, mapping))).unwrap_or_else(|_| e.clone())
}
