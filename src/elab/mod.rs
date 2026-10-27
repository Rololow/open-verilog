//! Elaboration: binds parameters, folds constants, unrolls constant generate loops and specialises
//! parameterised modules per instantiation (`Reg8<8>` becomes `Reg8__8`). The result has no parameters,
//! no generate constructs and only constant dimensions.

pub mod env;
pub(crate) mod fold;
mod fsm;
mod types;
pub(crate) mod width;
mod bundle;
mod vecmerge;
mod scope;
mod config;
pub mod isa;


use crate::pipeline;
use crate::pipeline::arrays::rewrite_item;

use std::collections::{HashMap, HashSet};

use crate::ast::{Arg, Conn, Decl, Declarator, DType, Dir, EnumDef, Expr, File, ForHeader, Instance, Item, ItemKind, Module, Param, Pos, Port, Sens, Stmt, StructDef};
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use env::{BundleRef, EnumInfo, FnDef, ResetSpec, StructInfo};
use crate::error::{Error, Result};
pub use env::Env;
use fold::{fold_dtype, fold_stmt};
pub use fold::{fold_expr, loop_values};
pub(crate) use fold::{int_of, substitute};

/// The structs: the width of each (the sum of its fields) and the position of each field, the first field being
/// the most significant.
fn build_structs(defs: &[StructDef], env: &Env, file: &str) -> Result<HashMap<String, StructInfo>> {
    // in declaration order: a struct field may be a struct declared before it, which is then known to the next one
    let mut env = env.clone();
    let mut table = HashMap::new();
    for def in defs.iter().filter(|d| d.params.is_empty()) {
        table.insert(def.name.clone(), build_struct(def, &env, file, &def.name)?);
        env.set_structs(table.clone());
    }
    Ok(table)
}

/// One struct, flattened to a bit vector: its fields placed (memory order when exported), and its methods.
/// `name` is the struct's name as it is known (an instance of a parameterized struct: its key).
pub(crate) fn build_struct(def: &StructDef, env: &Env, file: &str, name: &str) -> Result<StructInfo> {
    let mut widths = Vec::new();
    for (dtype, name) in &def.fields {
        let folded = fold_dtype(dtype, env);
        let w = width::signal_bits(&folded, env).filter(|w| *w > 0).ok_or_else(|| {
            Error::new(format!("the width of field '{name}' of struct '{}' must be known", def.name), file, def.pos.0, def.pos.1)
        })?;
        widths.push((name.clone(), w));
    }
    let total: u32 = widths.iter().map(|(_, w)| w).sum();
    let mut fields = Vec::new();
    if def.export {
        // memory order: the first field at the lowest bits, naturally aligned, as C lays it out
        let mut at = 0i64;
        for (name, w) in widths {
            let w = i64::from(w);
            if ![8, 16, 32, 64].contains(&w) || at % w != 0 {
                return Err(Error::new(
                    format!("field '{name}' of exported struct '{}' is {w} bits at bit {at}: an exported struct holds 8, 16, 32 or 64-bit fields, naturally aligned (as C lays them out)", def.name),
                    file,
                    def.pos.0,
                    def.pos.1,
                ));
            }
            fields.push((name, at + w - 1, at));
            at += w;
        }
        if !total.is_multiple_of(32) {
            return Err(Error::new(format!("exported struct '{}' is {total} bits: make it a whole number of 32-bit words", def.name), file, def.pos.0, def.pos.1));
        }
    } else {
        let mut at = i64::from(total);
        for (name, w) in widths {
            fields.push((name, at - 1, at - i64::from(w)));
            at -= i64::from(w);
        }
    }
    let methods = def
        .methods
        .iter()
        .filter_map(|m| match m {
            ItemKind::Fn { name, params, ret, body, .. } => {
                Some((name.clone(), FnDef { params: params.clone(), ret: ret.clone(), body: body.clone() }))
            }
            _ => None,
        })
        .collect();
    // the struct fields that are structs: their type names, which the package of this struct imports
    let mut field_types = HashMap::new();
    for (dtype, name) in &def.fields {
        if let Some(key) = fold::struct_key(dtype, env) {
            field_types.insert(name.clone(), key);
        }
    }
    // a struct is a packed struct type of its own when its struct fields are too; the others stay bit vectors
    let nested_typed = field_types.values().all(|key| env.struct_info(key).is_some_and(|s| s.typed));
    let typed = !def.export && nested_typed;
    if typed {
        env.push_package(&def.origin, crate::outputs::struct_package(name, &fields, &field_types));
    }
    Ok(StructInfo { width: total, fields, methods, typed, field_types })
}

/// The memory layout of an exported struct: each field's offset in bytes (the fields in order, from the lowest byte).
fn record_of(def: &StructDef, env: &Env) -> Result<crate::outputs::Record> {
    let mut at = 0i64;
    let mut fields = Vec::new();
    for (dtype, name) in &def.fields {
        let w = i64::from(width::dtype_width(&fold_dtype(dtype, env)).unwrap_or(0));
        fields.push(crate::outputs::RecordField {
            name: name.clone(),
            width: w,
            offset: at / 8,
            signed: dtype.signed,
            desc: def.field_desc.get(name).cloned().unwrap_or_default(),
        });
        at += w;
    }
    Ok(crate::outputs::Record { name: def.name.clone(), desc: def.desc.clone(), size: at / 8, fields })
}

/// The enums: each member's value (given, or one more than the previous) and the storage type
/// (given, or just wide enough for the largest value).
fn build_enums(defs: &[EnumDef], file: &str) -> Result<(BTreeMap<String, EnumInfo>, Vec<String>)> {
    let mut table = BTreeMap::new();
    let mut order = Vec::new();
    for def in defs {
        let fail = |msg: String| Error::new(msg, file, def.pos.0, def.pos.1);
        if table.contains_key(&def.name) {
            return Err(fail(format!("enum '{}' is defined twice", def.name)));
        }
        let mut members = Vec::new();
        let mut next = 0i64;
        for (member, value) in &def.members {
            let v = match value {
                Some(e) => int_of(&fold_expr(e, &Env::default()))
                    .ok_or_else(|| fail(format!("the value of '{}.{member}' is not a constant", def.name)))?,
                None => next,
            };
            if members.iter().any(|(m, _): &(String, i64)| m == member) {
                return Err(fail(format!("'{member}' is a member of enum '{}' twice", def.name)));
            }
            members.push((member.clone(), v));
            next = v + 1;
        }
        let dtype = match &def.dtype {
            Some(dt) => fold_dtype(dt, &Env::default()),
            None => {
                let largest = members.iter().map(|(_, v)| *v).max().unwrap_or(0).max(0);
                let bits = (64 - largest.leading_zeros()).max(1);
                DType {
                    name: "logic".into(),
                    packed: vec![(Expr::Num((bits - 1).to_string()), Expr::Num("0".into()))],
                    ..DType::default()
                }
            }
        };
        order.push(def.name.clone());
        table.insert(def.name.clone(), EnumInfo { dtype, members });
    }
    Ok((table, order))
}

/// Elaborates a file's modules in source order. A module is emitted after the specialisations it needs.
/// A parameterised module that is instantiated is emitted only as its specialisations; one that is never
/// instantiated is emitted with its defaults, at its place in the source.
/// The elaborated modules, the stage table of each pipeline and the warnings.
pub struct Elaborated {
    pub modules: Vec<Module>,
    pub pipelines: Vec<String>,
    pub warnings: Vec<String>,
    /// Each test with the module it drives (a specialisation, when the test gives arguments).
    pub tests: Vec<(crate::ast::TestDef, String)>,
    /// Each probe with the module it is bound into.
    pub probes: Vec<(crate::ast::ProbeDef, String)>,
    /// The environment of each module (by emitted name) and the test procedures, for the testbench.
    pub envs: HashMap<String, Env>,
    pub procs: crate::testgen::Procs,
    /// The memory maps: name, description, devices.
    pub memmaps: Vec<(String, String, Vec<env::MapDev>)>,
    /// The exported structs: their memory layouts.
    pub records: Vec<crate::outputs::Record>,
    /// The instruction sets, in declaration order.
    pub isas: Vec<isa::IsaInfo>,
    /// The packages of the packed structs: the file each is defined in, and its text.
    pub packages: Vec<(String, String)>,
    /// The handshake ports of each module (by emitted name): the bundle ports with valid and ready, and their role.
    pub handshakes: HashMap<String, Vec<(String, Dir, String)>>,
}

/// The methods that work on a value whose index is not a constant yet (their widths come from the signal, not the index).
const SYMBOLIC_METHODS: &[&str] = &["wrap", "reverse", "msb", "lsb", "zext", "sext", "fill"];

/// A method that needs a value to elaborate (`delay` a count, `list` the elements): only `SYMBOLIC_METHODS` work on a
/// value whose index is not a constant (their widths come from the signal).
fn value_method(x: &Expr) -> bool {
    matches!(x, Expr::Call(f, _) if matches!(&**f, Expr::Member(_, m, _) if !SYMBOLIC_METHODS.contains(&m.as_str())))
}

/// Whether `e` mentions one of the loop variables.
pub(crate) fn mentions_any(e: &Expr, vars: &[String]) -> bool {
    expr_any(e, &|y| matches!(y, Expr::Ident(n, _) if vars.contains(n)))
}

/// A part-select `x[msb:lsb]` whose bounds depend on a loop variable: SystemVerilog needs them constant (the indexed
/// forms `x[i +: w]` are fine).
pub(crate) fn variable_range(x: &Expr, vars: &[String]) -> bool {
    matches!(x, Expr::Slice(_, msb, lsb, op) if op.as_str() == ":" && (mentions_any(msb, vars) || mentions_any(lsb, vars)))
}

/// An expression that elaborates the same with the loop variables not constant.
fn symbolic_expr(e: &Expr, vars: &[String]) -> bool {
    !expr_any(e, &|x| value_method(x) || variable_range(x, vars))
}

/// Whether some expression of a statement (its targets, values, conditions and bounds) satisfies `pred`.
pub(crate) fn stmt_any_expr(s: &Stmt, pred: &dyn Fn(&Expr) -> bool) -> bool {
    match s {
        Stmt::Assign { lhs, rhs, .. } => expr_any(lhs, pred) || expr_any(rhs, pred),
        Stmt::Block(v) => v.iter().any(|x| stmt_any_expr(x, pred)),
        Stmt::If { cond, then, els } => {
            expr_any(cond, pred) || stmt_any_expr(then, pred) || els.as_deref().is_some_and(|e| stmt_any_expr(e, pred))
        }
        Stmt::Case { subject, arms, .. } => {
            expr_any(subject, pred)
                || arms.iter().any(|(labels, body)| labels.iter().flatten().any(|l| expr_any(l, pred)) || stmt_any_expr(body, pred))
        }
        Stmt::For { header, body } => {
            expr_any(&header.lo, pred) || header.bound.as_ref().is_some_and(|b| expr_any(b, pred)) || stmt_any_expr(body, pred)
        }
        _ => false,
    }
}

