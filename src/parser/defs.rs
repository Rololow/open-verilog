//! Top-level definitions: modules, enums, constants, functions, structs, typedefs and packages (whose
//! contents are flattened into the file).

use super::Parser;
use crate::ast::{BundleDef, BundleField, ConfigDef, Dir, DType, EnumDef, File, IsaDef, IsaField, IsaFormat, ItemKind, ProbeDef, StructDef, TestDef, TraitDef};
use crate::error::Result;
use crate::lexer::Kind;

impl Parser {
    /// A top-level function has one definition in the file: a second one is an error at it, with the first one's line.
    fn define_fns(&mut self, items: &[ItemKind], at: (usize, usize)) -> Result<()> {
        for item in items {
            let ItemKind::Fn { name, .. } = item else { continue };
            if let Some((_, first)) = self.fn_defs.iter().find(|(n, _)| n == name) {
                let msg = format!("'{name}' is already defined at {}:{}", self.file, first.0);
                return Err(crate::error::Error::new(msg, &self.file, at.0, at.1));
            }
            self.fn_defs.push((name.clone(), at));
        }
        Ok(())
    }

    /// Top-level items up to `closer` (`endpackage`), or to the end of the file when there is none.
    pub(super) fn top_items(&mut self, out: &mut File, closer: Option<&str>) -> Result<()> {
        loop {
            if self.tok().kind == Kind::Eof {
                return match closer {
                    Some(c) => Err(self.fail(format!("'{c}' missing"))),
                    None => Ok(()),
                };
            }
            if let Some(c) = closer {
                if self.accept(c) {
                    return Ok(());
                }
            }
            if self.at("module") || self.at("pipeline") {
                out.modules.push(self.parse_module()?);
            } else if self.at("enum") {
                out.enums.push(self.parse_enum_def()?);
            } else if self.at("const") && self.peek(1).kind == Kind::Ident && self.peek(2).text == "=" {
                // `const NAME = value;`: a constant without a type, like `val` (a list is a constant table)
                self.i += 1;
                loop {
                    let name = self.ident()?;
                    self.expect("=")?;
                    let expr = self.parse_expr(0)?;
                    out.values.push((name, expr));
                    if !self.accept(",") {
                        break;
                    }
                }
                self.expect(";")?;
            } else if self.at("const") || self.at("localparam") || self.at("parameter") {
                out.consts.push(self.parse_decl()?);
            } else if self.at("regmap") {
                // the register block is a module of the design, generated from the description
                let rm = self.parse_regmap()?;
                let unit = super::parse_file(&crate::regmap::module_source(&rm), &self.file)?;
                out.modules.extend(unit.modules);
                out.regmaps.push(rm);
            } else if self.at("val") {
                self.i += 1;
                let name = self.ident()?;
                self.expect("=")?;
                let expr = self.parse_expr(0)?;
                self.expect(";")?;
                out.values.push((name, expr));
            } else if self.at("fn") {
                let at = (self.tok().line, self.tok().col);
                let items = self.parse_item_kind()?;
                self.define_fns(&items, at)?;
                out.fns.extend(items);
            } else if self.at("function") {
                let at = (self.tok().line, self.tok().col);
                let item = self.parse_function_def()?;
                self.define_fns(std::slice::from_ref(&item), at)?;
                out.fns.push(item);
            } else if self.at("struct") {
                out.structs.push(self.parse_struct_def()?);
            } else if self.at("export") {
                // a memory layout shared with software: laid out as memory is
                self.i += 1;
                let mut def = self.parse_struct_def()?;
                def.export = true;
                out.structs.push(def);
            } else if self.at("bundle") {
                out.bundles.push(self.parse_bundle_def()?);
            } else if self.at("config") {
                out.configs.push(self.parse_config_def()?);
            } else if self.at("trait") {
                out.traits.push(self.parse_trait_def()?);
            } else if self.at("isa") {
                out.isas.push(self.parse_isa_def()?);
            } else if self.at("test") {
                out.tests.push(self.parse_test_def()?);
            } else if self.at("for") {
                out.test_loops.push(self.parse_test_loop()?);
            } else if self.at("probe") {
                out.probes.push(self.parse_probe_def()?);
            } else if self.at("typedef") {
                self.parse_typedef(out)?;
            } else if self.at("package") {
                self.i += 1;
                let name = self.ident()?;
                out.packages.push(name);
                self.expect(";")?;
                self.top_items(out, Some("endpackage"))?;
                self.accept(";");
            } else if self.at("import") {
                self.skip_import()?;
            } else {
                let t = self.tok().clone();
                return Err(crate::error::Error::new(format!("module expected, found '{}'", t.text), &self.file, t.line, t.col));
            }
        }
    }

