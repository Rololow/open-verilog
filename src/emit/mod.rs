//! SystemVerilog output for an elaborated module. Layout follows the reference emitter (ovc/emit_sv.py).

pub mod expr;

use crate::ast::{Arg, Conn, Decl, Expr, Instance, Item, ItemKind, Module, Sens, Stmt};
use crate::error::{Error, Result};
use expr::{dims, dir_keyword, dtype, expr};

/// Modules in order, separated by a blank line.
pub fn emit_modules(modules: &[Module], file: &str) -> Result<String> {
    let mut out = String::new();
    for (i, m) in modules.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let text = emit_module(m).map_err(|mut e| {
            e.file = if m.origin.is_empty() { file.to_string() } else { m.origin.clone() };
            if e.line == 0 {
                (e.line, e.col) = m.pos;
            }
            e
        })?;
        out += &text;
    }
    Ok(out)
}

fn emit_module(m: &Module) -> Result<String> {
    let mut e = Emitter::default();
    if m.ports.is_empty() {
        e.line(&format!("module {};", m.name));
    } else {
        e.line(&format!("module {} (", m.name));
        e.ind += 1;
        for (i, p) in m.ports.iter().enumerate() {
            let default = match &p.default {
                Some(d) => format!(" = {}", expr(d, -1, false)?),
                None => String::new(),
            };
            let comma = if i + 1 < m.ports.len() { "," } else { "" };
            e.line(&format!("{} {} {}{}{default}{comma}", dir_keyword(p.dir), dtype(&p.dtype)?, p.name, dims(&p.dims)?));
        }
        e.ind -= 1;
        e.line(");");
    }
    e.ind += 1;
    e.items(&m.items)?;
    e.ind -= 1;
    e.line("endmodule");
    Ok(e.out)
}

#[derive(Default)]
struct Emitter {
    out: String,
    ind: usize,
    /// The generate loops written so far in the module: each block gets its own name.
    blocks: usize,
}

impl Emitter {
    /// One line at the current indentation; an empty string is a blank line.
    fn line(&mut self, s: &str) {
        if !s.is_empty() {
            self.out += &"  ".repeat(self.ind);
            self.out += s;
        }
        self.out.push('\n');
    }

    fn items(&mut self, items: &[Item]) -> Result<()> {
        for item in items {
            self.item(item)?;
        }
        Ok(())
    }