/// Whether some sub-expression of `e` (itself included) satisfies `pred`.
pub(crate) fn expr_any(e: &Expr, pred: &dyn Fn(&Expr) -> bool) -> bool {
    if pred(e) {
        return true;
    }
    let any = |x: &Expr| expr_any(x, pred);
    match e {
        Expr::Num(_) | Expr::Ident(..) | Expr::OpRef(_) | Expr::Type(_) => false,
        Expr::Unary(_, a) | Expr::Member(a, _, _) | Expr::Keyed(_, a) => any(a),
        Expr::Binary(_, a, b, _) | Expr::Index(a, b) | Expr::Cast(a, b) => any(a) || any(b),
        Expr::Ternary(c, a, b) => any(c) || any(a) || any(b),
        Expr::Slice(a, b, c, _) => any(a) || any(b) || any(c),
        Expr::Call(f, args) => any(f) || args.iter().any(any),
        Expr::Concat(v) | Expr::List(v) | Expr::Pattern(v) => v.iter().any(any),
        Expr::Repl(n, v) => any(n) || v.iter().any(any),
        Expr::Lambda(_, body) => any(body),
        Expr::InSet(x, items) => any(x) || items.iter().any(|i| any(&i.lo) || i.hi.as_ref().is_some_and(any)),
        Expr::Match(x, arms) => any(x) || arms.iter().any(|(labels, v)| any(v) || labels.iter().flatten().any(|i| any(&i.lo) || i.hi.as_ref().is_some_and(any))),
        Expr::With(base, fields, _) => any(base) || fields.iter().any(|(_, v)| any(v)),
    }
}

/// Whether every expression of a statement is symbolic in the loop variables.
fn stmt_exprs_symbolic(s: &Stmt, vars: &[String]) -> bool {
    !stmt_any_expr(s, &|x| value_method(x) || variable_range(x, vars))
}

/// `M m(.*)`: the connections of `.*` written out, in the order of the module's ports, for each port that has no
/// connection of its own and a signal of the same name in scope. The other connections keep their place.
fn expand_star(conns: Vec<Conn>, target: &Module, env: &Env) -> Vec<Conn> {
    if !conns.iter().any(|c| c.port.as_deref() == Some("*")) {
        return conns;
    }
    let explicit: Vec<Conn> = conns.into_iter().filter(|c| c.port.as_deref() != Some("*")).collect();
    let named: HashSet<String> = explicit.iter().filter_map(|c| c.port.clone()).collect();
    let mut out = explicit;
    for p in &target.ports {
        if !named.contains(&p.name) && env.signal(&p.name).is_some() {
            out.push(Conn { port: Some(p.name.clone()), expr: Some(Expr::Ident(p.name.clone(), (0, 0))) });
        }
    }
    out
}

/// The names a source uses: every identifier and every member name (the Debug text of the tree, where each one is a string
/// followed by its position).
fn names_in_source(source: &File) -> HashSet<String> {
    let text = format!("{source:?}");
    let mut names = HashSet::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("\", (") {
        let at = from + found;
        if let Some(open) = text[..at].rfind('"') {
            names.insert(text[open + 1..at].to_string());
        }
        from = at + 1;
    }
    names
}

/// The coverage block of a module: one flag per item, set in a clocked block, and printed by a `final` block. Empty when
/// the module has no item.
fn coverage_lines(_env: &Env, items: Vec<(String, Expr)>, clocked: bool) -> Option<Vec<String>> {
    if items.is_empty() {
        return None;
    }
    let mut lines = vec!["`ifdef OVC_COVER".to_string(), "// ovc: coverage (case arms, states)".to_string()];
    lines.extend((0..items.len()).map(|i| format!("logic ov_cov_{i} = 1'b0;")));
    lines.push(if clocked { "always @(posedge clk) begin".into() } else { "always @* begin".into() });
    for (i, (_, cond)) in items.iter().enumerate() {
        let text = crate::emit::expr::expr(cond, -1, false).unwrap_or_default();
        lines.push(format!("  if ({text}) ov_cov_{i} = 1'b1;"));
    }
    lines.push("end".into());
    lines.push("final begin".into());
    for (i, (key, _)) in items.iter().enumerate() {
        lines.push(format!("  $display(\"ovc-cover %0d {}\", ov_cov_{i});", key.replace('"', "'")));
    }
    lines.push("end".into());
    lines.push("`endif".into());
    Some(lines)
}

/// The handshake bundles among the references of a port: those with valid and ready fields, not a monitor. Each one is
/// named as the flattened signals are (`p_s`) and shown as the source names it (`p.s`).
fn handshake_refs(base: &str, refs: &[(String, BundleRef)]) -> Vec<(String, Dir, String)> {
    refs.iter()
        .filter(|(_, b)| b.fields.iter().any(|(f, _)| f == "valid") && b.fields.iter().any(|(f, _)| f == "ready"))
        .filter(|(_, b)| b.role != Dir::Monitor)
        .map(|(key, b)| {
            let shown = match key.strip_prefix(base).and_then(|rest| rest.strip_prefix('_')) {
                Some(path) => format!("{base}.{}", path.replace('_', ".")),
                None => key.clone(),
            };
            (key.clone(), b.role, shown)
        })
        .collect()
}

/// The handshake channels of a port's references, with their names: each channel of a nested bundle too (`m_ar`, shown
/// as `m.ar`), so that its valid and ready get the checks of their own.
fn handshake_channels(base: &str, refs: &[(String, BundleRef)]) -> Vec<(String, BundleRef, String)> {
    refs.iter()
        .filter(|(_, b)| b.fields.iter().any(|(f, _)| f == "valid") && b.fields.iter().any(|(f, _)| f == "ready"))
        .filter(|(_, b)| b.role != Dir::Monitor)
        .map(|(key, b)| {
            let shown = match key.strip_prefix(base).and_then(|rest| rest.strip_prefix('_')) {
                Some(path) => format!("{base}.{}", path.replace('_', ".")),
                None => key.clone(),
            };
            (key.clone(), b.clone(), shown)
        })
        .collect()
}