    /// `import Pkg::*;` or `import Pkg::name;`: packages are flattened, so the import has nothing to do.
    pub(super) fn skip_import(&mut self) -> Result<()> {
        self.expect("import")?;
        // the package is checked once the sources are read: it must be defined by one of them
        if self.tok().kind == Kind::Ident {
            self.imports.push((self.tok().text.clone(), (self.tok().line, self.tok().col)));
        }
        while !self.accept(";") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("';' missing after import"));
            }
            self.i += 1;
        }
        Ok(())
    }

    /// `test "name" for Module[<args>] [timeout N] { statements }`. The statements are checked when the testbench
    /// is written (--emit-tests); here they are only parsed.
    pub(super) fn parse_test_def(&mut self) -> Result<TestDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("test")?;
        // the name is a string, or an expression made of a loop variable (`t.name`)
        let (name, name_expr) = if self.tok().kind == Kind::Str {
            let name = self.tok().text.trim_matches('"').to_string();
            self.i += 1;
            (name, None)
        } else {
            (String::new(), Some(self.parse_expr(0)?))
        };
        self.expect("for")?;
        let module = self.ident()?;
        let targs = if self.accept("<") { self.comma_list(">", |p| p.parse_targ(super::GENERIC_PREC))? } else { Vec::new() };
        let timeout = if self.tok().text == "timeout" {
            self.i += 1;
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        let body = self.parse_block_statements()?;
        Ok(TestDef { name, name_expr, module, targs, timeout, body, pos, origin: self.file.clone() })
    }

    /// `for t in list { test ... }`: tests for each row of a list, loops nested.
    pub(super) fn parse_test_loop(&mut self) -> Result<crate::ast::TestLoop> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("for")?;
        // `for (a, b) in rows`: each row's elements are named
        let (var, names) = if self.at("(") && self.peek(1).kind == Kind::Ident && self.peek(2).text == "," {
            (format!("ov_row_{}_{}", pos.0, pos.1), self.tuple_names()?)
        } else {
            (self.ident()?, Vec::new())
        };
        self.expect("in")?;
        let iter = self.parse_expr(0)?;
        self.expect("{")?;
        let (mut tests, mut loops) = (Vec::new(), Vec::new());
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            if self.at("test") {
                tests.push(self.parse_test_def()?);
            } else if self.at("for") {
                loops.push(self.parse_test_loop()?);
            } else {
                return Err(self.fail(format!("a test or a loop is expected here, found '{}'", self.found())));
            }
        }
        Ok(crate::ast::TestLoop { var, names, iter, tests, loops, pos })
    }

    /// `probe name on Module when (cond) [limit N] { statements }`
    pub(super) fn parse_probe_def(&mut self) -> Result<ProbeDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("probe")?;
        let name = self.ident()?;
        self.expect("on")?;
        let module = self.ident()?;
        self.expect("when")?;
        self.expect("(")?;
        let cond = self.parse_expr(0)?;
        self.expect(")")?;
        let limit = if self.tok().text == "limit" {
            self.i += 1;
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        let body = self.parse_block_statements()?;
        Ok(ProbeDef { name, module, cond, limit, body, pos, origin: self.file.clone() })
    }

    pub(super) fn parse_block_statements(&mut self) -> Result<Vec<crate::ast::Stmt>> {
        self.expect("{")?;
        let mut stmts = Vec::new();
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    /// `isa Name ["desc"] : opcode N [max M] { FORMAT = code ["desc"] (like Other; | { field : W [N]; ... }) ... }`
    pub(super) fn parse_isa_def(&mut self) -> Result<IsaDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("isa")?;
        let name = self.ident()?;
        let desc = self.description();
        self.expect(":")?;
        self.expect("opcode")?;
        let opcode_bits = self.parse_expr(0)?;
        let max = if self.tok().kind == Kind::Ident && self.tok().text == "max" {
            self.i += 1;
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        self.expect("{")?;
        let mut formats = Vec::new();
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            let fpos = (self.tok().line, self.tok().col);
            let fname = self.ident()?;
            self.expect("=")?;
            let code = self.parse_expr(0)?;
            let fdesc = self.description();
            let (like, fields) = if self.accept("like") {
                let base = self.ident()?;
                self.expect(";")?;
                (Some(base), Vec::new())
            } else {
                self.expect("{")?;
                let mut fields = Vec::new();
                while !self.accept("}") {
                    if self.tok().kind == Kind::Eof {
                        return Err(self.fail("'}' missing"));
                    }
                    let fieldpos = (self.tok().line, self.tok().col);
                    let field = self.ident()?;
                    self.expect(":")?;
                    self.no_index = true;
                    let width = self.parse_expr(0);
                    self.no_index = false;
                    let width = width?;
                    let count = if self.accept("[") {
                        let n = self.parse_expr(0)?;
                        self.expect("]")?;
                        Some(n)
                    } else {
                        None
                    };
                    let desc = self.description();
                    self.expect(";")?;
                    fields.push(IsaField { name: field, desc, width, count, pos: fieldpos });
                }
                (None, fields)
            };
            formats.push(IsaFormat { name: fname, code, like, fields, pos: fpos, desc: fdesc });
        }
        Ok(IsaDef { name, desc, opcode_bits, max, formats, pos })
    }

    /// An optional string after a name: the description (for the documentation and the SV package).
    fn description(&mut self) -> String {
        if self.tok().kind == Kind::Str {
            let text = self.tok().text.trim_matches('"').to_string();
            self.i += 1;
            return text;
        }
        String::new()
    }

    /// `trait Name { fn name(args) [-> type]; ... }`: the names of the functions.
    pub(super) fn parse_trait_def(&mut self) -> Result<TraitDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("trait")?;
        let name = self.ident()?;
        self.expect("{")?;
        let mut fns = Vec::new();
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            self.expect("fn")?;
            fns.push(self.ident()?);
            // the signature: its parameters and return type are checked where a module implements the trait
            while !self.accept(";") {
                if self.tok().kind == Kind::Eof {
                    return Err(self.fail("';' missing"));
                }
                self.i += 1;
            }
        }
        Ok(TraitDef { name, fns, pos })
    }

    /// `config Name [extends Base] { field = expr; ... }`
    pub(super) fn parse_config_def(&mut self) -> Result<ConfigDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("config")?;
        let name = self.ident()?;
        // `from "file.toml"`: the values come from a manifest
        let source = if self.accept("from") {
            if self.tok().kind != Kind::Str {
                return Err(self.fail("a file name in quotes is expected after 'from'"));
            }
            let path = self.tok().text.trim_matches('"').to_string();
            self.i += 1;
            Some(path)
        } else {
            None
        };
        let base = if self.accept("extends") { Some(self.ident()?) } else { None };
        self.expect("{")?;
        let mut fields = Vec::new();
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            let field = self.ident()?;
            self.expect("=")?;
            let value = self.parse_expr(0)?;
            self.expect(";")?;
            fields.push((field, value));
        }
        Ok(ConfigDef { name, base, fields, pos, source, origin: self.file.clone() })
    }

    /// `bundle Name<params> { [input|output] type field [= default]; ... fn ... }`
    pub(super) fn parse_bundle_def(&mut self) -> Result<BundleDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("bundle")?;
        let name = self.ident()?;
        let params = if self.accept("<") { self.comma_list(">", |p| p.parse_param(super::GENERIC_PREC))? } else { Vec::new() };
        self.expect("{")?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut monitors = Vec::new();
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            if self.at("fn") {
                methods.extend(self.parse_item_kind()?);
                continue;
            }
            if self.accept("monitor") {
                monitors.push(self.parse_dtype(false)?);
                self.expect(";")?;
                continue;
            }
            let dir = match self.ident()?.as_str() {
                "input" => Dir::Input,
                "output" => Dir::Output,
                other => return Err(self.fail(format!("a bundle signal is 'input' or 'output', not '{other}'"))),
            };
            let dtype = self.parse_dtype(false)?;
            let name = self.ident()?;
            let default = if self.accept("=") { Some(self.parse_expr(0)?) } else { None };
            self.expect(";")?;
            fields.push(BundleField { dir, dtype, name, default });
        }
        Ok(BundleDef { name, params, fields, methods, monitors, pos })
    }

    /// `struct [packed] Name { type field; ... fn ... }`. In `typedef struct packed { ... } Name;` the body has
    /// no name (it is empty here) and the caller reads the name after it.
    pub(super) fn parse_struct_def(&mut self) -> Result<StructDef> {
        let pos = (self.tok().line, self.tok().col);
        self.expect("struct")?;
        self.accept("packed");
        let name = if self.at("{") { String::new() } else { self.ident()? };
        let params = if self.accept("<") { self.comma_list(">", |p| p.parse_param(super::GENERIC_PREC))? } else { Vec::new() };
        let desc = self.opt_string();
        self.expect("{")?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut field_desc = std::collections::BTreeMap::new();
        while !self.accept("}") {
            if self.tok().kind == Kind::Eof {
                return Err(self.fail("'}' missing"));
            }
            if self.at("fn") {
                methods.extend(self.parse_item_kind()?);
            } else {
                let dtype = self.parse_dtype(false)?;
                let field = self.ident()?;
                let text = self.opt_string();
                if !text.is_empty() {
                    field_desc.insert(field.clone(), text);
                }
                self.expect(";")?;
                fields.push((dtype, field));
            }
        }
        Ok(StructDef { name, params, fields, methods, pos, export: false, desc, field_desc, origin: self.file.clone() })
    }

    /// `typedef enum ... Name;`, `typedef struct ... Name;` or `typedef type Name;`
    fn parse_typedef(&mut self, out: &mut File) -> Result<()> {
        self.expect("typedef")?;
        if self.at("enum") {
            self.i += 1;
            let dtype = if self.at("{") { None } else { Some(self.parse_dtype(false)?) };
            self.expect("{")?;
            let members = self.comma_list("}", |p| {
                let member = p.ident()?;
                let value = if p.accept("=") { Some(p.parse_expr(0)?) } else { None };
                Ok((member, value))
            })?;
            let name = self.ident()?;
            self.expect(";")?;
            let pos = (self.tok().line, self.tok().col);
            out.enums.push(EnumDef { name, dtype, members, pos });
        } else if self.at("struct") {
            let mut def = self.parse_struct_def()?;
            def.name = self.ident()?;
            self.expect(";")?;
            out.structs.push(def);
        } else {
            let dtype = self.parse_dtype(false)?;
            let name = self.ident()?;
            self.expect(";")?;
            out.typedefs.push((name, dtype));
        }
        Ok(())
    }

    /// `function [automatic] type name(input type arg, ...); return expr; endfunction`
    pub(super) fn parse_function_def(&mut self) -> Result<ItemKind> {
        self.expect("function")?;
        if !self.accept("automatic") {
            self.accept("static");
        }
        let ret = self.parse_dtype(false)?;
        let name = self.ident()?;
        self.expect("(")?;
        let typed = self.comma_list(")", |p| {
            p.accept("input");
            let ty = p.parse_dtype(true)?;
            Ok((ty, p.ident()?))
        })?;
        let (ptypes, params): (Vec<DType>, Vec<String>) = typed.into_iter().unzip();
        self.expect(";")?;
        let body = self.parse_fn_body("endfunction")?;
        let ret = if ret.packed.is_empty() { None } else { Some(ret) };
        Ok(ItemKind::Fn { name, params, ptypes, ret, body, override_at: None })
    }
}
