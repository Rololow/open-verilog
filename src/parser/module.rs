//! Module headers: parameters, ports, `extends` and traits, then the body.

use super::{Parser, GENERIC_PREC, PARAM_KINDS};
use crate::ast::{Arg, Dir, DType, Expr, Module, Param, Port};
use crate::error::Result;
use crate::lexer::Kind;

impl Parser {
    pub(super) fn parse_module(&mut self) -> Result<Module> {
        let pos = (self.tok().line, self.tok().col);
        let kind = self.ident()?;
        let name = self.ident()?;
        while self.at("import") {
            self.skip_import()?;
        }
        let mut params = Vec::new();
        if self.accept("<") {
            params = self.comma_list(">", |p| p.parse_param(GENERIC_PREC))?;
        } else if self.accept("#") {
            self.expect("(")?;
            params = self.comma_list(")", |p| p.parse_param(0))?;
        }
        let mut extends = self.parse_extends()?;
        let mut ports = Vec::new();
        if self.accept("(") {
            ports = self.parse_ports()?;
        }
        extends = self.parse_extends()?.or(extends);
        let mut traits = Vec::new();
        if self.accept(":") {
            traits.push(self.ident()?);
            while self.accept(",") {
                traits.push(self.ident()?);
            }
        }
        let closer = if self.accept(";") { "endmodule" } else { self.block_open()? };
        let enums_from = self.hoisted_enums.len();
        let items = super::fnbody::expand_procs(self.parse_items(closer)?);
        let enums = self.hoisted_enums.split_off(enums_from);
        self.block_close(closer)?;
        Ok(Module { kind, name, params, ports, extends, traits, items, pos, origin: self.file.clone(), enums })
    }

    pub(super) fn parse_extends(&mut self) -> Result<Option<(String, Vec<Arg>)>> {
        if !self.accept("extends") {
            return Ok(None);
        }
        let base = self.ident()?;
        let args = if self.accept("<") { self.comma_list(">", |p| p.parse_targ(GENERIC_PREC))? } else { Vec::new() };
        Ok(Some((base, args)))
    }

    /// Skips `[msb:lsb]` on a parameter: parameter widths are ignored, the value is what counts.
    fn skip_param_range(&mut self) -> Result<()> {
        while self.accept("[") {
            self.parse_expr(0)?;
            self.expect(":")?;
            self.parse_expr(0)?;
            self.expect("]")?;
        }
        Ok(())
    }

    pub(super) fn parse_param(&mut self, min_prec: u8) -> Result<Param> {
        if !self.accept("parameter") {
            self.accept("localparam");
        }
        let kind = if self.at("type") {
            self.ident()?
        } else if self.at("[") || self.at("signed") {
            self.accept("signed");
            self.skip_param_range()?;
            "int".to_string()
        } else if self.tok().kind == Kind::Ident && matches!(self.peek(1).text.as_str(), "=" | "," | ")" | ">") {
            "int".to_string() // parameter W = 8
        } else {
            let kind = self.ident()?;
            if !PARAM_KINDS.contains(&kind.as_str()) {
                return Err(self.fail(format!("unknown parameter type '{kind}'")));
            }
            if !self.accept("signed") {
                self.accept("unsigned");
            }
            self.skip_param_range()?;
            kind
        };
        let name = self.ident()?;
        let bound = if matches!(kind.as_str(), "module" | "config") && self.accept(":") { Some(self.ident()?) } else { None };
        let default = if self.accept("=") {
            Some(if kind == "type" { Arg::Type(self.parse_dtype(false)?) } else { Arg::Expr(self.parse_expr(min_prec)?) })
        } else {
            None
        };
        Ok(Param { kind, name, default, bound })
    }

    /// Ports up to and including `)`. A direction and type carry over to the following names.
    /// Supports `if (C) input x`, groups `if (C) { ... }`, interface ports `iface.modport bus`, and input defaults.
    pub(super) fn parse_ports(&mut self) -> Result<Vec<Port>> {
        let mut ports = Vec::new();
        let (mut dir, mut dtype) = (Dir::Input, DType::default());
        let mut group: Option<Expr> = None;
        while !self.at(")") {
            let mut cond = group.clone();
            if self.accept("if") {
                self.expect("(")?;
                let c = self.parse_expr(0)?;
                self.expect(")")?;
                if self.accept("{") {
                    group = Some(c.clone());
                }
                cond = Some(c);
            }
            if self.tok().kind == Kind::Ident && self.peek(1).text == "." && self.peek(2).kind == Kind::Ident && self.peek(3).kind == Kind::Ident {
                // iface.modport bus: an SV interface port, typed `iface__modport`
                let pos = (self.tok().line, self.tok().col);
                let iface = self.ident()?;
                self.expect(".")?;
                let modport = self.ident()?;
                let name = self.ident()?;
                let dims = self.parse_dims()?;
                let dtype = DType { name: format!("{iface}__{modport}"), ..DType::default() };
                ports.push(Port { dir: Dir::Master, dtype, name, dims, cond, is_async: false, default: None, pos });
            } else {
                let mut is_async = false;
                if let Some(d) = Dir::from_keyword(&self.tok().text).filter(|_| self.tok().kind == Kind::Ident) {
                    self.i += 1;
                    dir = d;
                    is_async = self.accept("async");
                    dtype = self.parse_dtype(true)?;
                }
                let pos = (self.tok().line, self.tok().col);
                let name = self.ident()?;
                let dims = self.parse_dims()?;
                let default = if dir == Dir::Input && self.accept("=") { Some(self.parse_expr(0)?) } else { None };
                ports.push(Port { dir, dtype: dtype.clone(), name, dims, cond, is_async, default, pos });
            }
            if group.is_some() && self.accept("}") {
                group = None;
            }
            if !self.accept(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(ports)
    }
}
