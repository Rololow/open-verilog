//! Pipeline lowering: combinational stages become registers between stages, one valid bit per stage, the carried
//! values and the control (stall, flush, kill). A stage may read earlier stages (the values are carried through
//! registers), and `stage.x` / `pipe.x` select a value explicitly.

pub(crate) mod arrays;
mod checks;
mod control;
pub(crate) mod rewrite;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use checks::Subject;
use control::{Carried, Conditions};
use rewrite::{map_expr, map_stmt, Failure};

use crate::ast::{DType, Decl, Declarator, Dir, Expr, Item, ItemKind, Module, Pos, Port};
use crate::emit::expr::{dims, dtype, expr};
use crate::error::{Error, Result};

/// A signal of the pipeline and the stage that defines it. Inputs enter with the instruction (stage 0).
#[derive(Clone)]
struct Signal {
    stage: usize,
    dtype: DType,
    dims: Vec<Expr>,
    input: bool,
}

/// A stage: its name and its items.
struct Stage<'a> {
    name: &'a str,
    items: &'a [Item],
}

/// A lowered pipeline: the module, the stage table and the warnings.
pub struct Lowered {
    pub module: Module,
    pub table: String,
    pub warnings: Vec<String>,
}

/// Lowers a pipeline module to a plain module.
/// The clock and reset a pipeline names with its directives (`clock NAME;`, `reset NAME [low];`), if it has them.
#[derive(Default)]
pub struct Directives {
    pub clock: Option<String>,
    pub reset: Option<(String, bool)>,
    /// `reset NAME [low] async;`: the registers are cleared by the reset edge, not by the clock.
    pub async_reset: bool,
}

