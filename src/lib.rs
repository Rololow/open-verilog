//! Compiler for Open Verilog (.ov): parse, elaborate, emit SystemVerilog.
//!
//! A project is a set of source units: they are parsed separately, merged into one design, elaborated together
//! (modules refer to each other across files), and emitted per origin file.

pub mod ast;
pub mod checks;
pub mod cover;
pub mod domains;
pub mod elab;
pub mod emit;
pub mod error;
pub mod hier;
pub mod library;
pub mod lexer;
pub mod parser;
pub mod pipeline;
pub mod preproc;
pub mod lint;
pub mod probes;
pub mod outputs;
pub mod regmap;
pub mod testgen;
pub mod toml;

use std::collections::HashSet;
use ast::File;
use error::{Error, Result};

/// The output of a project: the SystemVerilog of each origin file, the whole design, and the side outputs.
#[derive(Debug)]
pub struct Compiled {
    /// The whole design, in emission order.
    pub sv: String,
    /// The SystemVerilog of each source file (and of the standard library, when it is used).
    pub by_origin: Vec<(String, String)>,
    pub pipelines: Vec<String>,
    pub warnings: Vec<String>,
    /// The testbench of the `test` blocks (none without tests).
    pub tests_sv: Option<String>,
    /// The register maps (their C header and documentation are written from these).
    pub regmaps: Vec<crate::regmap::RegMap>,
    /// The memory maps: name, description, devices (their C defines, device tree and documentation come from these).
    pub memmaps: Vec<(String, String, Vec<crate::elab::env::MapDev>)>,
    /// The exported structs: their memory layouts.
    pub records: Vec<crate::outputs::Record>,
    /// The instruction sets.
    pub isas: Vec<crate::elab::isa::IsaInfo>,
    /// The probe modules and their bindings (none without probes).
    pub probes_sv: Option<String>,
    /// The packages of the packed structs, in declaration order. They are compiled before the modules that use them:
    /// the whole design starts with them, and `build` writes them to their own file, listed first.
    pub packages: String,
}

/// Compile options.
#[derive(Debug, Clone)]
pub struct Options {
    /// Emit the simulation-only checks of the pipelines.
    pub asserts: bool,
    /// The top module with its arguments (`Name<args>`): the only root emitted, with those parameters.
    pub top: Option<String>,
    /// Coverage items of the case statements (`--cover`).
    pub cover: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { asserts: true, top: None, cover: false }
    }
}

/// Compiles one source file.
pub fn compile_full(src: &str, file: &str) -> Result<Compiled> {
    compile_with(src, file, &Options::default())
}

/// Compiles one source file with options.
pub fn compile_with(src: &str, file: &str, options: &Options) -> Result<Compiled> {
    compile_units(&[(file.to_string(), src.to_string())], options)
}

/// Compiles a project: `(path, source)` units, parsed here in order.
pub fn compile_units(units: &[(String, String)], options: &Options) -> Result<Compiled> {
    let parsed = units.iter().map(|(path, src)| parser::parse_file(src, path)).collect::<Result<Vec<File>>>()?;
    compile_parsed(parsed, options)
}

/// Compiles parsed units (the driver parses them in parallel, one thread per file).
/// The checks of `ovc lint` on parsed units: the warnings of the elaboration and of the clock-domain check, and every
/// error of the domain check (the design is not emitted). An error of the elaboration itself stops the lint.
/// Every import names a package that the sources define.
fn check_imports(source: &File) -> Result<()> {
    for (name, (line, col), file) in &source.imports {
        if !source.packages.contains(name) {
            return Err(Error::new(format!("package '{name}' not found"), file, *line, *col));
        }
    }
    Ok(())
}

/// The result of a lint: the warnings and the errors of the checks, and the top modules that could not be elaborated.
pub struct LintOutput {
    pub warnings: Vec<String>,
    pub errors: Vec<Error>,
    pub skipped: Vec<Error>,
}

/// The checks of `ovc lint` on parsed units. The design is elaborated as a whole; when that fails, each top module
/// (a module that nothing instantiates) is elaborated on its own, so that an error stops only the top modules it is in.
pub fn lint_parsed(parsed: Vec<File>, top: Option<&ast::DType>) -> Result<LintOutput> {
    let mut source = File::default();
    for unit in parsed {
        merge(&mut source, unit)?;
    }
    library::merge(&mut source)?;
    check_imports(&source)?;
    let wrapped = checks::wrapped_in(&source);
    if let Some(root) = top {
        return Ok(match lint_top(&source, Some(root), &wrapped) {
            Ok((warnings, errors)) => LintOutput { warnings, errors, skipped: Vec::new() },
            Err(e) => LintOutput { warnings: Vec::new(), errors: Vec::new(), skipped: vec![e] },
        });
    }
    match lint_top(&source, None, &wrapped) {
        Ok((warnings, errors)) => Ok(LintOutput { warnings, errors, skipped: Vec::new() }),
        Err(_) => {
            let mut out = LintOutput { warnings: Vec::new(), errors: Vec::new(), skipped: Vec::new() };
            for root in top_modules(&source) {
                let dtype = ast::DType { name: root, ..ast::DType::default() };
                match lint_top(&source, Some(&dtype), &wrapped) {
                    Ok((warnings, errors)) => {
                        out.warnings.extend(warnings);
                        out.errors.extend(errors);
                    }
                    Err(e) => out.skipped.push(e),
                }
            }
            Ok(out)
        }
    }
}