    fn item(&mut self, item: &Item) -> Result<()> {
        match &item.kind {
            ItemKind::Decl(d) => self.decl(d),
            ItemKind::ContAssign { lhs, rhs } => {
                let s = format!("assign {} = {};", expr(lhs, -1, false)?, expr(rhs, -1, false)?);
                self.line(&s);
                Ok(())
            }
            ItemKind::AlwaysComb(body) => self.always("always_comb", body),
            ItemKind::Initial(body) => self.always("initial", body),
            ItemKind::Always { sens, body, keyword } => {
                let sens = match sens {
                    Sens::Edges(edges) => {
                        let parts: Result<Vec<String>> = edges.iter().map(|(edge, x)| Ok(format!("{edge} {}", expr(x, -1, false)?))).collect();
                        parts?.join(" or ")
                    }
                    Sens::Comb => return Err(Error::new("always with a combinational sensitivity", "", 0, 0)),
                };
                self.always(&format!("{keyword} @({sens})"), body)
            }
            ItemKind::Instance(i) => self.instance(i),
            ItemKind::GenFor { header, body } => {
                let bound = header.bound.as_ref().ok_or_else(|| Error::new("a generate loop needs a range", "", 0, 0))?;
                let v = &header.var;
                let block = format!("gen{}", self.blocks);
                self.blocks += 1;
                self.line(&format!("for (genvar {v} = {}; {v} {} {}; {v}++) begin : {block}", expr(&header.lo, -1, false)?, header.cmp, expr(bound, -1, false)?));
                self.ind += 1;
                self.items(body)?;
                self.ind -= 1;
                self.line("end");
                Ok(())
            }
            ItemKind::GenIf { cond, then, els } => {
                self.line(&format!("if ({}) begin", expr(cond, -1, false)?));
                self.ind += 1;
                self.items(then)?;
                self.ind -= 1;
                if let Some(els) = els {
                    self.line("end else begin");
                    self.ind += 1;
                    self.items(els)?;
                    self.ind -= 1;
                }
                self.line("end");
                Ok(())
            }
            ItemKind::Comment(text) => {
                self.line(&format!("// {text}"));
                Ok(())
            }
            ItemKind::SameDomain(names) => {
                self.line(&format!("// same clock domain: {}", names.join(", ")));
                Ok(())
            }
            ItemKind::Raw(lines) => {
                for line in lines {
                    self.line(line);
                }
                Ok(())
            }
            ItemKind::Task { name, args, body } => {
                let mut params = Vec::new();
                for (ty, arg) in args {
                    params.push(format!("input {} {arg}", dtype(ty)?));
                }
                self.line(&format!("task {name}({});", params.join(", ")));
                self.ind += 1;
                for s in body {
                    self.stmt(s)?;
                }
                self.ind -= 1;
                self.line("endtask");
                Ok(())
            }
            ItemKind::TieOff(_) | ItemKind::Stage { .. } | ItemKind::Ctrl { .. } | ItemKind::Fn { .. } | ItemKind::Proc { .. } | ItemKind::MemMap { .. } | ItemKind::Fsm { .. } | ItemKind::Reset { .. } | ItemKind::Clock { .. } | ItemKind::Val { .. } => {
                Err(Error::new("this item is not supported in SystemVerilog output yet", "", 0, 0))
            }
        }
    }

    fn decl(&mut self, d: &Decl) -> Result<()> {
        let kw = if d.is_const { "localparam " } else { "" };
        for x in &d.decls {
            let init = match &x.init {
                Some(e) => format!(" = {}", expr(e, -1, false)?),
                None => String::new(),
            };
            self.line(&format!("{kw}{} {}{}{init};", dtype(&d.dtype)?, x.name, dims(&x.dims)?));
        }
        Ok(())
    }

    fn instance(&mut self, i: &Instance) -> Result<()> {
        let targs = if i.targs.is_empty() {
            String::new()
        } else {
            let args: Result<Vec<String>> = i
                .targs
                .iter()
                .map(|(name, arg)| {
                    let value = match arg {
                        Arg::Type(t) => dtype(t)?,
                        Arg::Expr(e) => expr(e, -1, false)?,
                    };
                    Ok(match name {
                        Some(n) => format!(".{n}({value})"),
                        None => value,
                    })
                })
                .collect();
            format!(" #({})", args?.join(", "))
        };
        let conns: Result<Vec<String>> = i.conns.iter().map(conn).collect();
        self.line(&format!("{}{targs} {} ({});", i.module, i.name, conns?.join(", ")));
        Ok(())
    }

    fn always(&mut self, header: &str, body: &Stmt) -> Result<()> {
        self.line(&format!("{header} begin"));
        self.stmts_of(body)?;
        self.line("end");
        Ok(())
    }

