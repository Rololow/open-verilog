//! Recursive-descent parser for .ov. Expressions use precedence climbing.

mod expr;
mod fnbody;
mod item;
mod defs;
mod module;
mod regmap;
mod stmt;
mod types;

use crate::ast::File;
use crate::error::{Error, Result};
use crate::lexer::{lex, Kind, Token};

const BASE_TYPES: &[&str] = &["logic", "wire", "reg", "bit", "int", "integer", "byte", "shortint", "longint"];
const PARAM_KINDS: &[&str] = &["int", "bool", "type", "module", "config", "list", "logic", "integer", "bit", "byte", "shortint", "longint", "reg"];
const UNOPS: &[&str] = &["!", "~", "-", "+", "&", "|", "^"];
const ASSIGN_OPS: &[(&str, &str)] = &[("+=", "+"), ("-=", "-"), ("*=", "*"), ("/=", "/"), ("|=", "|"), ("&=", "&"), ("^=", "^")];
/// Keywords of constructs this build does not parse yet: reported by name instead of as a generic syntax error.
const UNSUPPORTED: &[&str] = &[
    "struct", "typedef", "import", "function", "override",
    "same_domain", "package", "interface", "trait", "bundle", "config", "export", "isa",
    "probe", "test", "break", "continue",
];
/// Relational precedence; generic arguments `<...>` are parsed above it.
const REL_PREC: u8 = 8;
const GENERIC_PREC: u8 = REL_PREC + 1;

/// Binding power of a binary operator (higher binds tighter), if `text` is one.
pub fn binop_prec(text: &str) -> Option<u8> {
    Some(match text {
        ".." => 1,
        "||" => 2,
        "&&" => 3,
        "|" => 4,
        "^" => 5,
        "&" => 6,
        "==" | "!=" | "===" | "!==" => 7,
        "<" | "<=" | ">" | ">=" => REL_PREC,
        "<<" | ">>" | "<<<" | ">>>" => 9,
        "+" | "-" | "++" => 10,
        "*" | "/" | "%" => 11,
        "**" => 12,
        _ => return None,
    })
}

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    file: String,
    /// Inside `<...>`: `>>` closes two generics instead of shifting.
    in_generic: bool,
    /// Inside `always_ff`: `x = e` is a non-blocking assignment.
    in_ff: bool,
    /// Inside an isa field width: a  after it is the count of the field, not an index.
    no_index: bool,
    /// Enums declared inside a module: they are global, so they move to the file when it is parsed.
    hoisted_enums: Vec<crate::ast::EnumDef>,
    /// `for (a, b) in list`: the bindings of the names, given to the body of the loop.
    unpack: Vec<(String, crate::ast::Expr)>,
    /// The imports of the file: the package of each, and where it is written.
    imports: Vec<(String, (usize, usize))>,
}

/// Parses a .ov file into its modules.
pub fn parse_file(src: &str, file: &str) -> Result<File> {
    let toks = lex(src, file)?;
    let mut p = Parser { toks, i: 0, file: file.to_string(), in_generic: false, in_ff: false, no_index: false, hoisted_enums: Vec::new(), unpack: Vec::new(), imports: Vec::new() };
    let mut out = File::default();
    p.top_items(&mut out, None)?;
    out.enums.append(&mut p.hoisted_enums);
    out.imports = p.imports.into_iter().map(|(name, pos)| (name, pos, file.to_string())).collect();
    Ok(out)
}

/// A type on the command line, such as the top module with its arguments: `DarkRiscv<DarkCfg with { csr = true }>`.
pub fn parse_top(src: &str, file: &str) -> Result<crate::ast::DType> {
    let toks = lex(src, file)?;
    let mut p = Parser { toks, i: 0, file: file.to_string(), in_generic: false, in_ff: false, no_index: false, hoisted_enums: Vec::new(), unpack: Vec::new(), imports: Vec::new() };
    let dt = p.parse_dtype(false)?;
    if p.tok().kind != Kind::Eof {
        return Err(p.fail(format!("unexpected '{}' after the top module", p.tok().text)));
    }
    Ok(dt)
}

impl Parser {
    fn tok(&self) -> &Token {
        &self.toks[self.i]
    }

    fn peek(&self, n: usize) -> &Token {
        &self.toks[(self.i + n).min(self.toks.len() - 1)]
    }

    fn at(&self, text: &str) -> bool {
        self.tok().text == text
    }

    fn accept(&mut self, text: &str) -> bool {
        let hit = self.at(text);
        if hit {
            self.i += 1;
        }
        hit
    }

    fn fail(&self, message: impl Into<String>) -> Error {
        let t = self.tok();
        Error::new(message, &self.file, t.line, t.col)
    }

    fn found(&self) -> &str {
        if self.tok().kind == Kind::Eof { "end of file" } else { &self.tok().text }
    }

    pub(super) fn expect(&mut self, text: &str) -> Result<()> {
        if self.accept(text) {
            return Ok(());
        }
        if matches!(text, ";" | ")" | "]" | ",") && self.i > 0 {
            // the symbol is missing after the previous token: point just past it
            let prev = &self.toks[self.i - 1];
            let col = prev.col + prev.text.chars().count();
            return Err(Error::new(format!("expected '{text}' after '{}', found '{}'", prev.text, self.found()), &self.file, prev.line, col));
        }
        Err(self.fail(format!("expected '{text}', found '{}'", self.found())))
    }

    pub(super) fn ident(&mut self) -> Result<String> {
        if self.tok().kind != Kind::Ident {
            return Err(self.fail(format!("identifier expected, found '{}'", self.found())));
        }
        self.i += 1;
        Ok(self.toks[self.i - 1].text.clone())
    }

    /// `item (, item)* close`; a `>>` that closes generics is split first.
    pub(super) fn comma_list<T>(&mut self, close: &str, mut item: impl FnMut(&mut Parser) -> Result<T>) -> Result<Vec<T>> {
        let mut out = Vec::new();
        while !self.at(close) {
            out.push(item(self)?);
            if close == ">" {
                self.split_angle();
            }
            if !self.accept(",") {
                break;
            }
        }
        self.expect(close)?;
        Ok(out)
    }

    /// The lexer reads `>>` in `A<B<N>>`: keep one `>` for the inner generic.
    fn split_angle(&mut self) {
        let t = self.tok().clone();
        if t.kind == Kind::Op && matches!(t.text.as_str(), ">>" | ">>>" | ">=" | ">>=") {
            let rest = Token { kind: Kind::Op, text: t.text[1..].to_string(), line: t.line, col: t.col + 1 };
            let close = Token { kind: Kind::Op, text: ">".to_string(), line: t.line, col: t.col };
            self.toks.splice(self.i..self.i + 1, [close, rest]);
        }
    }

    /// `{` or `begin`: returns the matching closer.
    pub(super) fn block_open(&mut self) -> Result<&'static str> {
        if self.accept("{") {
            return Ok("}");
        }
        if self.accept("begin") {
            if self.accept(":") {
                self.ident()?; // the label is not kept
            }
            return Ok("end");
        }
        Err(self.fail(format!("'{{' or 'begin' expected, found '{}'", self.found())))
    }

    pub(super) fn block_close(&mut self, closer: &str) -> Result<()> {
        self.expect(closer)?;
        if matches!(closer, "end" | "endmodule" | "endgenerate" | "endcase") && self.accept(":") {
            self.ident()?;
        }
        Ok(())
    }

    pub(super) fn at_block(&self) -> bool {
        self.at("{") || self.at("begin")
    }
}