/// The directives of a pipeline module, read from its source.
pub fn directives(m: &Module) -> Result<Directives> {
    let mut out = Directives::default();
    for item in &m.items {
        match &item.kind {
            ItemKind::Clock { name } => out.clock = Some(name.clone()),
            ItemKind::Reset { name, low, is_async } => {
                out.reset = Some((name.clone(), *low));
                out.async_reset = *is_async;
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Lowers a pipeline module to a plain module. The clock and reset are the directives', else the ports'.
pub fn lower(source: &Module, file: &str, asserts: bool, directives: &Directives, bundle_signals: &[String]) -> Result<Lowered> {
    let scalar = arrays::scalarize(source, file)?;
    let m = &scalar;
    let err = |pos: Pos, msg: String| Error::new(msg, file, pos.0, pos.1);
    let stages: Vec<Stage> = m
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            ItemKind::Stage { name, items } => Some(Stage { name, items }),
            _ => None,
        })
        .collect();
    if stages.is_empty() {
        return Err(err(m.pos, format!("pipeline '{}' has no stages", m.name)));
    }
    let ports: HashMap<&str, &Port> = m.ports.iter().map(|p| (p.name.as_str(), p)).collect();
    // the clock and the reset are the ones the pipeline declares: a name is never guessed from its ports
    let Some(clock) = directives.clock.clone() else {
        return Err(err(m.pos, format!("pipeline '{}' has no clock: declare `clock clk;`", m.name)));
    };
    if !ports.contains_key(clock.as_str()) {
        return Err(err(m.pos, format!("pipeline '{}' is clocked by '{clock}', which is not a port", m.name)));
    }
    let reset = directives.reset.clone();
    if let Some((name, _)) = &reset {
        if !ports.contains_key(name.as_str()) {
            return Err(err(m.pos, format!("pipeline '{}' is reset by '{name}', which is not a port", m.name)));
        }
    }
    if stages.len() > 1 && reset.is_none() {
        return Err(err(m.pos, format!("pipeline '{}' has no reset: declare `reset rst;`", m.name)));
    }
    // the reset as the condition that clears the registers, and as the text the checks use (inverted when low)
    let reset_name = reset.as_ref().map_or_else(String::new, |(n, _)| n.clone());
    let reset_cond = reset.as_ref().map(|(n, low)| if *low { Expr::Unary("!".into(), Box::new(Expr::Ident(n.clone(), m.pos))) } else { Expr::Ident(n.clone(), m.pos) });
    // a pipeline without a reset has no reset in its checks: `1'b0` never holds
    let reset_text = match &reset {
        Some((n, true)) => format!("!{n}"),
        Some((n, false)) => n.clone(),
        None => "1'b0".to_string(),
    };
    let control_names = [clock.as_str(), reset_name.as_str(), "valid_in", "valid_out", "ready_out"];
    let owners = find_owners(m, &stages, &ports, &control_names).map_err(|(pos, msg)| err(pos, msg))?;

    let mut lowering = Lowering { stages: &stages, owners: &owners, uses: BTreeMap::new(), fires: BTreeSet::new(), data_inputs: Vec::new(), file, bundle_signals };
    let (body, controls) = lowering.body(m)?;
    let n = stages.len();
    let names: Vec<&str> = stages.iter().map(|s| s.name).collect();
    let valid: Vec<String> = names.iter().map(|s| format!("ov_valid_{s}")).collect();
    let fires: Vec<usize> = lowering.fires.iter().copied().collect();
    let module_pos = m.pos;
    let plan = control::plan(&names, &valid, &controls, &fires, module_pos);
    let carried: Vec<Carried> = lowering
        .uses
        .iter()
        .map(|(name, last)| Carried { name: name.clone(), owner: owners[name].stage, last: *last })
        .collect();

    let generated = |kind: ItemKind| Item { pos: module_pos, kind };
    let mut out = Vec::new();
    for v in &valid {
        out.push(generated(decl_of(logic(), v.clone(), Vec::new())));
    }
    for c in &carried {
        let sig = &owners[&c.name];
        for j in c.owner + 1..=c.last {
            out.push(generated(decl_of(sig.dtype.clone(), format!("{}__s{j}", c.name), sig.dims.clone())));
        }
    }
    for stage in &stages {
        for it in stage.items {
            if let ItemKind::Decl(d) = &it.kind {
                let decls = d.decls.iter().map(|x| Declarator { name: x.name.clone(), dims: x.dims.clone(), init: None }).collect();
                out.push(Item {
                    pos: it.pos,
                    kind: ItemKind::Decl(Decl { dtype: d.dtype.clone(), decls, is_const: d.is_const, is_async: d.is_async }),
                });
            }
        }
    }
    for (name, _) in &plan.wires {
        out.push(generated(decl_of(logic(), name.clone(), Vec::new())));
    }
    for (name, value) in &plan.wires {
        out.push(generated(ItemKind::ContAssign { lhs: Expr::Ident(name.clone(), module_pos), rhs: value.clone() }));
    }
    // `valid_in` and `ready_out` are ports, or signals the pipeline declares (a producer inside the module)
    let declared: std::collections::HashSet<&str> = m
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            ItemKind::Decl(d) => Some(d.decls.iter().map(|x| x.name.as_str())),
            _ => None,
        })
        .flatten()
        .collect();
    let has_valid_in = ports.contains_key("valid_in") || declared.contains("valid_in");
    let has_ready_out = ports.contains_key("ready_out") || declared.contains("ready_out");
    let valid_in = if has_valid_in { Expr::Ident("valid_in".into(), module_pos) } else { Expr::Num("1'b1".into()) };
    out.push(generated(ItemKind::ContAssign { lhs: Expr::Ident(valid[0].clone(), module_pos), rhs: valid_in }));
    out.extend(body);
    // the clock edge, and the reset edge when the reset is asynchronous (`reset rstn low async;`)
    let mut edges = vec![("posedge".to_string(), Expr::Ident(clock.clone(), module_pos))];
    if directives.async_reset {
        if let Some((name, low)) = &reset {
            let edge = if *low { "negedge" } else { "posedge" };
            edges.push((edge.to_string(), Expr::Ident(name.clone(), module_pos)));
        }
    }
    for item in control::boundaries(&plan, &names, &valid, &carried, &edges, reset_cond, module_pos) {
        out.push(item);
    }
    if ports.contains_key("valid_out") {
        let rhs = plan.valid_out(&valid[n - 1], module_pos);
        out.push(generated(ItemKind::ContAssign { lhs: Expr::Ident("valid_out".into(), module_pos), rhs }));
    }
    let mut warnings = Vec::new();
    if has_ready_out {
        out.push(generated(ItemKind::ContAssign { lhs: Expr::Ident("ready_out".into(), module_pos), rhs: plan.ready_out() }));
    } else if plan.hold[0].is_some() {
        warnings.push(format!(
            "{file}:{}:{}: warning: pipeline '{}' can stall but has no 'ready_out' port: the producer cannot know when it must \
             hold its data, so inputs would be lost",
            m.pos.0, m.pos.1, m.name
        ));
    }

    if asserts {
        let subject = Subject {
            name: &m.name,
            names: &names,
            clock: clock.as_str(),
            rst: reset_text.as_str(),
            has_valid_in,
            data_inputs: &lowering.data_inputs,
        };
        let lines = checks::checks(&subject, &plan, &valid, &carried)?;
        out.push(generated(ItemKind::Raw(lines)));
    }

    // declarations first: the logic below uses them
    let (mut decls, rest): (Vec<Item>, Vec<Item>) = out.into_iter().partition(|i| matches!(i.kind, ItemKind::Decl(_)));
    decls.extend(rest);
    let out = decls;

    let table = report(m, &stages, &owners, &carried, &controls)?;
    let module = Module {
        enums: Vec::new(),
        origin: m.origin.clone(),
        kind: "module".into(),
        name: m.name.clone(),
        params: Vec::new(),
        ports: m.ports.clone(),
        extends: None,
        traits: Vec::new(),
        items: out,
        pos: m.pos,
    };
    Ok(Lowered { module, table, warnings })
}