/// The checks of one elaboration (the whole design, or one top module).
fn lint_top(source: &File, top: Option<&ast::DType>, wrapped: &checks::Wrapped) -> Result<(Vec<String>, Vec<Error>)> {
    let elab::Elaborated { mut modules, warnings, handshakes, envs, .. } = elab::elaborate(source, "<lint>", false, false, top)?;
    hier::thread(&mut modules, "<lint>")?;
    let (domain_warnings, errors) = domains::check_all(&modules, &handshakes);
    let mut all = warnings;
    all.extend(domain_warnings);
    all.extend(checks::check(&modules, &envs, wrapped));
    Ok((all, errors))
}

/// The modules that nothing instantiates: the tops of the design (all modules when every one is instantiated).
fn top_modules(source: &File) -> Vec<String> {
    fn instantiated(items: &[ast::Item], out: &mut HashSet<String>) {
        for it in items {
            match &it.kind {
                ast::ItemKind::Instance(i) => {
                    out.insert(i.module.clone());
                }
                ast::ItemKind::GenFor { body, .. } | ast::ItemKind::Stage { items: body, .. } => instantiated(body, out),
                ast::ItemKind::GenIf { then, els, .. } => {
                    instantiated(then, out);
                    if let Some(e) = els {
                        instantiated(e, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut used = HashSet::new();
    for m in &source.modules {
        instantiated(&m.items, &mut used);
    }
    // the library modules (origin in angle brackets) are not designs of their own
    let mine = |m: &&ast::Module| !m.origin.starts_with('<') && !m.origin.is_empty();
    let tops: Vec<String> = source.modules.iter().filter(mine).filter(|m| !used.contains(&m.name)).map(|m| m.name.clone()).collect();
    if tops.is_empty() {
        source.modules.iter().filter(mine).map(|m| m.name.clone()).collect()
    } else {
        tops
    }
}

pub fn compile_parsed(parsed: Vec<File>, options: &Options) -> Result<Compiled> {
    let mut source = File::default();
    for unit in parsed {
        merge(&mut source, unit)?;
    }
    library::merge(&mut source)?;
    check_imports(&source)?;
    let top = options.top.as_deref().map(|t| parser::parse_top(t, "<--top>")).transpose()?;
    let elab::Elaborated { mut modules, pipelines, mut warnings, tests, probes, envs, procs, memmaps, records, isas, packages, handshakes } =
        elab::elaborate(&source, "<project>", options.asserts, options.cover, top.as_ref())?;
    hier::thread(&mut modules, "<project>")?;
    // the clock domains are checked on the design as elaborated and connected: children before parents
    warnings.extend(domains::check(&modules, &handshakes)?);
    let tests_sv = if tests.is_empty() { None } else { Some(testgen::generate(&tests, &modules, &envs, &procs, "<project>")?) };
    let probes_sv = if probes.is_empty() { None } else { Some(probes::generate(&probes, &modules, "<project>")?) };
    let packages: String = packages.iter().map(|(_, text)| text.as_str()).collect();
    let sv = format!("{packages}{}", emit::emit_modules(&modules, "<project>")?);
    let by_origin = split_by_origin(&modules)?;
    let regmaps = source.regmaps.clone();
    Ok(Compiled { sv, by_origin, pipelines, warnings, tests_sv, probes_sv, regmaps, memmaps, records, isas, packages })
}

/// Compiles one source file to SystemVerilog text.
pub fn compile(src: &str, file: &str) -> Result<String> {
    Ok(compile_full(src, file)?.sv)
}

/// The SystemVerilog of each origin, in the order the origins first appear.
fn split_by_origin(modules: &[ast::Module]) -> Result<Vec<(String, String)>> {
    let mut origins: Vec<&str> = Vec::new();
    for m in modules {
        let origin = if m.origin.is_empty() { "<stdlib>" } else { m.origin.as_str() };
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    let mut out = Vec::new();
    for origin in origins {
        let group: Vec<ast::Module> = modules
            .iter()
            .filter(|m| (if m.origin.is_empty() { "<stdlib>" } else { m.origin.as_str() }) == origin)
            .cloned()
            .collect();
        out.push((origin.to_string(), emit::emit_modules(&group, origin)?));
    }
    Ok(out)
}

/// Adds the definitions of one unit to the design. A module defined twice is an error.
fn merge(design: &mut File, unit: File) -> Result<()> {
    for m in &unit.modules {
        if let Some(prev) = design.modules.iter().find(|p| p.name == m.name) {
            return Err(Error::new(
                format!("module '{}' is defined twice (first in {})", m.name, prev.origin),
                &m.origin,
                m.pos.0,
                m.pos.1,
            ));
        }
    }
    design.modules.extend(unit.modules);
    design.enums.extend(unit.enums);
    design.packages.extend(unit.packages);
    design.imports.extend(unit.imports);
    design.consts.extend(unit.consts);
    design.values.extend(unit.values);
    design.fns.extend(unit.fns);
    design.structs.extend(unit.structs);
    design.typedefs.extend(unit.typedefs);
    design.bundles.extend(unit.bundles);
    design.configs.extend(unit.configs);
    design.traits.extend(unit.traits);
    design.isas.extend(unit.isas);
    design.tests.extend(unit.tests);
    design.regmaps.extend(unit.regmaps);
    design.test_loops.extend(unit.test_loops);
    design.probes.extend(unit.probes);
    Ok(())
}