    /// The statements of a block at one level deeper, or the single statement.
    fn stmts_of(&mut self, s: &Stmt) -> Result<()> {
        self.ind += 1;
        match s {
            Stmt::Block(stmts) => {
                for x in stmts {
                    self.stmt(x)?;
                }
            }
            other => self.stmt(other)?,
        }
        self.ind -= 1;
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<()> {
        match s {
            Stmt::Block(_) => {
                self.line("begin");
                self.stmts_of(s)?;
                self.line("end");
            }
            Stmt::Assign { lhs, rhs, nonblocking, .. } => {
                let op = if *nonblocking { "<=" } else { "=" };
                let text = format!("{} {op} {};", expr(lhs, -1, false)?, expr(rhs, -1, false)?);
                self.line(&text);
            }
            Stmt::If { cond, then, els } => {
                self.line(&format!("if ({}) begin", expr(cond, -1, false)?));
                self.stmts_of(then)?;
                let mut next: Option<&Stmt> = els.as_deref();
                while let Some(s) = next {
                    match s {
                        Stmt::If { cond, then, els } => {
                            self.line(&format!("end else if ({}) begin", expr(cond, -1, false)?));
                            self.stmts_of(then)?;
                            next = els.as_deref();
                        }
                        other => {
                            self.line("end else begin");
                            self.stmts_of(other)?;
                            next = None;
                        }
                    }
                }
                self.line("end");
            }
            Stmt::For { header, body } => {
                let bound = header.bound.as_ref().ok_or_else(|| Error::new("a range loop inside a block is not supported yet", "", 0, 0))?;
                let v = &header.var;
                let incr = match &header.step {
                    None => format!("{v}++"),
                    Some(crate::ast::Expr::Num(one)) if one == "1" => format!("{v}++"),
                    Some(step) => format!("{v} += {}", expr(step, -1, false)?),
                };
                self.line(&format!(
                    "for (int {v} = {}; {v} {} {}; {incr}) begin",
                    expr(&header.lo, -1, false)?,
                    header.cmp,
                    expr(bound, -1, false)?
                ));
                self.stmts_of(body)?;
                self.line("end");
            }
            Stmt::Case { subject, kind, arms, .. } => {
                self.line(&format!("{} ({})", case_keyword(kind, arms), expr(subject, -1, false)?));
                self.ind += 1;
                for (labels, body) in arms {
                    let label = match labels {
                        None => "default".to_string(),
                        Some(ls) => {
                            let parts: Result<Vec<String>> = ls.iter().map(|x| expr(x, -1, false)).collect();
                            parts?.join(", ")
                        }
                    };
                    self.line(&format!("{label}: begin"));
                    self.stmts_of(body)?;
                    self.line("end");
                }
                self.ind -= 1;
                self.line("endcase");
            }
            Stmt::ExprStmt(e) => {
                self.line(&format!("{};", expr(e, -1, false)?));
                // a failed check of the design fails the tests too (the testbench counts `ovc_check_failed`)
                if matches!(e, Expr::Call(f, _) if matches!(&**f, Expr::Ident(name, _) if name == "$error")) {
                    self.line("`ifdef OVC_TESTS");
                    self.line("->ovc_tests.ovc_check_failed;");
                    self.line("`endif");
                }
            }
            Stmt::Decl(d) => self.decl(d)?,
            Stmt::Return(_) | Stmt::Goto(_) | Stmt::Val(..) => {
                return Err(Error::new("this statement is not supported in SystemVerilog output yet", "", 0, 0));
            }
        }
        Ok(())
    }
}

/// The keyword of a case statement. Its labels are distinct constants, so at most one arm can match: `unique0` says
/// so to the tools (and the simulator checks it). Any other label, or a repeated value, keeps the plain keyword.
fn case_keyword(kind: &str, arms: &[(Option<Vec<Expr>>, Stmt)]) -> String {
    if kind != "case" {
        return kind.to_string();
    }
    let mut seen: Vec<i64> = Vec::new();
    for (labels, _) in arms {
        for label in labels.iter().flatten() {
            let Expr::Num(text) = label else { return kind.to_string() };
            match crate::lexer::number_value(text) {
                Some(v) if !seen.contains(&v) => seen.push(v),
                _ => return kind.to_string(),
            }
        }
    }
    format!("unique0 {kind}")
}

fn conn(c: &Conn) -> Result<String> {
    Ok(match (&c.port, &c.expr) {
        (Some(p), _) if p == "*" => ".*".to_string(),
        (Some(p), Some(e)) => format!(".{p}({})", expr(e, -1, false)?),
        (Some(p), None) => format!(".{p}()"),
        (None, Some(e)) => expr(e, -1, false)?,
        (None, None) => String::new(),
    })
}