fn logic() -> DType {
    DType { name: "logic".into(), ..DType::default() }
}

fn decl_of(dtype: DType, name: String, dims: Vec<Expr>) -> ItemKind {
    ItemKind::Decl(Decl { dtype, decls: vec![Declarator { name, dims, init: None }], is_const: false, is_async: false })
}

/// Which stage defines each signal. Inputs (other than the control ports) belong to stage 0.
fn find_owners(m: &Module, stages: &[Stage], ports: &HashMap<&str, &Port>, control: &[&str]) -> std::result::Result<HashMap<String, Signal>, (Pos, String)> {
    let pipeline_decls: HashMap<&str, (&DType, &Vec<Expr>)> = m
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            ItemKind::Decl(d) => Some(d.decls.iter().map(move |x| (x.name.as_str(), (&d.dtype, &x.dims)))),
            _ => None,
        })
        .flatten()
        .collect();
    let mut owners: HashMap<String, Signal> = HashMap::new();
    for p in &m.ports {
        if p.dir == Dir::Input && !control.contains(&p.name.as_str()) {
            owners.insert(p.name.clone(), Signal { stage: 0, dtype: p.dtype.clone(), dims: p.dims.clone(), input: true });
        }
    }
    for (k, stage) in stages.iter().enumerate() {
        for it in stage.items {
            match &it.kind {
                ItemKind::Decl(d) => {
                    for x in &d.decls {
                        if owners.contains_key(&x.name) {
                            return Err((it.pos, format!("signal '{}' is defined twice (stage '{}')", x.name, stage.name)));
                        }
                        owners.insert(x.name.clone(), Signal { stage: k, dtype: d.dtype.clone(), dims: x.dims.clone(), input: false });
                    }
                }
                ItemKind::ContAssign { lhs: Expr::Ident(name, _), .. } => {
                    // an output port, or a signal declared at pipeline level, becomes owned by the stage driving it
                    let (dtype, dims) = match (ports.get(name.as_str()), pipeline_decls.get(name.as_str())) {
                        (Some(p), _) if p.dir == Dir::Input => {
                            return Err((it.pos, format!("input port '{name}' is driven in stage '{}'", stage.name)));
                        }
                        (Some(p), _) => (p.dtype.clone(), p.dims.clone()),
                        (None, Some((dtype, dims))) => ((*dtype).clone(), (*dims).clone()),
                        (None, None) => continue,
                    };
                    match owners.get(name) {
                        None => {
                            owners.insert(name.clone(), Signal { stage: k, dtype, dims, input: false });
                        }
                        Some(s) if s.stage != k => {
                            return Err((it.pos, format!("'{name}' is defined in stage '{}' but driven in '{}'", stages[s.stage].name, stage.name)));
                        }
                        Some(_) => {}
                    }
                }
                ItemKind::ContAssign { .. } | ItemKind::Comment(_) | ItemKind::Ctrl { .. } => {}
                other => return Err((it.pos, format!("a stage holds only assignments and declarations, not {}", item_word(other)))),
            }
        }
    }
    Ok(owners)
}

