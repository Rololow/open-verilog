//! `regmap Name "desc" { REG @ addr [strobe] "desc" { field : access [msb:lsb] [= reset] "desc"; ... } ... }`.
//! The register block is generated as a module (see `regmap::module_source`) and parsed with the design.

use super::Parser;
use crate::ast::Pos;
use crate::error::Result;
use crate::lexer::{number_value, Kind};
use crate::regmap::{check, Field, Reg, RegMap};

impl Parser {
    /// A number: a decimal or based literal.
    fn reg_number(&mut self) -> Result<i64> {
        if self.tok().kind != Kind::Num {
            return Err(self.fail(format!("number expected, found '{}'", self.found())));
        }
        let text = self.tok().text.clone();
        let Some(v) = number_value(&text) else {
            return Err(self.fail(format!("'{text}' is not a number")));
        };
        self.i += 1;
        Ok(v)
    }

    /// An optional string (a description); empty when there is none.
    pub(super) fn opt_string(&mut self) -> String {
        if self.tok().kind == Kind::Str {
            let text = self.tok().text.trim_matches('"').to_string();
            self.i += 1;
            return text;
        }
        String::new()
    }

    pub(super) fn parse_regmap(&mut self) -> Result<RegMap> {
        let pos: Pos = (self.tok().line, self.tok().col);
        self.expect("regmap")?;
        let name = self.ident()?;
        let desc = self.opt_string();
        self.expect("{")?;
        let mut regs = Vec::new();
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            let rpos: Pos = (self.tok().line, self.tok().col);
            let rname = self.ident()?;
            self.expect("@")?;
            let addr = self.reg_number()?;
            let strobe = self.accept("strobe");
            let rdesc = self.opt_string();
            self.expect("{")?;
            let mut fields = Vec::new();
            while !self.accept("}") {
                if self.tok().kind == Kind::Eof {
                    return Err(self.fail("'}' missing"));
                }
                let fpos: Pos = (self.tok().line, self.tok().col);
                let fname = self.ident()?;
                self.expect(":")?;
                let access = self.ident()?;
                self.expect("[")?;
                let msb = self.reg_number()?;
                let lsb = if self.accept(":") { self.reg_number()? } else { msb };
                self.expect("]")?;
                let reset = if self.accept("=") { self.reg_number()? } else { 0 };
                let desc = self.opt_string();
                self.expect(";")?;
                fields.push(Field { name: fname, access, msb, lsb, reset, desc, pos: fpos });
            }
            regs.push(Reg { name: rname, addr, desc: rdesc, pos: rpos, fields, strobe });
        }
        let rm = RegMap { name, desc, pos, regs };
        check(&rm, &self.file)?;
        Ok(rm)
    }
}
