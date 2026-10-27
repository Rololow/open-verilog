//! Data types, packed and unpacked dimensions, generic arguments.

use super::{Parser, BASE_TYPES, GENERIC_PREC};
use crate::ast::{Arg, DType, EnumDef, Expr};
use crate::error::Result;
use crate::lexer::Kind;

impl Parser {
    /// `[N]` or `[hi:lo]` after a name. `[hi:lo]` is stored as its size, `abs(hi - lo) + 1`.
    pub(super) fn parse_dims(&mut self) -> Result<Vec<Expr>> {
        let mut dims = Vec::new();
        while self.accept("[") {
            let (line, col) = (self.tok().line, self.tok().col);
            let mut e = self.parse_expr(0)?;
            if self.accept(":") {
                let lo = self.parse_expr(0)?;
                let diff = Expr::Binary("-".into(), Box::new(e), Box::new(lo), (line, col));
                e = Expr::Binary("+".into(), Box::new(Expr::Call(Box::new(Expr::Ident("abs".into(), (line, col))), vec![diff])), Box::new(Expr::Num("1".into())), (line, col));
            }
            dims.push(e);
            self.expect("]")?;
        }
        Ok(dims)
    }

    /// `[Name<args>] [signed] [msb:lsb]...`. Without a type name the type is `logic`; `optional` allows no type at all.
    pub(super) fn parse_dtype(&mut self, optional: bool) -> Result<DType> {
        let mut dt = DType::default();
        if self.tok().kind == Kind::Ident && self.peek(1).text == "::" {
            self.i += 2; // Pkg::type: packages are flattened
        }
        let t = self.tok().clone();
        let named = t.kind == Kind::Ident
            && t.text != "signed"
            && (BASE_TYPES.contains(&t.text.as_str())
                || self.peek(1).kind == Kind::Ident
                || self.peek(1).kind == Kind::Eof // a bare module name on the command line: `--top Top`
                || self.peek(1).text == "<"
                || (self.peek(1).text == "[" && self.typed_packed_decl()));
        if named {
            dt.name = self.ident()?;
            if self.accept("<") {
                dt.targs = self.comma_list(">", |p| p.parse_targ(GENERIC_PREC))?;
            }
        } else if !optional {
            return Err(self.fail("type expected"));
        }
        if self.accept("signed") {
            dt.signed = true;
        }
        self.accept("unsigned");
        while self.accept("[") {
            let mut msb = self.parse_expr(0)?;
            let lsb = if self.accept(":") {
                self.parse_expr(0)?
            } else {
                // `[N]` before the name is N bits, i.e. [N-1:0]
                let (line, col) = (self.tok().line, self.tok().col);
                msb = Expr::Binary("-".into(), Box::new(msb), Box::new(Expr::Num("1".into())), (line, col));
                Expr::Num("0".into())
            };
            self.expect("]")?;
            dt.packed.push((msb, lsb));
        }
        Ok(dt)
    }

    /// At `name [..]... other`: a declaration whose type is a name with packed dimensions.
    pub(super) fn typed_packed_decl(&self) -> bool {
        let mut n = 1;
        let mut depth: i32 = 0;
        while self.peek(n).kind != Kind::Eof {
            let tx = self.peek(n).text.as_str();
            if depth == 0 && tx != "[" {
                return n > 1 && self.peek(n).kind == Kind::Ident;
            }
            depth += (tx == "[") as i32 - (tx == "]") as i32;
            n += 1;
        }
        false
    }

    /// `enum Name [: type] { A, B = 3, C }`
    pub(super) fn parse_enum_def(&mut self) -> Result<EnumDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("enum")?;
        let name = self.ident()?;
        let dtype = if self.accept(":") { Some(self.parse_dtype(false)?) } else { None };
        self.expect("{")?;
        let members = self.comma_list("}", |p| {
            let member = p.ident()?;
            let value = if p.accept("=") { Some(p.parse_expr(0)?) } else { None };
            Ok((member, value))
        })?;
        self.accept(";");
        Ok(EnumDef { name, dtype, members, pos })
    }

    /// A generic argument: a type (`logic [7:0]`) or an expression.
    pub(super) fn parse_targ(&mut self, min_prec: u8) -> Result<Arg> {
        let t = self.tok();
        if t.kind == Kind::Ident && (BASE_TYPES.contains(&t.text.as_str()) || self.peek(1).text == "<") {
            return Ok(Arg::Type(self.parse_dtype(false)?));
        }
        let saved = std::mem::replace(&mut self.in_generic, true);
        let e = self.parse_expr(min_prec);
        self.in_generic = saved;
        Ok(Arg::Expr(e?))
    }

    /// `#(.W(8), .T(logic [7:0]), 8)`: a named `(name, value)` or a positional value.
    pub(super) fn parse_named_targ(&mut self) -> Result<(Option<String>, Arg)> {
        if self.accept(".") {
            let name = self.ident()?;
            self.expect("(")?;
            let v = self.parse_targ(0)?;
            self.expect(")")?;
            return Ok((Some(name), v));
        }
        Ok((None, self.parse_targ(0)?))
    }
}