fn item_word(kind: &ItemKind) -> &'static str {
    match kind {
        ItemKind::Always { .. } => "a sequential block (always)",
        ItemKind::Proc { .. } => "a procedure",
        ItemKind::Task { .. } => "a task",
        ItemKind::MemMap { .. } => "a memory map",
        ItemKind::AlwaysComb(_) => "an always_comb block",
        ItemKind::Instance(_) => "an instance",
        ItemKind::GenIf { .. } => "a generate if",
        ItemKind::GenFor { .. } => "a generate for",
        ItemKind::TieOff(_) => "tie_off",
        ItemKind::Stage { .. } => "a nested stage",
        ItemKind::Comment(_) | ItemKind::Raw(_) => "a comment",
        ItemKind::SameDomain(_) => "a same_domain declaration",
        ItemKind::Initial(_) => "an initial block",
        ItemKind::Fn { .. } => "a function",
        ItemKind::Fsm { .. } => "an fsm",
        ItemKind::Reset { .. } => "a reset directive",
        ItemKind::Clock { .. } => "a clock directive",
        ItemKind::Val { .. } => "a val",
        ItemKind::Decl(_) => "a declaration",
        ItemKind::ContAssign { .. } => "an assignment",
        ItemKind::Ctrl { .. } => "a stall or flush condition",
    }
}

/// Resolves the references of a pipeline. Records, for each carried value, the last stage that reads it, and the
/// stages whose `stage.fire` is used.
struct Lowering<'a, 'm> {
    stages: &'m [Stage<'a>],
    owners: &'m HashMap<String, Signal>,
    /// name -> last stage that reads the carried copy of `name`
    uses: BTreeMap<String, usize>,
    fires: BTreeSet<usize>,
    /// input ports read unqualified in a stage, in order of first use
    data_inputs: Vec<String>,
    file: &'m str,
    /// the signals of the bundle ports (`o_ready`): read as they are, not as the instruction's input
    bundle_signals: &'m [String],
}

