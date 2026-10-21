//! Module items: declarations, assignments, always blocks, instances and generate constructs.

use super::{Parser, BASE_TYPES, GENERIC_PREC, UNSUPPORTED};
use crate::ast::{Conn, Decl, Declarator, Expr, Instance, Item, ItemKind, MapEntryDef, Sens, Stmt, DType};
use crate::error::Result;
use crate::lexer::Kind;

impl Parser {
    /// Items up to `closer` (not consumed).
    pub(super) fn parse_items(&mut self, closer: &str) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        while !self.at(closer) {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail(format!("'{closer}' missing")));
            }
            items.extend(self.parse_item()?);
        }
        Ok(items)
    }

    /// A generate branch: a block of items, or one item.
    fn parse_gen_body(&mut self) -> Result<Vec<Item>> {
        if self.at_block() {
            let closer = self.block_open()?;
            let items = self.parse_items(closer)?;
            self.block_close(closer)?;
            return Ok(items);
        }
        self.parse_item()
    }

    fn parse_item(&mut self) -> Result<Vec<Item>> {
        let pos = (self.tok().line, self.tok().col);
        if self.accept("generate") {
            // the block's items keep their own positions
            let items = self.parse_items("endgenerate")?;
            self.block_close("endgenerate")?;
            return Ok(items);
        }
        Ok(self.parse_item_kind()?.into_iter().map(|kind| Item { pos, kind }).collect())
    }

    pub(super) fn parse_item_kind(&mut self) -> Result<Vec<ItemKind>> {
        let t = self.tok().clone();
        if t.text == "begin" {
            let then = self.parse_gen_body()?;
            return Ok(vec![ItemKind::GenIf { cond: Expr::Num("1".into()), then, els: None }]);
        }
        if t.kind != Kind::Ident {
            let found = if t.kind == Kind::Eof { "end of file" } else { &t.text };
            return Err(self.fail(format!("module item expected, found '{found}'")));
        }
        match t.text.as_str() {
            "genvar" => {
                self.i += 1;
                self.comma_list(";", |p| p.ident())?;
                Ok(vec![])
            }
            "assign" => {
                self.i += 1;
                let lhs = self.parse_postfix()?;
                self.expect("=")?;
                let rhs = self.parse_expr(0)?;
                self.expect(";")?;
                Ok(vec![ItemKind::ContAssign { lhs, rhs }])
            }
            "initial" => {
                self.i += 1;
                Ok(vec![ItemKind::Initial(self.parse_stmt()?)])
            }
            "always_comb" => {
                self.i += 1;
                Ok(vec![ItemKind::AlwaysComb(self.parse_stmt()?)])
            }
            "always_ff" => {
                self.i += 1;
                let sens = if self.at("@") { self.parse_sens()? } else { Sens::Edges(Vec::new()) };
                let body = self.parse_ff_body()?;
                Ok(vec![ItemKind::Always { sens, body, keyword: t.text }])
            }
            "always" => {
                self.i += 1;
                match self.parse_sens()? {
                    Sens::Comb => Ok(vec![ItemKind::AlwaysComb(self.parse_stmt()?)]),
                    sens => {
                        let body = self.parse_ff_body()?;
                        Ok(vec![ItemKind::Always { sens, body, keyword: t.text }])
                    }
                }
            }
            "if" => {
                self.i += 1;
                self.expect("(")?;
                let cond = self.parse_expr(0)?;
                self.expect(")")?;
                let then = self.parse_gen_body()?;
                let els = if self.accept("else") { Some(self.parse_gen_body()?) } else { None };
                Ok(vec![ItemKind::GenIf { cond, then, els }])
            }
            "for" => {
                let header = self.parse_for_header()?;
                let pre = std::mem::take(&mut self.unpack);
                let mut body = self.parse_gen_body()?;
                if !pre.is_empty() {
                    let pos = header.pos;
                    let mut items: Vec<Item> = pre.into_iter().map(|(name, expr)| Item { pos, kind: ItemKind::Val { name, expr } }).collect();
                    items.append(&mut body);
                    body = items;
                }
                Ok(vec![ItemKind::GenFor { header, body }])
            }
            "val" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                self.expect("=")?;
                let expr = self.parse_expr(0)?;
                self.expect(";")?;
                Ok(vec![ItemKind::Val { name, expr }])
            }
            "val" if self.peek(1).text == "(" => {
                let pos = (self.tok().line, self.tok().col);
                self.i += 1;
                let names = self.tuple_names()?;
                self.expect("=")?;
                let expr = self.parse_expr(0)?;
                self.expect(";")?;
                Ok(self.tuple_bindings(&names, expr, pos).into_iter().map(|(name, expr)| ItemKind::Val { name, expr }).collect())
            }
            "reset" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                let (mut low, mut is_async) = (false, false);
                while !self.accept(";") {
                    match self.ident()?.as_str() {
                        "low" => low = true,
                        "high" => low = false,
                        "async" => is_async = true,
                        "sync" => is_async = false,
                        other => return Err(self.fail(format!("unknown reset option '{other}'"))),
                    }
                }
                Ok(vec![ItemKind::Reset { name, low, is_async }])
            }
            "clock" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                self.expect(";")?;
                Ok(vec![ItemKind::Clock { name }])
            }
            "fsm" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let enum_name = self.ident()?;
                let name = self.ident()?;
                self.expect("=")?;
                let init = self.ident()?;
                let closer = self.block_open()?;
                let mut defaults = Vec::new();
                let mut arms = Vec::new();
                while !self.at(closer) {
                    if self.tok().kind == Kind::Eof {
                        return Err(self.fail(format!("'{closer}' missing")));
                    }
                    if self.tok().kind == Kind::Ident && self.peek(1).text == ":" {
                        let label = self.ident()?;
                        self.expect(":")?;
                        arms.push((label, self.parse_stmt()?));
                    } else {
                        defaults.push(self.parse_stmt()?);
                    }
                }
                self.block_close(closer)?;
                Ok(vec![ItemKind::Fsm { enum_name, name, init, defaults, arms }])
            }
            "override" if self.peek(1).text == "fn" => {
                // an override is a function that replaces the one of the base module
                self.i += 1;
                self.parse_item_kind()
            }
            "fn" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                self.expect("(")?;
                let typed = self.comma_list(")", |p| {
                    let ty = p.parse_dtype(true)?;
                    Ok((ty, p.ident()?))
                })?;
                let (ptypes, params): (Vec<DType>, Vec<String>) = typed.into_iter().unzip();
                let ret = if self.accept("->") { Some(self.parse_dtype(false)?) } else { None };
                let start = self.i;
                match self.parse_fn_body("}") {
                    Ok(body) => Ok(vec![ItemKind::Fn { name, params, ptypes, ret, body }]),
                    Err(err) => {
                        // not a pure function: a procedural one, whose statements are inlined at its calls
                        self.i = start;
                        match self.parse_block_statements() {
                            Ok(body) => Ok(vec![ItemKind::Proc { name, params, body }]),
                            Err(_) => Err(err),
                        }
                    }
                }
            }
            "memmap" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                let desc = self.opt_string();
                let closer = self.block_open()?;
                let mut entries = Vec::new();
                while !self.at(closer) {
                    if self.tok().kind == Kind::Eof {
                        return Err(self.fail(format!("'{closer}' missing")));
                    }
                    let pos = (self.tok().line, self.tok().col);
                    let ename = self.ident()?;
                    self.expect("@")?;
                    let base = self.parse_expr(0)?;
                    if self.ident()? != "size" {
                        return Err(self.fail("'size' expected after the base address"));
                    }
                    let size = self.parse_expr(0)?;
                    let mut e = MapEntryDef {
                        name: ename,
                        base,
                        size,
                        irq: None,
                        line: None,
                        compatible: String::new(),
                        dt: String::new(),
                        desc: String::new(),
                        cond: None,
                        pos,
                    };
                    while !self.accept(";") {
                        if self.accept("irq") {
                            e.irq = Some(self.parse_expr(0)?);
                            if self.accept("=") {
                                e.line = Some(self.parse_expr(0)?);
                            }
                        } else if self.accept("compatible") {
                            e.compatible = self.opt_string();
                        } else if self.accept("dt") {
                            e.dt = self.opt_string();
                        } else if self.accept("if") {
                            self.expect("(")?;
                            e.cond = Some(self.parse_expr(0)?);
                            self.expect(")")?;
                        } else if self.tok().kind == Kind::Str {
                            e.desc = self.opt_string();
                        } else {
                            return Err(self.fail(format!(
                                "memmap entry '{}': 'irq', 'compatible', 'dt', 'if', a description or ';' expected, found '{}'",
                                e.name,
                                self.found()
                            )));
                        }
                    }
                    entries.push(e);
                }
                self.block_close(closer)?;
                Ok(vec![ItemKind::MemMap { name, desc, entries }])
            }
            "task" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                self.expect("(")?;
                let args = self.comma_list(")", |p| {
                    p.accept("input");
                    let ty = p.parse_dtype(false)?;
                    Ok((ty, p.ident()?))
                })?;
                self.accept(";");
                let mut body = Vec::new();
                while !self.accept("endtask") {
                    if self.tok().kind == Kind::Eof {
                        return Err(self.fail("'endtask' missing"));
                    }
                    body.push(self.parse_stmt()?);
                }
                Ok(vec![ItemKind::Task { name, args, body }])
            }
            "stage" if self.peek(1).kind == Kind::Ident => {
                self.i += 1;
                let name = self.ident()?;
                let closer = self.block_open()?;
                let items = self.parse_items(closer)?;
                self.block_close(closer)?;
                Ok(vec![ItemKind::Stage { name, items }])
            }
            "stall_when" | "flush_when" | "flush_younger_when" if self.peek(1).text == "(" => {
                let kind = match t.text.as_str() {
                    "stall_when" => "stall",
                    "flush_when" => "flush",
                    _ => "younger",
                };
                self.i += 2;
                let cond = self.parse_expr(0)?;
                self.expect(")")?;
                self.expect(";")?;
                Ok(vec![ItemKind::Ctrl { kind: kind.into(), cond }])
            }
            "tie_off" if self.peek(1).text == "(" => {
                self.i += 2;
                let target = self.parse_expr(0)?;
                self.expect(")")?;
                self.expect(";")?;
                Ok(vec![ItemKind::TieOff(target)])
            }
            "async" => {
                self.i += 1;
                let mut d = self.parse_decl()?;
                d.is_async = true;
                Ok(vec![ItemKind::Decl(d)])
            }
            "enum" => {
                // a module's enum is global: it is hoisted to the file
                let def = self.parse_enum_def()?;
                self.hoisted_enums.push(def);
                Ok(vec![])
            }
            k if UNSUPPORTED.contains(&k) =>Err(self.fail(format!("'{k}' is not supported yet"))),
            k if matches!(k, "const" | "localparam" | "parameter") || BASE_TYPES.contains(&k) => Ok(vec![ItemKind::Decl(self.parse_decl()?)]),
            _ if self.looks_like_instance() => Ok(vec![ItemKind::Instance(self.parse_instance()?)]),
            _ => Ok(vec![ItemKind::Decl(self.parse_decl()?)]),
        }
    }

    pub(super) fn parse_ff_body(&mut self) -> Result<Stmt> {
        let saved = std::mem::replace(&mut self.in_ff, true);
        let body = self.parse_stmt();
        self.in_ff = saved;
        body
    }

    /// `@(posedge clk or negedge rst)` is edge-sensitive; `@(*)` or any level signal is combinational.
    pub(super) fn parse_sens(&mut self) -> Result<Sens> {
        self.expect("@")?;
        if self.accept("*") {
            return Ok(Sens::Comb);
        }
        self.expect("(")?;
        if self.accept("*") {
            self.expect(")")?;
            return Ok(Sens::Comb);
        }
        let mut edges = Vec::new();
        let mut level = false;
        loop {
            if self.at("posedge") || self.at("negedge") {
                let edge = self.ident()?;
                edges.push((edge, self.parse_expr(0)?));
            } else {
                self.parse_expr(0)?;
                level = true;
            }
            if !(self.accept("or") || self.accept(",")) {
                break;
            }
        }
        self.expect(")")?;
        Ok(if level { Sens::Comb } else { Sens::Edges(edges) })
    }

    /// `[const] type name[dims] [= init], ...;`
    pub(super) fn parse_decl(&mut self) -> Result<Decl> {
        let mut is_const = false;
        for kw in ["const", "localparam", "parameter"] {
            is_const |= self.accept(kw);
        }
        let dtype = self.parse_dtype(is_const)?;
        let mut decls = Vec::new();
        loop {
            let name = self.ident()?;
            let dims = self.parse_dims()?;
            let init = if self.accept("=") { Some(self.parse_expr(0)?) } else { None };
            decls.push(Declarator { name, dims, init });
            if !self.accept(",") {
                break;
            }
        }
        self.expect(";")?;
        Ok(Decl { dtype, decls, is_const, is_async: false })
    }

    /// Does the statement at the current token start an instance (`Mod [<...>|#(...)] name(`)?
    fn looks_like_instance(&self) -> bool {
        let mut n = 1;
        if matches!(self.peek(n).text.as_str(), "<" | "#") {
            let (open, close) = if self.peek(n).text == "<" { ("<", ">") } else { ("(", ")") };
            if open == "(" {
                n += 1;
            }
            let mut depth: i32 = 0;
            while self.peek(n).kind != Kind::Eof {
                let tx = self.peek(n).text.as_str();
                depth += (tx == open) as i32 - (tx == close) as i32;
                n += 1;
                if depth == 0 {
                    break;
                }
            }
        }
        self.peek(n).kind == Kind::Ident && self.peek(n + 1).text == "("
    }

    fn parse_instance(&mut self) -> Result<Instance> {
        let pos = (self.tok().line, self.tok().col);
        let module = self.ident()?;
        let mut targs = Vec::new();
        if self.accept("<") {
            targs = self.comma_list(">", |p| Ok((None, p.parse_targ(GENERIC_PREC)?)))?;
        } else if self.accept("#") {
            self.expect("(")?;
            targs = self.comma_list(")", |p| p.parse_named_targ())?;
        }
        let name = self.ident()?;
        self.expect("(")?;
        let conns = self.comma_list(")", |p| p.parse_conn())?;
        self.expect(";")?;
        Ok(Instance { module, targs, name, conns, pos })
    }

    /// `.*`, `.port`, `.port(expr)`, `.port()` or a positional expression.
    fn parse_conn(&mut self) -> Result<Conn> {
        if self.at(".") && self.peek(1).text == "*" {
            self.i += 2;
            return Ok(Conn { port: Some("*".into()), expr: None });
        }
        if self.accept(".") {
            let (line, col) = (self.tok().line, self.tok().col);
            let port = self.ident()?;
            if !self.at("(") {
                // .clk is shorthand for .clk(clk)
                return Ok(Conn { expr: Some(Expr::Ident(port.clone(), (line, col))), port: Some(port) });
            }
            self.expect("(")?;
            let expr = if self.at(")") { None } else { Some(self.parse_expr(0)?) };
            self.expect(")")?;
            return Ok(Conn { port: Some(port), expr });
        }
        Ok(Conn { port: None, expr: Some(self.parse_expr(0)?) })
    }
}