/// The modules with their base modules merged in (`extends`), and the traits checked.
fn merge_extends(source: &File, file: &str) -> Result<Vec<Module>> {
    let by_name: HashMap<&str, &Module> = source.modules.iter().map(|m| (m.name.as_str(), m)).collect();
    fn merged(m: &Module, by_name: &HashMap<&str, &Module>, file: &str, depth: usize) -> Result<Module> {
        let Some((base_name, args)) = &m.extends else { return Ok(m.clone()) };
        let fail = |msg: String| Error::new(msg, file, m.pos.0, m.pos.1);
        if depth > 32 {
            return Err(fail(format!("module '{}' extends itself", m.name)));
        }
        if !args.is_empty() {
            return Err(fail("extends with arguments is not supported yet".into()));
        }
        let base = by_name.get(base_name.as_str()).ok_or_else(|| fail(format!("unknown base module '{base_name}'")))?;
        let base = merged(base, by_name, file, depth + 1)?;
        let own_fns: Vec<&str> = m
            .items
            .iter()
            .filter_map(|it| match &it.kind {
                ItemKind::Fn { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        // an override must replace a function of the base: one that replaces nothing is a mistake
        let base_fns: Vec<&str> = base
            .items
            .iter()
            .filter_map(|it| match &it.kind {
                ItemKind::Fn { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        for it in &m.items {
            if let ItemKind::Fn { name, override_at: Some(at), .. } = &it.kind {
                if !base_fns.contains(&name.as_str()) {
                    return Err(Error::new(format!("'{name}' does not override anything"), file, at.0, at.1));
                }
            }
        }
        // an override replaces the base's function of the same name
        let mut items: Vec<Item> = base
            .items
            .iter()
            .filter(|it| !matches!(&it.kind, ItemKind::Fn { name, .. } if own_fns.contains(&name.as_str())))
            .cloned()
            .collect();
        items.extend(m.items.iter().cloned());
        let mut ports = base.ports.clone();
        ports.extend(m.ports.iter().cloned());
        Ok(Module { ports, items, extends: None, ..m.clone() })
    }
    let mut out = Vec::with_capacity(source.modules.len());
    for m in &source.modules {
        let flat = merged(m, &by_name, file, 0)?;
        for t in &flat.traits {
            let Some(def) = source.traits.iter().find(|d| &d.name == t) else {
                return Err(Error::new(format!("unknown trait '{t}'"), file, m.pos.0, m.pos.1));
            };
            for f in &def.fns {
                let has = flat.items.iter().any(|it| matches!(&it.kind, ItemKind::Fn { name, .. } if name == f));
                if !has {
                    return Err(Error::new(
                        format!("module '{}' does not implement trait '{t}': missing fn '{f}'", m.name),
                        file,
                        m.pos.0,
                        m.pos.1,
                    ));
                }
            }
        }
        out.push(flat);
    }
    Ok(out)
}

/// `top`: the root module with its arguments, when the design has one (the only root then).
pub fn elaborate(source: &File, file: &str, asserts: bool, cover: bool, top: Option<&DType>) -> Result<Elaborated> {
    let merged = merge_extends(source, file)?;
    let modules = &merged;
    let (enums, enum_order) = build_enums(&source.enums, file)?;
    let mut globals = Env::with_enums(Rc::new(enums.clone()));
    globals.bind_int("true", 1);
    globals.bind_int("false", 0);
    globals.set_configs(config::table(&source.configs));
    // a config named without a parameter (`Reg<Cfg.w>`): its fields are values of the design, with its own defaults
    for def in source.configs.iter().filter(|d| d.source.is_none()) {
        if let Ok(values) = config::evaluate(&def.name, &globals, &[], &def.origin, def.pos) {
            for (field, value) in values {
                globals.define_global_expr(&format!("{}.{field}", def.name), value);
            }
        }
    }
    // a config from a manifest: its entries are values named `Config.entry` (lists of records, ...)
    for def in source.configs.iter().filter(|d| d.source.is_some()) {
        let rel = def.source.as_deref().unwrap_or_default();
        let path = std::path::Path::new(&def.origin).parent().map_or_else(|| std::path::PathBuf::from(rel), |d| d.join(rel));
        let fail = |msg: String| Error::new(msg, &def.origin, def.pos.0, def.pos.1);
        let text = std::fs::read_to_string(&path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
        for (key, value) in crate::toml::parse(&text).map_err(|e| fail(format!("{}: {e}", path.display())))? {
            globals.define_global_expr(&format!("{}.{key}", def.name), value);
        }
    }
    for decl in &source.consts {
        for d in &decl.decls {
            let init = d.init.as_ref().ok_or_else(|| Error::new(format!("constant '{}' has no value", d.name), file, 0, 0))?;
            // a constant has an integer value whatever the literal's form (`8'b1010` included)
            let v = match fold_expr(init, &globals) {
                Expr::Num(text) => crate::lexer::number_value(&text),
                other => int_of(&other),
            }
            .ok_or_else(|| Error::new(format!("constant '{}' is not a constant", d.name), file, 0, 0))?;
            globals.bind_int(&d.name, v);
        }
    }
    let top_fns = source.fns.iter().filter_map(|k| match k {
        ItemKind::Fn { name, params, ret, body, .. } => {
            Some((name.clone(), FnDef { params: params.clone(), ret: ret.clone(), body: body.clone() }))
        }
        _ => None,
    });
    globals.set_fns(top_fns.collect());
    // the top-level values are folded where the functions are known: a value that calls a function folds it now, and does
    // not keep the call for a module whose port of the same name would hide the function
    for (name, expr) in &source.values {
        match fold_expr(expr, &globals) {
            Expr::List(items) => globals.define_global_list(name, items),
            value => match int_of(&value) {
                Some(v) => globals.bind_int(name, v),
                None => globals.define_global_expr(name, value),
            },
        }
    }
    // typedefs first (fields and signals use them), then the structs, flattened to bit vectors
    for (name, dtype) in &source.typedefs {
        let folded = fold_dtype(dtype, &globals);
        globals.bind_type(name, folded);
    }
    globals.set_struct_defs(source.structs.iter().filter(|d| !d.params.is_empty()).map(|d| (d.name.clone(), d.clone())).collect());
    globals.set_structs(build_structs(&source.structs, &globals, file)?);
    // the exported structs: their layout in memory (the first field at the lowest bytes), for the C and Python outputs
    let records = source.structs.iter().filter(|d| d.export).map(|d| record_of(d, &globals)).collect::<Result<Vec<_>>>()?;
    globals.set_bundles(source.bundles.iter().map(|b| (b.name.clone(), b.clone())).collect());
    let mut isas = HashMap::new();
    for def in &source.isas {
        isas.insert(def.name.clone(), isa::build(def, &globals, file)?);
    }
    globals.set_isas(isas);
    // the instruction sets in declaration order, for the SV package, the Python encoder and the documentation
    let isa_list = source.isas.iter().map(|def| isa::build(def, &globals, file)).collect::<Result<Vec<_>>>()?;
    let mut design = Design {
        groups: Vec::new(),
        globals,
        enums: Rc::new(enums),
        enum_order,
        by_name: modules.iter().map(|m| (m.name.as_str(), m)).collect(),
        file,
        specs: HashMap::new(),
        emitted: HashSet::new(),
        out: Vec::new(),
        pipelines: Vec::new(),
        warnings: Vec::new(),
        asserts,
        cover,
        local_count: 0,
        memmaps: Vec::new(),
        envs: HashMap::new(),
        handshakes: HashMap::new(),
        used_names: names_in_source(source),
    };
    // roots: the modules that nothing instantiates, extends or binds as a module parameter; each emits its
    // dependencies first. The others are emitted where they are instantiated.
    let mut used = HashSet::new();
    for m in &source.modules {
        collect_instances(&m.items, &mut used);
        if let Some((base, _)) = &m.extends {
            used.insert(base.clone());
        }
        for p in &m.params {
            if let (true, Some(Arg::Expr(Expr::Ident(n, _)))) = (p.kind == "module", &p.default) {
                used.insert(n.clone());
            }
        }
    }
    if let Some(top) = top {
        // the top module with its arguments: its specialisation is the only root (named after the module)
        let target = modules.iter().find(|m| m.name == top.name).ok_or_else(|| {
            Error::new(format!("no module '{}' to use as the top", top.name), file, 0, 0)
        })?;
        let targs: Vec<(Option<String>, Arg)> = top.targs.iter().map(|a| (None, a.clone())).collect();
        let overrides = instance_overrides(target, &targs, &design.globals, &Ctx { file, pos: target.pos })?;
        design.emitted.insert(target.name.clone());
        let elab = design.elab_module(target, &overrides, &target.name)?;
        design.out.push(elab);
    }
    for m in modules {
        if top.is_some() || used.contains(&m.name) || source.library.contains(&m.name) || !m.params.iter().all(|p| p.default.is_some()) {
            continue;
        }
        if !m.params.is_empty() {
            let bound = bind_params(m, &Overrides::default(), &design.globals, &Ctx { file, pos: m.pos })?;
            let values = spec_values(m, &Overrides::default(), &bound);
            design.specs.insert(format!("{}<{}>", m.name, values.join(",")), m.name.clone());
        } else {
            design.emitted.insert(m.name.clone());
        }
        let elab = design.elab_module(m, &Overrides::default(), &m.name)?;
        design.out.push(elab);
    }
    // the modules that tests instantiate, after the roots
    let globals = design.globals.clone();
    let mut tests = Vec::new();
    let mut all_tests = source.tests.clone();
    for l in &source.test_loops {
        expand_test_loop(l, &[], &globals, file, &mut all_tests)?;
    }
    // a test's name is its line in the report: two of one name would report under one name
    for (k, t) in all_tests.iter().enumerate() {
        if let Some(earlier) = all_tests[..k].iter().find(|u| u.name == t.name) {
            return Err(Error::new(format!("two tests are named \"{}\" (first at line {})", t.name, earlier.pos.0), file, t.pos.0, t.pos.1));
        }
    }
    for t in &all_tests {
        let inst = Instance {
            module: t.module.clone(),
            targs: t.targs.iter().map(|a| (None, a.clone())).collect(),
            name: String::new(),
            conns: Vec::new(),
            pos: t.pos,
        };
        let dut = design.instance_module(&inst, &globals, &Ctx { file, pos: t.pos })?;
        tests.push((t.clone(), dut));
    }
    let mut probes = Vec::new();
    for p in &source.probes {
        let inst = Instance { module: p.module.clone(), targs: Vec::new(), name: String::new(), conns: Vec::new(), pos: p.pos };
        let target = design.instance_module(&inst, &globals, &Ctx { file, pos: p.pos })?;
        probes.push((p.clone(), target));
    }
    let procs: crate::testgen::Procs = source
        .fns
        .iter()
        .filter_map(|k| match k {
            ItemKind::Proc { name, params, body } => Some((name.clone(), (params.clone(), body.clone()))),
            _ => None,
        })
        .collect();
    // the packages of the packed structs, the instances included, in the order they were built
    let packages = globals.packages();
    Ok(Elaborated { modules: design.out, pipelines: design.pipelines, warnings: design.warnings, tests, probes, envs: design.envs, procs, memmaps: design.memmaps, records, isas: isa_list, packages, handshakes: design.handshakes })
}

struct Ctx<'a> {
    file: &'a str,
    pos: (usize, usize),
}

impl Ctx<'_> {
    fn err(&self, message: impl Into<String>) -> Error {
        Error::new(message, self.file, self.pos.0, self.pos.1)
    }

    fn at(&self, pos: (usize, usize)) -> Ctx<'_> {
        Ctx { file: self.file, pos }
    }
}

struct Design<'a> {
    /// The vectors of bundles of the module being elaborated (name, count): merged when it is done.
    groups: Vec<(String, usize)>,
    /// The memory maps elaborated: name, description and devices.
    memmaps: Vec<(String, String, Vec<env::MapDev>)>,
    /// The environment of each module elaborated, by emitted name.
    envs: HashMap<String, Env>,
    /// Every name the source uses as an identifier or a member (`inst.port`): an instance with one of these names may be
    /// read from another module, so its loop is unrolled (its name must stay as it is).
    used_names: HashSet<String>,
    /// The handshake ports of each module (by emitted name): the bundle ports with valid and ready, and their role.
    handshakes: HashMap<String, Vec<(String, Dir, String)>>,
    /// Block-local variables hoisted to the module so far (their names are numbered).
    local_count: usize,
    /// Top-level constants and the enums.
    globals: Env,
    enums: Rc<BTreeMap<String, EnumInfo>>,
    /// Enum names in declaration order.
    enum_order: Vec<String>,
    by_name: HashMap<&'a str, &'a Module>,
    file: &'a str,
    /// Specialisation key -> emitted module name.
    specs: HashMap<String, String>,
    /// The modules without parameters that have been emitted.
    emitted: HashSet<String>,
    out: Vec<Module>,
    /// The stage tables of the pipelines, for `--dump-pipeline`.
    pipelines: Vec<String>,
    warnings: Vec<String>,
    asserts: bool,
    /// `--cover`: the case statements of each module get coverage items.
    cover: bool,
}

/// The struct a view names: `x.as(S)` or `x.as(S<args>)`.
fn view_struct_key(expr: &Expr, env: &Env) -> Option<String> {
    let Expr::Call(f, args) = expr else { return None };
    let (Expr::Member(_, method, _), [arg]) = (&**f, args.as_slice()) else { return None };
    if method != "as" {
        return None;
    }
    match arg {
        Expr::Ident(n, _) if env.struct_info(n).is_some() => Some(n.clone()),
        Expr::Type(dt) => fold::struct_key(dt, env),
        _ => None,
    }
}

impl<'a> Design<'a> {
    /// The constant bounds of a generate loop that stays a loop: a range `lo..hi` whose body is instances, continuous
    /// and procedural assignments, and loops of the same kind. Other loops are unrolled.
    fn generate_bounds(&self, header: &ForHeader, body: &[Item], env: &Env, outer: &[String]) -> Option<(i64, i64)> {
        if header.bound.is_some() || header.step.is_some() {
            return None;
        }
        let Expr::Binary(op, lo, hi, _) = fold_expr(&header.lo, env) else { return None };
        if op != ".." {
            return None;
        }
        let bounds = (int_of(&lo)?, int_of(&hi)?);
        let mut inner = env.clone();
        inner.unbind(&header.var);
        let mut vars = outer.to_vec();
        vars.push(header.var.clone());
        body.iter().all(|it| self.plain_generate_item(it, &vars, &inner)).then_some(bounds)
    }

    fn plain_generate_item(&self, item: &Item, vars: &[String], env: &Env) -> bool {
        match &item.kind {
            ItemKind::Instance(i) => {
                let targs_plain = i.targs.iter().all(|(_, a)| match a {
                    Arg::Expr(e) => !mentions_any(e, vars),
                    Arg::Type(_) => true,
                });
                let target = env.module_alias(&i.module).unwrap_or(&i.module);
                let scalar_ports = self.by_name.get(target).is_some_and(|m| {
                    m.ports.iter().all(|p| self.globals.bundle_def(&p.dtype.name).is_none() && bundle::vec_of(&p.dtype).is_none())
                });
                targs_plain && scalar_ports && !self.used_names.contains(&i.name)
            }
            ItemKind::ContAssign { lhs, rhs } => symbolic_expr(lhs, vars) && symbolic_expr(rhs, vars),
            ItemKind::Comment(_) => true,
            ItemKind::AlwaysComb(body) => fold::is_plain_body(body) && stmt_exprs_symbolic(body, vars),
            ItemKind::Always { sens: Sens::Edges(_), body, .. } => fold::is_plain_body(body) && stmt_exprs_symbolic(body, vars),
            ItemKind::GenFor { header, body } => self.generate_bounds(header, body, env, vars).is_some(),
            _ => false,
        }
    }

    /// The monitors of a bundle port: an instance of each, on the clock and reset and the port.
    fn monitor_items(&self, def: &crate::ast::BundleDef, p: &Port, env: &Env, ctx: &Ctx) -> Result<Vec<Item>> {
        // a monitor watches the slave ports of its bundle; they are checks, so `--no-asserts` leaves them out
        if p.dir != Dir::Slave || !self.asserts {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for (k, (module, targs)) in bundle::monitors(def, &p.dtype.targs, env, self.file, p.pos)?.into_iter().enumerate() {
            let port = self
                .by_name
                .get(module.as_str())
                .and_then(|m| m.ports.iter().find(|q| q.dir == Dir::Monitor))
                .map(|q| q.name.clone())
                .ok_or_else(|| ctx.err(format!("monitor '{module}' has no monitor port")))?;
            let conns = vec![
                Conn { port: Some("clk".into()), expr: Some(Expr::Ident("clk".into(), p.pos)) },
                Conn { port: Some("rst".into()), expr: Some(Expr::Ident("rst".into(), p.pos)) },
                Conn { port: Some(port), expr: Some(Expr::Ident(p.name.clone(), p.pos)) },
            ];
            let name = format!("{}_mon{k}", p.name);
            let targs = targs.into_iter().map(|a| (None, a)).collect();
            out.push(Item { pos: p.pos, kind: ItemKind::Instance(Instance { module, targs, name, conns, pos: p.pos }) });
        }
        Ok(out)
    }

    /// A vector of bundles, `Vec<B, N>`: its bundle definition, the element type and the count.
    fn vec_bundle(&self, dtype: &DType, env: &Env, ctx: &Ctx) -> Result<Option<(crate::ast::BundleDef, DType, usize)>> {
        let Some((elem, count)) = bundle::vec_of(dtype) else { return Ok(None) };
        // a vector of anything else is a packed array, lowered like any other type
        let Some(def) = self.globals.bundle_def(&elem.name).cloned() else { return Ok(None) };
        let n = int_of(&fold_expr(&count, env)).ok_or_else(|| ctx.err("the size of a vector of bundles must be constant"))?;
        Ok(Some((def, elem, usize::try_from(n).unwrap_or(0))))
    }

    /// A module: its body is elaborated, then its vectors of bundles are merged (see `vecmerge`).
    fn elab_module(&mut self, m: &'a Module, overrides: &Overrides, name: &str) -> Result<Module> {
        let saved = std::mem::take(&mut self.groups);
        let result = self.elab_module_body(m, overrides, name);
        let groups = std::mem::replace(&mut self.groups, saved);
        let module = result?;
        let (module, elements) = vecmerge::merge(module, &groups);
        // the checks read the names the module has in the output: after the vectors are merged
        if let Some(env) = self.envs.get(name) {
            if let Some((msg, pos)) = member_error(&module, env) {
                return Err(Error::new(msg, ctx_file(m, self.file), pos.0, pos.1));
            }
        }
        if let Some((name, pos)) = unknown_identifier(&module) {
            // a `fn` of the module used as a value: a fn of statements has no value (it is called as a statement)
            if m.items.iter().any(|it| matches!(&it.kind, ItemKind::Fn { name: n, .. } | ItemKind::Proc { name: n, .. } if *n == name)) {
                return Err(Error::new(
                    format!("a fn of statements has no `return` and is called as a statement: '{name}' is used as a value"),
                    ctx_file(m, self.file),
                    pos.0,
                    pos.1,
                ));
            }
            let hint = suggest(&name, known_names(&module));
            return Err(Error::new(format!("unknown identifier '{name}'{hint}"), ctx_file(m, self.file), pos.0, pos.1));
        }
        if let Some((port, pos)) = driven_input(&module) {
            return Err(Error::new(format!("input port '{port}' is driven inside the module"), ctx_file(m, self.file), pos.0, pos.1));
        }
        if let Some((name, pos)) = written_by_ff_and_initial(&module) {
            return Err(Error::new(format!("'{name}' is written by an always_ff and by an initial block"), ctx_file(m, self.file), pos.0, pos.1));
        }
        if let Some(env) = self.envs.get_mut(name) {
            env.set_vec_elements(elements);
        }
        Ok(module)
    }

    fn elab_module_body(&mut self, m: &'a Module, overrides: &Overrides, name: &str) -> Result<Module> {
        let ctx = Ctx { file: if m.origin.is_empty() { self.file } else { &m.origin }, pos: m.pos };
        if m.kind != "module" && m.kind != "pipeline" {
            return Err(ctx.err(format!("'{}' is not supported yet", m.kind)));
        }
        let mut env = bind_params(m, overrides, &self.globals, &ctx)?;
        env.start_cover(self.cover, &m.origin);
        // the enums the module declares shadow the top-level ones of the same name
        let (local, local_order) = build_enums(&m.enums, ctx.file)?;
        let mut table = (*self.enums).clone();
        table.extend(local);
        let table = Rc::new(table);
        env.set_enums(table.clone());
        let mut ports = Vec::new();
        let mut bundle_of: HashMap<String, BundleRef> = HashMap::new();
        // the handshake ports of the module: name, role, and how it is shown in the messages (`p.s` for `p_s`)
        let mut handshakes: Vec<(String, Dir, String)> = Vec::new();
        let mut channels: Vec<(String, BundleRef, String)> = Vec::new();
        let mut monitor_items: Vec<Item> = Vec::new();
        let mut struct_of: HashMap<String, String> = HashMap::new();
        for p in &m.ports {
            if let Some((def, elem, n)) = self.vec_bundle(&p.dtype, &env, &ctx.at(p.pos))? {
                self.groups.push((p.name.clone(), n));
                for k in 0..n {
                    let element = Port { name: format!("{}_{k}", p.name), dtype: elem.clone(), ..p.clone() };
                    let (expanded, refs, structs) = bundle::expand_port(&def, &element, &env, self.file)?;
                    handshakes.extend(handshake_refs(&element.name, &refs));
                    channels.extend(handshake_channels(&element.name, &refs));
                    bundle_of.extend(refs);
                    struct_of.extend(structs);
                    ports.extend(expanded);
                    monitor_items.extend(self.monitor_items(&def, &element, &env, &ctx)?);
                }
                continue;
            }
            if let Some(def) = self.globals.bundle_def(&p.dtype.name).cloned() {
                let (expanded, refs, structs) = bundle::expand_port(&def, p, &env, self.file)?;
                handshakes.extend(handshake_refs(&p.name, &refs));
                channels.extend(handshake_channels(&p.name, &refs));
                bundle_of.extend(refs);
                struct_of.extend(structs);
                ports.extend(expanded);
                monitor_items.extend(self.monitor_items(&def, p, &env, &ctx)?);
                continue;
            }
            if let Some(cond) = &p.cond {
                match int_of(&fold_expr(cond, &env)) {
                    Some(0) => continue,
                    Some(_) => {}
                    None => return Err(ctx.at(p.pos).err("non-constant port conditions are not supported yet")),
                }
            }
            if matches!(p.dir, Dir::Master | Dir::Slave | Dir::Monitor) {
                return Err(ctx.at(p.pos).err(format!("'{}' ports are not supported yet", dir_word(p.dir))));
            }
            let (dtype, extra) = types::lower(fold_dtype(&p.dtype, &env)).map_err(|e| ctx.at(p.pos).err(e))?;
            let mut dims = extra;
            dims.extend(p.dims.iter().map(|d| fold_expr(d, &env)));
            ports.push(Port {
                dir: p.dir,
                dtype,
                name: p.name.clone(),
                dims,
                cond: None,
                is_async: p.is_async,
                default: p.default.as_ref().map(|d| fold_expr(d, &env)),
                pos: p.pos,
            });
        }
        // what the list methods and the implicit clock and reset need to know about the module
        let mut signals: HashMap<String, (DType, Vec<Expr>)> = ports.iter().map(|p| (p.name.clone(), (p.dtype.clone(), p.dims.clone()))).collect();
        let mut reg_inits = Vec::new();
        // an `output reg` port is reset like a local register: to 0
        for p in &ports {
            if p.dtype.name == "reg" && p.dims.is_empty() {
                reg_inits.push((p.name.clone(), Expr::Num("0".into())));
            }
        }
        let mut reset = None;
        // the integer constants and the memory maps come first, in order: the sizes of the vectors of bundles may be
        // made of them (`io.count`); the errors of a map are reported where it is elaborated
        for it in &m.items {
            match &it.kind {
                ItemKind::Val { name, expr } => {
                    // a delay line is a register, never a constant: folding it here would build it twice
                    if !expr_any(expr, &|x| matches!(x, Expr::Call(f, _) if matches!(&**f, Expr::Member(_, m, _) if m == "delay"))) {
                        if let Some(v) = int_of(&fold_expr(expr, &env)) {
                            env.define_const(name, v);
                        }
                    }
                }
                // `const int A = 0, B = 1;` with integer values: compile-time constants, nothing is emitted
                ItemKind::Decl(d) if d.is_const => {
                    for x in &d.decls {
                        if let Some(v) = x.init.as_ref().and_then(|e| int_of(&fold_expr(e, &env))) {
                            env.define_const(&x.name, v);
                        }
                    }
                }
                ItemKind::MemMap { name, entries, .. } => {
                    if let Ok(map) = build_memmap(entries, &env, &ctx) {
                        env.define_memmap(name, map);
                    }
                }
                _ => {}
            }
        }
        for it in &m.items {
            match &it.kind {
                ItemKind::Decl(d) => {
                    if let Some((def, elem, n)) = self.vec_bundle(&d.dtype, &env, &ctx)? {
                        for x in &d.decls {
                            for k in 0..n {
                                let (refs, structs) = bundle::local_refs(&def, &elem.targs, &format!("{}_{k}", x.name), &env, self.file, it.pos)?;
                                bundle_of.extend(refs);
                                struct_of.extend(structs);
                            }
                        }
                        continue;
                    }
                    if let Some(def) = self.globals.bundle_def(&d.dtype.name).cloned() {
                        for x in &d.decls {
                            let (refs, structs) = bundle::local_refs(&def, &d.dtype.targs, &x.name, &env, self.file, it.pos)?;
                            bundle_of.extend(refs);
                            struct_of.extend(structs);
                        }
                        continue;
                    }
                    let (dtype, extra) = types::lower(fold_dtype(&d.dtype, &env)).map_err(|e| ctx.at(it.pos).err(e))?;
                    for x in &d.decls {
                        let mut dims = extra.clone();
                        dims.extend(x.dims.iter().map(|e| fold_expr(e, &env)));
                        signals.insert(x.name.clone(), (dtype.clone(), dims));
                        // a register is reset to its initial value, or to 0 (see implicit_clocking)
                        if d.dtype.name == "reg" {
                            let init = match &x.init {
                                Some(e) => fold_expr(e, &env),
                                None => Expr::Num("0".into()),
                            };
                            reg_inits.push((x.name.clone(), init));
                        }
                    }
                }
                ItemKind::Reset { name, low, is_async } => {
                    reset = Some(ResetSpec { name: name.clone(), low: *low, is_async: *is_async });
                }
                _ => {}
            }
        }
        // the signals declared with a struct type: field access and methods go through them
        for p in &m.ports {
            if let Some(key) = fold::struct_key(&p.dtype, &env) {
                struct_of.insert(p.name.clone(), key);
            }
        }
        // a `val` that views bits as a struct (`val p = x.as(Pte);`) is a signal of that struct
        fn struct_views(items: &[Item], env: &Env, out: &mut HashMap<String, String>) {
            for it in items {
                match &it.kind {
                    ItemKind::Val { name, expr } => {
                        if let Some(key) = view_struct_key(expr, env) {
                            out.insert(name.clone(), key);
                        }
                    }
                    ItemKind::Stage { items, .. } => struct_views(items, env, out),
                    _ => {}
                }
            }
        }
        struct_views(&m.items, &env, &mut struct_of);
        for it in &m.items {
            if let ItemKind::Decl(d) = &it.kind {
                if let Some(key) = fold::struct_key(&d.dtype, &env) {
                    for x in &d.decls {
                        struct_of.insert(x.name.clone(), key.clone());
                    }
                }
            }
        }
        env.set_struct_of(struct_of);
        env.set_bundle_of(bundle_of);
        env.set_signals(signals);
        // without a `reset` declaration, the module's reset is the port the implicit clock and reset use
        let reset = reset.or_else(|| default_reset(&ports));
        env.set_reset(reset, reg_inits);
        if let Some(msg) = select_past_width(&m.items, &env).or_else(|| view_width_error(&m.items, &env)) {
            return Err(ctx.err(msg));
        }
        // a port is driven from outside: only a bundle of the module itself can be tied off
        for it in &m.items {
            if let ItemKind::TieOff(Expr::Ident(name, _)) = &it.kind {
                if m.ports.iter().any(|p| &p.name == name) {
                    return Err(ctx.at(it.pos).err("tie_off() takes a bundle signal, not a port"));
                }
            }
        }
        let mut items = Vec::new();
        self.elab_items(&m.items, &env, &ctx, &[], &mut items)?;
        if fold::take_fold_overflow() {
            return Err(ctx.err("a fold builds more than 200000 terms: its value uses its accumulator more than once, so it doubles at each step"));
        }
        self.elab_items(&monitor_items, &env, &ctx, &[], &mut items)?;
        // the coverage of the case statements: sticky flags, printed at the end of the simulation (`ovc cover`)
        if let Some(lines) = coverage_lines(&env, env.cover_items(), m.ports.iter().any(|p| p.name == "clk")) {
            items.push(Item { pos: m.pos, kind: ItemKind::Raw(lines) });
        }
        // the members of the enums this module uses, declared first
        let used = env.used_enums();
        let mut declared = Vec::new();
        let mut seen = HashSet::new();
        for name in local_order.iter().chain(self.enum_order.iter()).filter(|n| used.contains(*n) && seen.insert((*n).clone())) {
            let info = &table[name];
            for (member, value) in &info.members {
                let decl = Decl {
                    dtype: info.dtype.clone(),
                    decls: vec![Declarator { name: format!("{name}_{member}"), dims: Vec::new(), init: Some(Expr::Num(value.to_string())) }],
                    is_const: true,
                    is_async: false,
                };
                declared.push(Item { pos: m.pos, kind: ItemKind::Decl(decl) });
            }
        }
        // the valid/ready checks of the handshake ports
        let bundle_ports: Vec<(String, env::BundleRef)> = m
            .ports
            .iter()
            .flat_map(|p| {
                let names: Vec<String> = if bundle::vec_of(&p.dtype).is_some() {
                    (0..bundle::element_count(&env, &p.name)).map(|k| format!("{}_{k}", p.name)).collect()
                } else {
                    vec![p.name.clone()]
                };
                names.into_iter().filter_map(|n| env.bundle_of(&n).map(|b| (n.clone(), b.clone()))).collect::<Vec<_>>()
            })
            .collect();
        // the rule of the clock-domain check: a producer's valid must not depend on its ready (see `domains`)
        self.handshakes.insert(name.to_string(), handshakes);
        // a bundle port checks its own valid and ready; its nested channels (`m.ar`) check theirs
        let mut checked: Vec<(String, BundleRef, String)> = bundle_ports.into_iter().map(|(port, b)| (port.clone(), b, port)).collect();
        for (key, b, shown) in channels {
            if !checked.iter().any(|(k, _, _)| *k == key) {
                checked.push((key, b, shown));
            }
        }
        let checks: Vec<String> = checked
            .into_iter()
            .flat_map(|(port, b, shown)| {
                let fields: Vec<String> = b.fields.iter().map(|(f, _)| f.clone()).collect();
                bundle::handshake_checks(&m.name, &port, &shown, b.role, &fields, self.asserts)
            })
            .collect();
        if !checks.is_empty() {
            items.push(Item { pos: m.pos, kind: ItemKind::Raw(checks) });
        }
        // the length tables of the instruction sets this module uses
        for isa_name in env.used_isas() {
            let info = env.isa_info(&isa_name).expect("used isa exists");
            let largest = info.lengths.iter().copied().max().unwrap_or(1).max(1);
            let bits = 64 - (largest as u64).leading_zeros() as i64;
            let dtype = DType { name: "logic".into(), packed: vec![(Expr::Num((bits - 1).to_string()), Expr::Num("0".into()))], ..DType::default() };
            let table = Expr::Pattern(info.lengths.iter().map(|l| Expr::Num(l.to_string())).collect());
            let decl = Decl {
                dtype,
                decls: vec![Declarator { name: format!("{isa_name}_lengths"), dims: vec![Expr::Num(info.lengths.len().to_string())], init: Some(table) }],
                is_const: true,
                is_async: false,
            };
            declared.push(Item { pos: m.pos, kind: ItemKind::Decl(decl) });
        }
        // order: declarations (the enum members, the delay registers, the module's own), then the delay
        // blocks, then the logic in source order
        let (dly_decls, dly_blocks) = env.take_delay_items();
        let (module_decls, rest): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|i| matches!(i.kind, ItemKind::Decl(_)));
        let mut ordered = declared;
        ordered.extend(dly_decls);
        ordered.extend(module_decls);
        ordered.extend(dly_blocks);
        ordered.extend(rest);
        let items = ordered;
        let module = Module {
            enums: Vec::new(),
            origin: m.origin.clone(),
            kind: m.kind.clone(),
            name: name.to_string(),
            params: Vec::new(),
            ports,
            extends: None,
            traits: Vec::new(),
            items,
            pos: m.pos,
        };
        if m.kind == "pipeline" {
            let directives = pipeline::directives(m)?;
            // the flattened signals of the bundle ports: `o_ready` of `master Stream o`
            let bundle_signals: Vec<String> = env
                .bundle_refs()
                .iter()
                .flat_map(|b| b.fields.iter().map(move |(f, _)| format!("{}_{f}", b.prefix)))
                .collect();
            let lowered = pipeline::lower(&module, self.file, self.asserts, &directives, &bundle_signals)?;
            self.pipelines.push(lowered.table);
            self.warnings.extend(lowered.warnings);
            return Ok(lowered.module);
        }
        // the environment of the module is kept: the tests that drive it resolve its bundle fields and structs
        self.envs.insert(name.to_string(), env.clone());
        Ok(module)
    }

    fn elab_items(&mut self, items: &[Item], env: &Env, ctx: &Ctx, loops: &[i64], out: &mut Vec<Item>) -> Result<()> {
        for item in items {
            let at = |kind: ItemKind| Item { pos: item.pos, kind };
            match &item.kind {
                ItemKind::Decl(d) => {
                    if let Some((def, elem, n)) = self.vec_bundle(&d.dtype, env, &ctx.at(item.pos))? {
                        for x in &d.decls {
                            self.groups.push((x.name.clone(), n));
                            for k in 0..n {
                                // the bundle elements are named in this iteration (`core_lsu_read_0_1`)
                                let (refs, _) = bundle::local_refs(&def, &elem.targs, &format!("{}_{k}", x.name), env, self.file, item.pos)?;
                                for (name, r) in refs {
                                    env.define_bundle_alias(&name, r);
                                }
                                let element = Decl {
                                    dtype: elem.clone(),
                                    decls: vec![Declarator { name: format!("{}_{k}", x.name), dims: x.dims.clone(), init: None }],
                                    is_const: false,
                                    is_async: false,
                                };
                                for dd in bundle::expand_decl(&def, &element, env, self.file, item.pos)? {
                                    out.push(at(ItemKind::Decl(dd)));
                                }
                            }
                        }
                        continue;
                    }
                    if let Some(def) = self.globals.bundle_def(&d.dtype.name).cloned() {
                        for x in &d.decls {
                            let (refs, _) = bundle::local_refs(&def, &d.dtype.targs, &x.name, env, self.file, item.pos)?;
                            for (name, r) in refs {
                                env.define_bundle_alias(&name, r);
                            }
                        }
                        for dd in bundle::expand_decl(&def, d, env, self.file, item.pos)? {
                            out.push(at(ItemKind::Decl(dd)));
                        }
                        continue;
                    }
                    // a constant with integer values is known at compile time (see above): not a declaration
                    if d.is_const && d.decls.iter().all(|x| x.init.as_ref().is_some_and(|e| int_of(&fold_expr(e, env)).is_some())) {
                        continue;
                    }
                    let decl = elab_decl(d, env, &ctx.at(item.pos))?;
                    // the signals of a stage or a block are known from here on (widths of the later statements)
                    for x in &decl.decls {
                        env.define_signal(&x.name, decl.dtype.clone(), x.dims.clone());
                    }
                    // a register of a generate block is reset too (the module's own were collected up front)
                    if d.dtype.name == "reg" {
                        for x in &d.decls {
                            env.add_reg_init(&x.name, x.init.as_ref().map_or_else(|| Expr::Num("0".into()), |e| fold_expr(e, env)));
                        }
                    }
                    // `wire T x = e;` is a declaration plus a continuous assignment at this place
                    if decl.dtype.name == "wire" && decl.decls.iter().any(|x| x.init.is_some()) {
                        let mut plain = decl.clone();
                        for x in &mut plain.decls {
                            x.init = None;
                        }
                        out.push(at(ItemKind::Decl(plain)));
                        for x in &decl.decls {
                            if let Some(init) = &x.init {
                                let lhs = Expr::Ident(x.name.clone(), item.pos);
                                out.push(at(ItemKind::ContAssign { lhs, rhs: init.clone() }));
                            }
                        }
                    } else {
                        out.push(at(ItemKind::Decl(decl)));
                    }
                }
                ItemKind::ContAssign { lhs, rhs } => {
                    // a `match` in the value is covered at the line of the assignment (an expression has no line of its own)
                    if env.cover_on() {
                        fold::cover_matches(rhs, item.pos, env);
                    }
                    out.push(at(ItemKind::ContAssign { lhs: fold_expr(lhs, env), rhs: fold_expr(rhs, env) }))
                }
                ItemKind::AlwaysComb(body) => {
                    let (body, locals) = scope::hoist_locals(&fold_stmt(body, env), &mut self.local_count);
                    out.extend(locals.into_iter().map(|d| at(ItemKind::Decl(d))));
                    out.push(at(ItemKind::AlwaysComb(body)));
                }
                ItemKind::Initial(body) => {
                    let (body, locals) = scope::hoist_locals(&fold_stmt(body, env), &mut self.local_count);
                    out.extend(locals.into_iter().map(|d| at(ItemKind::Decl(d))));
                    out.push(at(ItemKind::Initial(body)));
                }
                ItemKind::Always { sens, body, keyword } => {
                    let (folded, locals) = scope::hoist_locals(&fold_stmt(body, env), &mut self.local_count);
                    out.extend(locals.into_iter().map(|d| at(ItemKind::Decl(d))));
                    let (sens, body) = match sens {
                        Sens::Edges(edges) if edges.is_empty() => implicit_clocking(env, folded, item.pos),
                        Sens::Edges(edges) => (Sens::Edges(edges.iter().map(|(e, x)| (e.clone(), fold_expr(x, env))).collect()), folded),
                        Sens::Comb => (Sens::Comb, folded),
                    };
                    out.push(at(ItemKind::Always { sens, body, keyword: keyword.clone() }));
                }
                // the reset and clock directives are read from the source module (see elab_module_body)
                ItemKind::Reset { .. } | ItemKind::Clock { .. } => {}
                ItemKind::Val { name, expr } => {
                    // a view of an instruction: `instr.as(Isa.FORMAT)` names the fields of `instr`
                    if let Expr::Call(f, args) = expr {
                        if let (Expr::Member(b, m, _), [Expr::Member(ib, fmt, _)]) = (&**f, args.as_slice()) {
                            if let ("as", Expr::Ident(sig, _), Expr::Ident(isa_name, _)) = (m.as_str(), &**b, &**ib) {
                                if env.isa_info(isa_name).is_some() {
                                    env.define_view(name, sig, isa_name, fmt);
                                    continue;
                                }
                            }
                        }
                    }
                    let value = fold_expr(expr, env);
                    // a name for a signal is an alias of it (no signal of its own), as the reference makes it
                    // (a struct view, `x.as(S)`, keeps its signal: the view's type is on that signal)
                    let is_view = matches!(expr, Expr::Call(f, _) if matches!(&**f, Expr::Member(_, m, _) if m == "as"));
                    // a view inside a generate loop has the name of its iteration (`rel_v_0`): its fields are read from it
                    if let (true, Some(key)) = (is_view, view_struct_key(expr, env)) {
                        env.add_struct_of(name, key);
                    }
                    if let (Expr::Ident(..), false) = (&value, is_view) {
                        env.define_expr(name, value);
                        continue;
                    }
                    // an element of a vector of bundles (or a bundle): the val names its signals, as the reference does
                    if let Some(bundle) = bundle_named_by(&value, env) {
                        env.define_bundle_alias(name, bundle);
                        continue;
                    }
                    // a list is compile-time too: its elements are read by index, or by `select`
                    if let Expr::List(items) = &value {
                        env.define_list(name, items.clone());
                        continue;
                    }
                    // a constant value is compile-time, like a parameter (no signal)
                    if let Some(v) = int_of(&value) {
                        env.define_const(name, v);
                        continue;
                    }
                    // a value whose width is not known is an inline alias, as the reference makes it (no signal)
                    let Some(w) = width::width_of(&value, env) else {
                        env.define_expr(name, value);
                        continue;
                    };
                    let packed = if w.width > 1 { vec![(Expr::Num((w.width - 1).to_string()), Expr::Num("0".into()))] } else { Vec::new() };
                    let dtype = DType { name: "logic".into(), packed, ..DType::default() };
                    env.define_signal(name, dtype.clone(), Vec::new());
                    out.push(at(ItemKind::Decl(Decl { dtype, decls: vec![Declarator { name: name.clone(), dims: Vec::new(), init: None }], is_const: false, is_async: false })));
                    out.push(at(ItemKind::ContAssign { lhs: Expr::Ident(name.clone(), item.pos), rhs: value }));
                }
                ItemKind::GenIf { cond, then, els } => {
                    let cond = fold_expr(cond, env);
                    match int_of(&cond) {
                        Some(v) => {
                            if let Some(branch) = if v != 0 { Some(then) } else { els.as_ref() } {
                                self.elab_items(branch, env, ctx, loops, out)?;
                            }
                        }
                        None => {
                            let mut then_out = Vec::new();
                            self.elab_items(then, env, ctx, loops, &mut then_out)?;
                            let els_out = match els {
                                Some(e) => {
                                    let mut v = Vec::new();
                                    self.elab_items(e, env, ctx, loops, &mut v)?;
                                    Some(v)
                                }
                                None => None,
                            };
                            out.push(at(ItemKind::GenIf { cond, then: then_out, els: els_out }));
                        }
                    }
                }
                ItemKind::GenFor { header, body } if self.generate_bounds(header, body, env, &[]).is_some() => {
                    // a loop of instances and assignments stays a loop: the body is elaborated once, with the loop
                    // variable as a genvar (`for (genvar i = 0; i < 4; i++) begin ... end`)
                    let (lo, hi) = self.generate_bounds(header, body, env, &[]).unwrap_or((0, 0));
                    let mut inner = env.clone();
                    inner.unbind(&header.var);
                    inner.define_signal(&header.var, DType { name: "int".into(), ..DType::default() }, Vec::new());
                    let mut elaborated = Vec::new();
                    self.elab_items(body, &inner, ctx, loops, &mut elaborated)?;
                    let bounds = ForHeader { var: header.var.clone(), lo: Expr::Num(lo.to_string()), bound: Some(Expr::Num(hi.to_string())), cmp: "<".into(), step: None, pos: header.pos };
                    out.push(at(ItemKind::GenFor { header: bounds, body: elaborated }));
                }
                ItemKind::GenFor { header, body } => {
                    let values = loop_values(header, env).ok_or_else(|| ctx.at(item.pos).err("the bounds of a generate loop must be constant"))?;
                    for v in values {
                        let mut inner = env.clone();
                        inner.bind_int(&header.var, v);
                        let mut nested = loops.to_vec();
                        nested.push(v);
                        // what the body declares is local to the iteration: `va` becomes `va_0`, `va_1`, ...
                        let suffix: String = nested.iter().map(|x| format!("_{x}")).collect();
                        let locals: HashMap<String, String> =
                            scope::declared_names(body).into_iter().map(|n| (n.clone(), format!("{n}{suffix}"))).collect();
                        if locals.is_empty() {
                            self.elab_items(body, &inner, ctx, &nested, out)?;
                        } else {
                            self.elab_items(&scope::rename_items(body, &locals), &inner, ctx, &nested, out)?;
                        }
                    }
                }
                ItemKind::Instance(i) => {
                    // a module parameter stands for the module it is bound to
                    let resolved = Instance { module: env.module_alias(&i.module).map_or_else(|| i.module.clone(), str::to_string), ..i.clone() };
                    let module = self.instance_module(&resolved, env, &ctx.at(item.pos))?;
                    let suffix: String = loops.iter().map(|v| format!("_{v}")).collect();
                    let folded: Vec<Conn> = i
                        .conns
                        .iter()
                        .map(|c| Conn { port: c.port.clone(), expr: c.expr.as_ref().map(|e| fold_expr(e, env)) })
                        .collect();
                    let target = self.by_name.get(resolved.module.as_str()).copied();
                    // `.*` is written out: each port of the module connected to the signal of the same name
                    let folded = match target {
                        Some(t) => expand_star(folded, t, env),
                        None => folded,
                    };
                    // an external module's connections are kept as they are
                    let conns = match target {
                        Some(_) => {
                            let spec = self.out.iter().find(|m| m.name == module);
                            bundle::expand_conns(target, spec, &|n| self.globals.bundle_def(n).cloned(), folded, env, self.file, item.pos)?
                        }
                        None => folded,
                    };
                    let targs = if target.is_none() { i.targs.iter().map(|(n, a)| (n.clone(), a.clone())).collect() } else { Vec::new() };
                    out.push(at(ItemKind::Instance(Instance {
                        module,
                        targs,
                        name: format!("{}{suffix}", i.name),
                        conns,
                        pos: i.pos,
                    })));
                }
                // a bundle with no master: what a master drives takes its idle value
                ItemKind::TieOff(target) => {
                    let prefixes = bundle::bundle_prefixes(target, env).unwrap_or_default();
                    let [prefix] = prefixes.as_slice() else {
                        let msg = if prefixes.len() > 1 {
                            "tie_off() takes a bundle signal: name an element of the Vec (v[i]), not the whole vector"
                        } else {
                            "tie_off() takes a bundle signal"
                        };
                        return Err(ctx.at(item.pos).err(msg));
                    };
                    let prefix = prefix.clone();
                    let mut idle = Vec::new();
                    bundle::idle_outputs(env, &prefix, &mut idle);
                    for (name, value) in idle {
                        out.push(at(ItemKind::ContAssign { lhs: Expr::Ident(name, item.pos), rhs: value }));
                    }
                }
                ItemKind::Stage { name, items: stage_items } => {
                    let mut elaborated = Vec::new();
                    self.elab_items(stage_items, env, ctx, loops, &mut elaborated)?;
                    let suffix: String = loops.iter().map(|v| format!("_{v}")).collect();
                    out.push(at(ItemKind::Stage { name: format!("{name}{suffix}"), items: elaborated }));
                }
                ItemKind::Comment(text) => out.push(at(ItemKind::Comment(text.clone()))),
                ItemKind::SameDomain(names) => out.push(at(ItemKind::SameDomain(names.clone()))),
                // expanded by the parser at each call
                ItemKind::Proc { .. } => {}
                // a memory map: no hardware of its own; its devices are read as constants (`io.UART`, `io.count`)
                ItemKind::MemMap { name, desc, entries } => {
                    // the map is read as `name.DEV`, which a signal of the same name would hide
                    if env.signal(name).is_some() || env.bundle_of(name).is_some() {
                        return Err(ctx.err(format!("memmap '{name}' reuses the name of a signal")));
                    }
                    let map = build_memmap(entries, env, ctx)?;
                    self.memmaps.push((name.clone(), desc.clone(), map.devices.clone()));
                    env.define_memmap(name, map);
                }
                ItemKind::Task { name, args, body } => out.push(at(ItemKind::Task {
                    name: name.clone(),
                    args: args.iter().map(|(t, n)| (fold_dtype(t, env), n.clone())).collect(),
                    body: fold::fold_body(body, env),
                })),
                ItemKind::Raw(lines) => out.push(at(ItemKind::Raw(lines.clone()))),
                // functions are inlined at their calls; a function that is called as a method of an instance is
                // kept until the hierarchy pass gives it ports (then removed)
                ItemKind::Fn { name, params, ptypes, ret, body, override_at } => out.push(at(ItemKind::Fn {
                    name: name.clone(),
                    params: params.clone(),
                    ptypes: ptypes.iter().map(|t| fold_dtype(t, env)).collect(),
                    ret: ret.as_ref().map(|r| fold_dtype(r, env)),
                    body: fold_expr(body, env),
                    override_at: *override_at,
                })),
                ItemKind::Fsm { enum_name, name, init, defaults, arms } => {
                    let defaults: Vec<_> = defaults.iter().map(|s| fold_stmt(s, env)).collect();
                    let arms: Vec<_> = arms.iter().map(|(l, s)| (l.clone(), fold_stmt(s, env))).collect();
                    out.extend(fsm::lower(env, item.pos, ctx.file, (enum_name, name, init), &defaults, &arms)?);
                }
                ItemKind::Ctrl { kind, cond } => out.push(at(ItemKind::Ctrl { kind: kind.clone(), cond: fold_expr(cond, env) })),
            }
        }
        Ok(())
    }

    /// The module an instance refers to, specialised when the target is parameterised.
    fn instance_module(&mut self, i: &Instance, env: &Env, ctx: &Ctx) -> Result<String> {
        let ictx = ctx.at(i.pos);
        // a module that is not in the design is external (a library of the simulator, say): kept as it is
        let Some(target) = self.by_name.get(i.module.as_str()).copied() else {
            return Ok(i.module.clone());
        };
        if target.params.is_empty() {
            if !i.targs.is_empty() {
                return Err(ictx.err(format!("'{}' has no parameters", target.name)));
            }
            // emitted once, where it is first instantiated (its own dependencies come first)
            if self.emitted.insert(target.name.clone()) {
                let spec = self.elab_module(target, &Overrides::default(), &target.name)?;
                self.out.push(spec);
            }
            return Ok(target.name.clone());
        }
        let overrides = instance_overrides(target, &i.targs, env, &ictx)?;
        let bound = bind_params(target, &overrides, &self.globals, &ictx)?;
        // a specialisation is named by its values; a config by its name and its overrides (`CpuCfg_lanes4`)
        let values = spec_values(target, &overrides, &bound);
        let key = format!("{}<{}>", target.name, values.join(","));
        if let Some(name) = self.specs.get(&key) {
            return Ok(name.clone());
        }
        let name = format!("{}__{}", target.name, values.join("_"));
        self.specs.insert(key, name.clone());
        let spec = self.elab_module(target, &overrides, &name)?;
        self.out.push(spec);
        Ok(name)
    }
}

/// Names of all modules instantiated in these items, through generate constructs too.
fn collect_instances(items: &[Item], names: &mut HashSet<String>) {
    for item in items {
        match &item.kind {
            ItemKind::Instance(i) => {
                names.insert(i.module.clone());
            }
            ItemKind::GenIf { then, els, .. } => {
                collect_instances(then, names);
                if let Some(e) = els {
                    collect_instances(e, names);
                }
            }
            ItemKind::GenFor { body, .. } => collect_instances(body, names),
            _ => {}
        }
    }
}

fn dir_word(dir: Dir) -> &'static str {
    match dir {
        Dir::Input => "input",
        Dir::Output => "output",
        Dir::Inout => "inout",
        Dir::Master => "master",
        Dir::Slave => "slave",
        Dir::Monitor => "monitor",
    }
}

/// Integer parameters take the override when given, else their default. Parameters are bound in order,
/// so a default may use the parameters before it.
/// The values that name a specialisation: the configs (with their overrides) and the integer parameters.
fn spec_values(target: &Module, overrides: &Overrides, bound: &Env) -> Vec<String> {
    target
        .params
        .iter()
        .filter_map(|p| match p.kind.as_str() {
            "config" => {
                let arg = overrides.configs.get(&p.name).cloned().unwrap_or_else(|| config::ConfigArg { name: default_config(p), overrides: Vec::new() });
                let suffix: String = arg.overrides.iter().map(|(f, v)| format!("_{f}{v}")).collect();
                Some(format!("{}{suffix}", arg.name))
            }
            "module" => bound.module_alias(&p.name).map(str::to_string),
            // a type names its specialisation by its base name and packed bounds: logic_7_0
            "type" => bound.ty(&p.name).map(|t| {
                let packed: String = t.packed.iter().filter_map(|(m, l)| Some(format!("_{}_{}", int_of(m)?, int_of(l)?))).collect();
                // an instance of a parameterized struct is named `Name__args`
                let name = t.name.replace('<', "__").replace(", ", "_").replace('>', "");
                format!("{}{packed}", if name.is_empty() { "logic" } else { &name })
            }),
            _ => match bound.int(&p.name) {
                Some(v) => Some(v.to_string()),
                None => match bound.list(&p.name) {
                    // a list: a short hash of its elements
                    Some(items) => {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        format!("{items:?}").hash(&mut h);
                        Some(format!("L{:06x}", h.finish() & 0xff_ffff))
                    }
                    // a string: its letters and digits
                    None => match bound.global_expr(&p.name) {
                        Some(Expr::Num(text)) => Some(text.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect()),
                        _ => None,
                    },
                },
            },
        })
        .collect()
}

/// The config a config parameter takes when the instance does not name one: its default.
fn default_config(p: &Param) -> String {
    match &p.default {
        Some(Arg::Expr(Expr::Ident(n, _))) => n.clone(),
        _ => p.bound.clone().unwrap_or_default(),
    }
}

/// The tests of `for t in list { test ... }`: one set per row of the list, the row bound to the loop variable.
fn expand_test_loop(l: &crate::ast::TestLoop, binds: &[(String, Expr)], globals: &Env, file: &str, out: &mut Vec<crate::ast::TestDef>) -> Result<()> {
    let names: Vec<String> = binds.iter().map(|(n, _)| n.clone()).collect();
    let values: Vec<Expr> = binds.iter().map(|(_, v)| v.clone()).collect();
    let fail = |msg: String| Error::new(msg, file, l.pos.0, l.pos.1);
    let Expr::List(rows) = fold_expr(&substitute(&l.iter, &names, &values), globals) else {
        return Err(fail("a test loop runs over a constant list (the rows of a manifest)".into()));
    };
    for row in rows {
        let mut inner = binds.to_vec();
        if l.names.is_empty() {
            inner.push((l.var.clone(), row));
        } else {
            let Expr::List(items) = &row else { return Err(fail("the rows of a test loop with (a, b) names are lists".into())) };
            inner.extend(l.names.iter().cloned().zip(items.iter().cloned()));
        }
        let names: Vec<String> = inner.iter().map(|(n, _)| n.clone()).collect();
        let values: Vec<Expr> = inner.iter().map(|(_, v)| v.clone()).collect();
        let sub = |e: &Expr| fold_expr(&substitute(e, &names, &values), globals);
        for t in &l.tests {
            let name = match &t.name_expr {
                Some(e) => match sub(e) {
                    Expr::Num(s) if s.starts_with('"') => s.trim_matches('"').to_string(),
                    _ => return Err(Error::new("the name of a test in a loop is a string", &t.origin, t.pos.0, t.pos.1)),
                },
                None => t.name.clone(),
            };
            let targs = t
                .targs
                .iter()
                .map(|a| match a {
                    Arg::Expr(e) => Arg::Expr(sub(e)),
                    other => other.clone(),
                })
                .collect();
            let body = t
                .body
                .iter()
                .map(|s| {
                    crate::pipeline::rewrite::map_stmt(s, &mut |x| {
                        Ok(match x {
                            Expr::Ident(n, _) => names.iter().position(|v| v == n).map(|k| values[k].clone()),
                            _ => None,
                        })
                    })
                    .unwrap_or_else(|_| s.clone())
                })
                .collect();
            out.push(crate::ast::TestDef { name, name_expr: None, targs, timeout: t.timeout.as_ref().map(sub), body, ..t.clone() });
        }
        for sub_loop in &l.loops {
            expand_test_loop(sub_loop, &inner, globals, file, out)?;
        }
    }
    Ok(())
}

/// A memory map: its devices, constant and checked, and the interrupt lines in their numbers.
fn build_memmap(entries: &[crate::ast::MapEntryDef], env: &Env, ctx: &Ctx) -> Result<env::MapVal> {
    let constant = |x: &Expr, what: &str, dev: &str| {
        int_of(&fold_expr(x, env)).ok_or_else(|| ctx.err(format!("the {what} of '{dev}' must be constant")))
    };
    let mut devices: Vec<env::MapDev> = Vec::new();
    let mut lines: BTreeMap<i64, Expr> = BTreeMap::new();
    for e in entries {
        if let Some(cond) = &e.cond {
            if constant(cond, "condition", &e.name)? == 0 {
                continue;
            }
        }
        if ["count", "regions", "irqs"].contains(&e.name.as_str()) {
            return Err(ctx.err(format!("memmap: '{}' is reserved, rename the device", e.name)));
        }
        let base = constant(&e.base, "base", &e.name)?;
        let size = constant(&e.size, "size", &e.name)?;
        let irq = e.irq.as_ref().map(|x| constant(x, "irq", &e.name)).transpose()?;
        if let Some(line) = &e.line {
            let k = irq.ok_or_else(|| ctx.err(format!("device '{}': an interrupt line needs 'irq'", e.name)))?;
            lines.insert(k, fold_expr(line, env));
        }
        devices.push(env::MapDev { name: e.name.clone(), base, size, irq, compatible: e.compatible.clone(), dt: e.dt.clone(), desc: e.desc.clone() });
    }
    for (i, a) in devices.iter().enumerate() {
        if a.size <= 0 {
            return Err(ctx.err(format!("device '{}': size must be positive", a.name)));
        }
        for b in &devices[i + 1..] {
            if a.base < b.base + b.size && b.base < a.base + a.size {
                return Err(ctx.err(format!(
                    "devices '{}' ('h{:x}..'h{:x}) and '{}' ('h{:x}..'h{:x}) overlap",
                    a.name,
                    a.base,
                    a.base + a.size - 1,
                    b.name,
                    b.base,
                    b.base + b.size - 1
                )));
            }
            if a.irq.is_some() && a.irq == b.irq {
                return Err(ctx.err(format!("devices '{}' and '{}' both use interrupt {}", a.name, b.name, a.irq.unwrap_or(0))));
            }
        }
    }
    // the interrupt lines as one vector, the highest line first, at least 32 bits
    let width = lines.keys().next_back().map_or(32, |k| (k + 1).max(32));
    let irqs = Expr::Concat((0..width).rev().map(|k| lines.get(&k).cloned().unwrap_or_else(|| Expr::Num("1'b0".into()))).collect());
    Ok(env::MapVal { devices, irqs })
}

/// What an instance gives its module's parameters: integers, lists, and configs with their overrides.
#[derive(Default)]
struct Overrides {
    lists: HashMap<String, Vec<Expr>>,
    exprs: HashMap<String, Expr>,
    ints: HashMap<String, i64>,
    configs: HashMap<String, config::ConfigArg>,
    modules: HashMap<String, String>,
    types: HashMap<String, DType>,
}

fn bind_params(m: &Module, overrides: &Overrides, globals: &Env, ctx: &Ctx) -> Result<Env> {
    let mut env = globals.scope();
    let mut fns = globals.fns();
    fns.extend(m.items.iter().filter_map(|it| match &it.kind {
        ItemKind::Fn { name, params, ret, body, .. } => {
            Some((name.clone(), FnDef { params: params.clone(), ret: ret.clone(), body: body.clone() }))
        }
        _ => None,
    }));
    env.set_fns(fns);
    for p in &m.params {
        match (p.kind.as_str(), p.default.as_ref()) {
            ("type", _) if overrides.types.contains_key(&p.name) => env.bind_type(&p.name, overrides.types[&p.name].clone()),
            ("int" | "bool", _) if overrides.ints.contains_key(&p.name) => env.bind_int(&p.name, overrides.ints[&p.name]),
            ("list", _) if overrides.lists.contains_key(&p.name) => env.define_list(&p.name, overrides.lists[&p.name].clone()),
            (_, _) if overrides.exprs.contains_key(&p.name) => env.define_expr(&p.name, overrides.exprs[&p.name].clone()),
            ("module", _) => {
                let module = match overrides.modules.get(&p.name) {
                    Some(m) => m.clone(),
                    None => match &p.default {
                        Some(Arg::Expr(Expr::Ident(n, _))) => n.clone(),
                        _ => return Err(ctx.err(format!("module parameter '{}' needs a default module", p.name))),
                    },
                };
                env.bind_module(&p.name, &module);
            }
            ("config", _) => {
                let name = default_config(p);
                let arg = overrides.configs.get(&p.name).cloned().unwrap_or(config::ConfigArg { name, overrides: Vec::new() });
                let values = config::evaluate(&arg.name, globals, &arg.overrides, ctx.file, ctx.pos)?;
                env.bind_config(&p.name, arg, values);
            }
            ("type", Some(Arg::Type(t))) => {
                let t = fold_dtype(t, &env);
                env.bind_type(&p.name, t);
            }
            ("list", Some(Arg::Expr(e))) => match fold_expr(e, &env) {
                Expr::List(items) => env.define_list(&p.name, items),
                _ => return Err(ctx.err(format!("the default of parameter '{}' is not a constant list", p.name))),
            },
            ("int" | "bool", Some(Arg::Expr(e))) => match int_of(&fold_expr(e, &env)) {
                Some(v) => env.bind_int(&p.name, v),
                None => env.define_expr(&p.name, fold_expr(e, &env)),
            },
            ("type" | "int" | "bool", None) => {
                return Err(ctx.err(format!("parameter '{}' has no default value", p.name)));
            }
            (kind, _) => return Err(ctx.err(format!("parameters of kind '{kind}' are not supported yet"))),
        }
    }
    Ok(env)
}

/// The integer values an instance gives to the target's parameters, positional or named.
fn instance_overrides(target: &Module, targs: &[(Option<String>, Arg)], env: &Env, ctx: &Ctx) -> Result<Overrides> {
    let mut out = Overrides::default();
    let mut next_positional = 0;
    for (name, arg) in targs {
        let param: &Param = match name {
            Some(n) => target.params.iter().find(|p| &p.name == n).ok_or_else(|| ctx.err(format!("'{}' has no parameter '{n}'", target.name)))?,
            None => {
                next_positional += 1;
                target.params.get(next_positional - 1).ok_or_else(|| ctx.err(format!("too many generic arguments for '{}'", target.name)))?
            }
        };
        match (param.kind.as_str(), arg) {
            ("module", Arg::Expr(Expr::Ident(n, _))) => {
                out.modules.insert(param.name.clone(), n.clone());
            }
            ("type", Arg::Type(t)) => {
                // a struct stays a struct: its key names the specialisation (its packed type would not)
                let bound = match fold::struct_key(t, env) {
                    Some(key) => DType { name: key, ..DType::default() },
                    None => fold_dtype(t, env),
                };
                out.types.insert(param.name.clone(), bound);
            }
            ("type", Arg::Expr(Expr::Ident(n, _))) if fold::struct_key(&DType { name: n.clone(), ..DType::default() }, env).is_some() => {
                let key = fold::struct_key(&DType { name: n.clone(), ..DType::default() }, env).unwrap_or_default();
                out.types.insert(param.name.clone(), DType { name: key, ..DType::default() });
            }
            ("type", Arg::Expr(Expr::Ident(n, _))) if env.ty(n).is_some() => {
                let t = env.ty(n).cloned().expect("checked");
                out.types.insert(param.name.clone(), t);
            }
            // a configuration parameter of the instantiating module is passed on with its overrides
            ("config", Arg::Expr(Expr::Ident(n, _))) if env.config_arg(n).is_some() => {
                out.configs.insert(param.name.clone(), env.config_arg(n).unwrap_or(config::ConfigArg { name: n.clone(), overrides: Vec::new() }));
            }
            ("config", Arg::Expr(Expr::Ident(n, _))) => {
                out.configs.insert(param.name.clone(), config::ConfigArg { name: n.clone(), overrides: Vec::new() });
            }
            ("config", Arg::Expr(Expr::With(base, overrides, _))) => {
                let Expr::Ident(n, _) = &**base else { return Err(ctx.err("a configuration with overrides names the config first")) };
                let mut values = Vec::new();
                for (field, v) in overrides {
                    let value = int_of(&fold_expr(v, env))
                        .ok_or_else(|| ctx.err(format!("the override of '{field}' is not a constant")))?;
                    values.push((field.clone(), value));
                }
                out.configs.insert(param.name.clone(), config::ConfigArg { name: n.clone(), overrides: values });
            }
            (_, Arg::Expr(e)) if param.kind == "list" => match fold_expr(e, env) {
                Expr::List(items) => {
                    out.lists.insert(param.name.clone(), items);
                }
                _ => return Err(ctx.err(format!("the value of parameter '{}' is not a constant list", param.name))),
            },
            (_, Arg::Expr(e)) if matches!(param.kind.as_str(), "int" | "bool") => match int_of(&fold_expr(e, env)) {
                Some(value) => {
                    out.ints.insert(param.name.clone(), value);
                }
                None => {
                    out.exprs.insert(param.name.clone(), fold_expr(e, env));
                }
            },
            _ => return Err(ctx.err("type parameters of instances are not supported yet")),
        }
    }
    Ok(out)
}

/// The base names of the variables a statement assigns.
pub(crate) fn assigned_names(s: &Stmt, out: &mut BTreeSet<String>) {
    match s {
        Stmt::Assign { lhs, .. } => {
            let mut base = lhs;
            loop {
                match base {
                    Expr::Index(b, _) | Expr::Slice(b, _, _, _) | Expr::Member(b, _, _) => base = b,
                    Expr::Ident(n, _) => {
                        out.insert(n.clone());
                        break;
                    }
                    _ => break,
                }
            }
        }
        Stmt::Block(stmts) => stmts.iter().for_each(|x| assigned_names(x, out)),
        Stmt::If { then, els, .. } => {
            assigned_names(then, out);
            if let Some(e) = els {
                assigned_names(e, out);
            }
        }
        Stmt::For { body, .. } => assigned_names(body, out),
        Stmt::Case { arms, .. } => arms.iter().for_each(|(_, b)| assigned_names(b, out)),
        _ => {}
    }
}

/// The reset of a module from its port names: `rst`, `reset` (active high), or `rst_n`, `reset_n`, `resetn` (active low).
fn default_reset(ports: &[Port]) -> Option<ResetSpec> {
    let names = [("rst", false), ("reset", false), ("rst_n", true), ("reset_n", true), ("resetn", true)];
    names.into_iter().find(|(n, _)| ports.iter().any(|p| p.name == *n)).map(|(n, low)| ResetSpec { name: n.to_string(), low, is_async: false })
}

/// The file a module comes from (the design's file when it has none).
fn ctx_file<'a>(m: &'a Module, default: &'a str) -> &'a str {
    if m.origin.is_empty() { default } else { &m.origin }
}

/// The first identifier of the elaborated module that names nothing: not a port, a declaration, an instance, a task.
/// Instance connections are not checked (their names are the child's), nor the system functions (`$signed`).
/// The similarity of two names, as `difflib` computes it: twice the size of the matching blocks over the total length.
fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    2.0 * matching(&a, &b) as f64 / (a.len() + b.len()) as f64
}

/// The number of characters in matching blocks, found the way `difflib` finds them (the longest block first, then the
/// blocks on each side of it).
fn matching(a: &[char], b: &[char]) -> usize {
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
fn known_names(module: &Module) -> Vec<String> {
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
fn member_error(module: &Module, env: &Env) -> Option<(String, Pos)> {
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

fn unknown_identifier(module: &Module) -> Option<(String, Pos)> {
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
fn assigned_name(lhs: &Expr) -> Option<String> {
    match lhs {
        Expr::Ident(name, _) => Some(name.clone()),
        Expr::Index(base, _) => assigned_name(base),
        _ => None,
    }
}

/// The variables a statement assigns.
fn assigned_in_stmt(s: &Stmt, out: &mut Vec<String>) {
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
fn written_by_ff_and_initial(module: &Module) -> Option<(String, Pos)> {
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
fn visit_item(it: &Item, visit: &dyn Fn(&Expr) -> std::result::Result<Option<Expr>, crate::pipeline::rewrite::Failure>) {
    let _ = rewrite_item(it, visit);
    if let ItemKind::Val { expr, .. } = &it.kind {
        let mut f = |e: &Expr| visit(e);
        let _ = crate::pipeline::rewrite::map_expr(expr, &mut f);
    }
}

/// A part-select past the width of an expression (`(a + b)[9:4]` of a 9-bit sum): SV would take the missing bits as
/// nothing, so it is an error. The first one, as a message.
fn select_past_width(items: &[Item], env: &Env) -> Option<String> {
    let found: RefCell<Option<String>> = RefCell::new(None);
    for it in items {
        visit_item(it, &|e| {
            if let Expr::Slice(base, msb, lsb, op) = e {
                if op.as_str() == ":" && fold::is_expression(&**base) && found.borrow().is_none() {
                    if let (Some(m), Some(l), Some(w)) = (int_of(&fold_expr(&**msb, env)), int_of(&fold_expr(&**lsb, env)), width::width_of(&**base, env)) {
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
fn view_width_error(items: &[Item], env: &Env) -> Option<String> {
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
                        if let (Some(s), Some(w)) = (key.as_ref().and_then(|k| env.struct_info(k)), width::width_of(&**base, env)) {
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
fn declared_in_stmt(s: &Stmt, known: &mut HashSet<String>) {
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
fn idents_in(e: &Expr, out: &mut Vec<(String, Pos)>) {
    let _ = crate::pipeline::rewrite::map_expr(e, &mut |x| {
        if let Expr::Ident(name, pos) = x {
            out.push((name.clone(), *pos));
        }
        Ok(None)
    });
}

fn idents_in_stmt(s: &Stmt, out: &mut Vec<(String, Pos)>) {
    let _ = crate::pipeline::rewrite::map_stmt(s, &mut |x| {
        if let Expr::Ident(name, pos) = x {
            out.push((name.clone(), *pos));
        }
        Ok(None)
    });
}

/// The first input port that the module drives: an assignment whose target starts from it (`a`, `a[3]`, `a.b`).
fn driven_input(module: &Module) -> Option<(String, Pos)> {
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
fn assigns_in_expr(target: &Expr, out: &mut Vec<(String, Pos)>) {
    match target {
        Expr::Ident(name, pos) => out.push((name.clone(), *pos)),
        Expr::Index(base, _) | Expr::Slice(base, _, _, _) | Expr::Member(base, _, _) => assigns_in_expr(base, out),
        _ => {}
    }
}

fn assigns_in_stmt(s: &Stmt, out: &mut Vec<(String, Pos)>) {
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

/// The bundle a value names: a bundle signal, or an element `v[k]` of a vector of bundles (`v_k`).
fn bundle_named_by(value: &Expr, env: &Env) -> Option<BundleRef> {
    match value {
        Expr::Ident(name, _) => env.bundle_of(name),
        Expr::Index(base, idx) => {
            let Expr::Ident(vec, _) = &**base else { return None };
            let k = int_of(idx)?;
            env.bundle_of(&format!("{vec}_{k}"))
        }
        _ => None,
    }
}

/// `always_ff { ... }` without an event: the module's clock and reset, and the registers reset to their
/// initial values.
fn implicit_clocking(env: &Env, body: Stmt, pos: Pos) -> (Sens, Stmt) {
    let reset = env.reset().cloned().unwrap_or(ResetSpec { name: "rst".into(), low: false, is_async: false });
    let mut edges = vec![("posedge".to_string(), Expr::Ident("clk".into(), pos))];
    if reset.is_async {
        let edge = if reset.low { "negedge" } else { "posedge" };
        edges.push((edge.to_string(), Expr::Ident(reset.name.clone(), pos)));
    }
    // the registers the block writes
    let mut written = BTreeSet::new();
    assigned_names(&body, &mut written);
    let resets: Vec<Stmt> = env
        .reg_inits()
        .into_iter()
        .filter(|(name, _)| written.contains(name))
        .flat_map(|(name, init)| {
            // an array is reset element by element
            let dims: Option<Vec<i64>> = env.signal(&name).and_then(|(_, dims)| dims.iter().map(int_of).collect());
            let mut targets = vec![Expr::Ident(name.clone(), pos)];
            for n in dims.unwrap_or_default() {
                targets = targets.into_iter().flat_map(|t| (0..n).map(move |i| Expr::Index(Box::new(t.clone()), Box::new(Expr::Num(i.to_string()))))).collect();
            }
            targets.into_iter().map(|lhs| Stmt::Assign { lhs, rhs: init.clone(), nonblocking: true, pos: (0, 0) }).collect::<Vec<_>>()
        })
        .collect();
    let body = if resets.is_empty() {
        body
    } else {
        let active = Expr::Ident(reset.name.clone(), pos);
        let cond = if reset.low { Expr::Unary("!".into(), Box::new(active)) } else { active };
        Stmt::If { cond, then: Box::new(Stmt::Block(resets)), els: Some(Box::new(body)) }
    };
    (Sens::Edges(edges), body)
}

fn elab_decl(d: &Decl, env: &Env, ctx: &Ctx) -> Result<Decl> {
    let (mut dtype, extra) = types::lower(fold_dtype(&d.dtype, env)).map_err(|e| ctx.err(e))?;
    // `reg` is a `logic` whose initial value is its reset value (see `implicit_clocking`)
    let is_reg = d.dtype.name == "reg";
    if is_reg {
        dtype.name = "logic".into();
    }
    let decls = d
        .decls
        .iter()
        .map(|x| {
            let mut dims = extra.clone();
            dims.extend(x.dims.iter().map(|e| fold_expr(e, env)));
            let init = if is_reg { None } else { x.init.as_ref().map(|e| fold_expr(e, env)) };
            Declarator { name: x.name.clone(), dims, init }
        })
        .collect();
    Ok(Decl { dtype, decls, is_const: d.is_const, is_async: d.is_async })
}