impl<'a> Lowering<'a, '_> {
    fn stage_index(&self, name: &str) -> Option<usize> {
        self.stages.iter().position(|s| s.name == name)
    }

    /// The stage items without control conditions and declarations, the control conditions per stage, and the
    /// items outside stages with references resolved.
    fn body(&mut self, m: &Module) -> Result<(Vec<Item>, Vec<Conditions>)> {
        let mut body = Vec::new();
        let mut controls = vec![Conditions::default(); self.stages.len()];
        for (k, stage) in self.stages.iter().enumerate() {
            for it in stage.items {
                match &it.kind {
                    ItemKind::Decl(d) => {
                        for x in &d.decls {
                            if let Some(init) = &x.init {
                                let rhs = self.resolve(init, Some(k), it.pos)?;
                                body.push(Item { pos: it.pos, kind: ItemKind::ContAssign { lhs: Expr::Ident(x.name.clone(), it.pos), rhs } });
                            }
                        }
                    }
                    ItemKind::ContAssign { lhs, rhs } => {
                        let rhs = self.resolve(rhs, Some(k), it.pos)?;
                        body.push(Item { pos: it.pos, kind: ItemKind::ContAssign { lhs: lhs.clone(), rhs } });
                    }
                    ItemKind::Ctrl { kind, cond } => {
                        let cond = self.resolve(cond, Some(k), it.pos)?;
                        match kind.as_str() {
                            "stall" => controls[k].stall.push(cond),
                            "flush" => controls[k].flush.push(cond),
                            _ => controls[k].younger.push(cond),
                        }
                    }
                    _ => {}
                }
            }
        }
        for it in &m.items {
            match &it.kind {
                ItemKind::Stage { .. } => {}
                // a procedure is compile-time: its calls are expanded where they are made
                ItemKind::Proc { .. } => {}
                // an array no stage drives stays an array (scalarized arrays are already split)
                ItemKind::Decl(_) => body.push(it.clone()),
                // a pure function is compile-time: it is inlined at its calls
                ItemKind::Fn { .. } => body.push(it.clone()),
                // the checks and the coverage the module generated: kept as they are
                ItemKind::Raw(_) => body.push(it.clone()),
                // an instance outside the stages: its connections may name stage values
                ItemKind::Instance(inst) => {
                    let mut conns = Vec::with_capacity(inst.conns.len());
                    for c in &inst.conns {
                        let expr = match &c.expr {
                            Some(e) => Some(self.resolve(e, None, it.pos)?),
                            None => None,
                        };
                        conns.push(crate::ast::Conn { port: c.port.clone(), expr });
                    }
                    body.push(Item { pos: it.pos, kind: ItemKind::Instance(crate::ast::Instance { conns, ..inst.clone() }) });
                }
                ItemKind::ContAssign { lhs, rhs } => {
                    let rhs = self.resolve(rhs, None, it.pos)?;
                    body.push(Item { pos: it.pos, kind: ItemKind::ContAssign { lhs: lhs.clone(), rhs } });
                }
                ItemKind::Always { sens, body: stmt, keyword } => {
                    let stmt = map_stmt(stmt, &mut |e| self.resolve_node(e, None)).map_err(|(at, msg)| self.failure(at, it.pos, msg))?;
                    body.push(Item { pos: it.pos, kind: ItemKind::Always { sens: sens.clone(), body: stmt, keyword: keyword.clone() } });
                }
                ItemKind::Ctrl { kind, .. } => {
                    let word = match kind.as_str() {
                        "stall" => "stall_when",
                        "flush" => "flush_when",
                        _ => "flush_younger_when",
                    };
                    return Err(Error::new(format!("'{word}' must be inside a stage"), self.file, it.pos.0, it.pos.1));
                }
                // a comment is no hardware
                ItemKind::Comment(_) => {}
                other => {
                    return Err(Error::new(format!("{} is not supported in a pipeline yet", item_word(other)), self.file, it.pos.0, it.pos.1));
                }
            }
        }
        Ok((body, controls))
    }

    /// An error at the failing name when known, else at the item.
    fn failure(&self, at: Pos, item: Pos, msg: String) -> Error {
        let pos = if at.0 == 0 { item } else { at };
        Error::new(msg, self.file, pos.0, pos.1)
    }

    fn resolve(&mut self, e: &Expr, cur: Option<usize>, pos: Pos) -> Result<Expr> {
        map_expr(e, &mut |node| self.resolve_node(node, cur)).map_err(|(at, msg)| self.failure(at, pos, msg))
    }

    /// `x` in stage `m` reads the value of `x` in `m`: carried when `x` is defined earlier, an error when later.
    /// `s.x` reads `x` in stage `s`: carried when defined earlier, forwarded combinationally when defined later.
    /// `pipe.x` is the raw input port; `s.valid` is the valid bit of stage `s`; `s.fire` the fire signal of `s`.
    fn resolve_node(&mut self, node: &Expr, cur: Option<usize>) -> std::result::Result<Option<Expr>, Failure> {
        match node {
            Expr::Member(base, field, _) => {
                let Expr::Ident(target, _) = &**base else { return Ok(None) };
                if target == "pipe" {
                    return Ok(Some(Expr::Ident(field.clone(), (0, 0))));
                }
                let Some(t) = self.stage_index(target) else { return Ok(None) };
                match field.as_str() {
                    "valid" => return Ok(Some(Expr::Ident(format!("ov_valid_{target}"), (0, 0)))),
                    "fire" => {
                        self.fires.insert(t);
                        return Ok(Some(Expr::Ident(format!("ov_fire_{target}"), (0, 0))));
                    }
                    _ => {}
                }
                let Some(sig) = self.owners.get(field) else {
                    return Err(((0, 0), format!("'{field}' is not a pipeline signal")));
                };
                Ok(Some(self.value_in(field, sig.stage, t)))
            }
            Expr::Ident(name, at) => {
                // the valid bit of the stage a statement is in
                if name == "valid" && !self.owners.contains_key(name) {
                    if let Some(m) = cur {
                        return Ok(Some(Expr::Ident(format!("ov_valid_{}", self.stages[m].name), *at)));
                    }
                }
                let (Some(m), Some(sig)) = (cur, self.owners.get(name)) else { return Ok(None) };
                // a bundle port is read as it is now: the value of the producer, not one carried from the instruction
                if sig.input && self.bundle_signals.contains(name) {
                    return Ok(Some(Expr::Ident(name.clone(), *at)));
                }
                if sig.input && sig.stage == 0 && m > 0 {
                    let here = self.stages[m].name;
                    return Err((*at, format!(
                        "input '{name}' is read in stage '{here}', but an input enters the pipeline with the instruction: \
                         write '{here}.{name}' for the value that came with this instruction, or 'pipe.{name}' for its value now"
                    )));
                }
                if sig.stage > m {
                    let later = self.stages[sig.stage].name;
                    return Err((*at, format!("'{name}' is defined in stage '{later}' (a later stage); write '{later}.{name}' for explicit forwarding")));
                }
                let stage = sig.stage;
                if sig.input && !self.data_inputs.contains(name) {
                    self.data_inputs.push(name.clone());
                }
                Ok(Some(self.value_in(name, stage, m)))
            }
            _ => Ok(None),
        }
    }

    /// The name of `name` (defined in stage `owner`) as read in stage `t`.
    fn value_in(&mut self, name: &str, owner: usize, t: usize) -> Expr {
        if owner < t {
            let last = self.uses.entry(name.to_string()).or_insert(t);
            *last = (*last).max(t);
            return Expr::Ident(format!("{name}__s{t}"), (0, 0));
        }
        Expr::Ident(name.to_string(), (0, 0))
    }
}

/// The stage table printed by `--dump-pipeline`.
fn report(m: &Module, stages: &[Stage], owners: &HashMap<String, Signal>, carried: &[Carried], controls: &[Conditions]) -> Result<String> {
    let n = stages.len();
    let latency = n - 1;
    let mut text = format!(
        "pipeline {}: {n} stage{}, latency {latency} cycle{} (valid_in -> valid_out)\n",
        m.name,
        if n == 1 { "" } else { "s" },
        if latency == 1 { "" } else { "s" }
    );
    text += &format!("  {:<12}{:<26}{:<44}{}\n", "stage", "defines", "carries in", "control");
    for (k, stage) in stages.iter().enumerate() {
        let mut defines: Vec<&str> = owners.iter().filter(|(_, s)| s.stage == k && !s.input).map(|(name, _)| name.as_str()).collect();
        defines.sort_unstable();
        let mut carries = Vec::new();
        for c in carried {
            if c.owner < k && k <= c.last {
                carries.push(format!("{}: {}{}", c.name, dtype(&owners[&c.name].dtype)?, dims(&owners[&c.name].dims)?));
            }
        }
        let carries = if carries.is_empty() { "-".to_string() } else { carries.join(", ") };
        let mut control = Vec::new();
        for (kind, conds) in [("stall", &controls[k].stall), ("flush", &controls[k].flush), ("kills earlier stages", &controls[k].younger)] {
            if !conds.is_empty() {
                let texts: Result<Vec<String>> = conds.iter().map(|c| expr(c, -1, false)).collect();
                control.push(format!("{kind} on {}", texts?.join(" || ")));
            }
        }
        let control = if control.is_empty() { "-".to_string() } else { control.join("; ") };
        text += &format!("  {:<12}{:<26}{:<44}{}\n", stage.name, defines.join(", "), carries, control);
    }
    Ok(text)
}
