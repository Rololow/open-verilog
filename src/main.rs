//! Command-line driver of the compiler.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;

use ovc::ast::File;
use ovc::error::Error;

const USAGE: &str = "\
usage: ovc FILE.ov... [-o OUT.sv] [--emit-tests TB.sv] [--emit-probes P.sv] [--emit-isa-sv P.sv] [--emit-isa-py P.py]
             [--emit-doc DOC.md] [--dump-pipeline] [--no-asserts] [--werror]
       ovc build PATH... [-o OUT_DIR] [--root DIR]

  FILE.ov...      the files are one design: they are compiled together, the SystemVerilog goes to -o or stdout
  build PATH...   every .ov file under PATH (directories are searched); one .sv per file, in OUT_DIR (default: out),
                  keeping the folder structure, and a file list (files.f)";

#[derive(Default)]
struct Args {
    inputs: Vec<String>,
    output: Option<String>,
    emit_tests: Option<String>,
    emit_c: Option<String>,
    emit_doc: Option<String>,
    emit_dts: Option<String>,
    emit_py: Option<String>,
    emit_isa_sv: Option<String>,
    emit_isa_py: Option<String>,
    emit_probes: Option<String>,
    dump_pipeline: bool,
    werror: bool,
    asserts: bool,
    top: Option<String>,
    build: bool,
    root: Option<String>,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut args = Args { asserts: true, ..Args::default() };
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "build" => args.build = true,
            "-o" => args.output = Some(value("-o")?),
            "--root" => args.root = Some(value("--root")?),
            "--emit-tests" => args.emit_tests = Some(value("--emit-tests")?),
            "--emit-c" => args.emit_c = Some(value("--emit-c")?),
            "--emit-doc" => args.emit_doc = Some(value("--emit-doc")?),
            "--emit-dts" => args.emit_dts = Some(value("--emit-dts")?),
            "--emit-py" => args.emit_py = Some(value("--emit-py")?),
            "--emit-isa-sv" => args.emit_isa_sv = Some(value("--emit-isa-sv")?),
            "--emit-isa-py" => args.emit_isa_py = Some(value("--emit-isa-py")?),
            "--emit-probes" => args.emit_probes = Some(value("--emit-probes")?),
            "--dump-pipeline" => args.dump_pipeline = true,
            "--werror" => args.werror = true,
            "--no-asserts" => args.asserts = false,
            "--top" => args.top = Some(it.next().ok_or("--top needs a module with its arguments")?),
            "-h" | "--help" => return Err(String::new()),
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            _ => args.inputs.push(arg),
        }
    }
    if args.inputs.is_empty() {
        return Err("no input".into());
    }
    Ok(args)
}

/// The `.ov` files under the given paths, in a stable order.
fn collect(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out)?;
            } else if p.extension().is_some_and(|e| e == "ov") {
                out.push(p);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_dir() {
            walk(&path, &mut out).map_err(|e| format!("{p}: {e}"))?;
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1).collect()) {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("ovc: {msg}");
            }
            eprintln!("{USAGE}");
            return if msg.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE };
        }
    };
    // elaboration recurses through deep expressions (a table read by a signal is a chain of conditionals as deep as
    // the table), so the work runs on a thread with a large stack
    let worker = thread::Builder::new().stack_size(512 << 20).spawn(move || run(args));
    match worker.map(|h| h.join()) {
        Ok(Ok(Ok(()))) => ExitCode::SUCCESS,
        Ok(Ok(Err(msg))) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
        _ => {
            eprintln!("ovc: the compiler stopped unexpectedly");
            ExitCode::FAILURE
        }
    }
}

/// Reads and parses the units in parallel, one thread per file.
fn parse_units(paths: &[PathBuf]) -> Result<Vec<(String, String, File)>, String> {
    let results: Vec<Result<(String, String, File), String>> = thread::scope(|s| {
        let jobs: Vec<_> = paths
            .iter()
            .map(|path| {
                s.spawn(move || {
                    let name = path.to_string_lossy().replace('\\', "/");
                    let src = std::fs::read_to_string(path).map_err(|e| format!("ovc: {name}: {e}"))?;
                    match ovc::parser::parse_file(&src, &name) {
                        Ok(file) => Ok((name, src, file)),
                        Err(e) => Err(e.render(&src)),
                    }
                })
            })
            .collect();
        jobs.into_iter().map(|j| j.join().expect("parser thread panicked")).collect()
    });
    results.into_iter().collect()
}

fn run(args: Args) -> Result<(), String> {
    let paths = collect(&args.inputs)?;
    if paths.is_empty() {
        return Err("no .ov file found".into());
    }
    let units = parse_units(&paths)?;
    let options = ovc::Options { asserts: args.asserts, top: args.top.clone() };
    let sources: Vec<(String, String)> = units.iter().map(|(n, s, _)| (n.clone(), s.clone())).collect();
    let parsed: Vec<File> = units.into_iter().map(|(_, _, f)| f).collect();
    let compiled = match ovc::compile_parsed(parsed, &options) {
        Ok(c) => c,
        Err(e) => return Err(render(&e, &sources)),
    };
    for warning in &compiled.warnings {
        eprintln!("{warning}");
    }
    if args.werror && !compiled.warnings.is_empty() {
        return Err("ovc: warnings are errors (--werror)".into());
    }
    if args.dump_pipeline {
        for table in &compiled.pipelines {
            eprint!("{table}");
        }
    }
    if let Some(path) = &args.emit_tests {
        let text = compiled.tests_sv.as_ref().ok_or("ovc: --emit-tests: the input has no test blocks")?;
        std::fs::write(path, text).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    if let Some(path) = &args.emit_probes {
        let text = compiled.probes_sv.as_ref().ok_or("ovc: --emit-probes: the input has no probe blocks")?;
        std::fs::write(path, text).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    // the outputs derived from the register maps and the memory maps (the same sections as the Python compiler)
    if let Some(path) = &args.emit_c {
        let parts = [
            (!compiled.regmaps.is_empty()).then(|| ovc::regmap::c_header(&compiled.regmaps)),
            (!compiled.records.is_empty()).then(|| ovc::outputs::record_c_header(&compiled.records)),
            (!compiled.memmaps.is_empty()).then(|| ovc::outputs::memmap_c_header(&compiled.memmaps)),
        ];
        let text: Vec<String> = parts.into_iter().flatten().filter(|x| !x.is_empty()).collect();
        if text.is_empty() {
            return Err("ovc: --emit-c: no regmap, exported struct or memmap".into());
        }
        std::fs::write(path, text.join("
")).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    if let Some(path) = &args.emit_isa_sv {
        if compiled.isas.is_empty() {
            return Err("ovc: --emit-isa-sv: no isa".into());
        }
        std::fs::write(path, ovc::outputs::isa_sv_package(&compiled.isas)).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    if let Some(path) = &args.emit_isa_py {
        if compiled.isas.is_empty() {
            return Err("ovc: --emit-isa-py: no isa".into());
        }
        std::fs::write(path, ovc::outputs::isa_python_module(&compiled.isas)).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    if let Some(path) = &args.emit_py {
        if compiled.records.is_empty() {
            return Err("ovc: --emit-py: no exported struct".into());
        }
        std::fs::write(path, ovc::outputs::record_python_module(&compiled.records)).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    if let Some(path) = &args.emit_dts {
        if compiled.memmaps.is_empty() {
            return Err("ovc: --emit-dts: no memmap (in a root module)".into());
        }
        std::fs::write(path, ovc::outputs::memmap_dts(&compiled.memmaps)).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    if let Some(path) = &args.emit_doc {
        let parts = [
            (!compiled.regmaps.is_empty()).then(|| ovc::regmap::markdown(&compiled.regmaps)),
            (!compiled.isas.is_empty()).then(|| ovc::outputs::isa_markdown(&compiled.isas)),
            (!compiled.records.is_empty()).then(|| ovc::outputs::record_markdown(&compiled.records)),
            (!compiled.memmaps.is_empty()).then(|| ovc::outputs::memmap_markdown(&compiled.memmaps)),
        ];
        let text: Vec<String> = parts.into_iter().flatten().filter(|x| !x.is_empty()).collect();
        if text.is_empty() {
            return Err("ovc: --emit-doc: no regmap, isa, exported struct or memmap".into());
        }
        std::fs::write(path, text.join("
")).map_err(|e| format!("ovc: {path}: {e}"))?;
    }
    if args.build {
        return write_project(&args, &compiled);
    }
    match &args.output {
        Some(path) => std::fs::write(path, &compiled.sv).map_err(|e| format!("ovc: {path}: {e}")),
        None => {
            print!("{}", compiled.sv);
            Ok(())
        }
    }
}

/// `ovc build`: one `.sv` per source file under the output folder, and the file list.
fn write_project(args: &Args, compiled: &ovc::Compiled) -> Result<(), String> {
    let out_dir = PathBuf::from(args.output.clone().unwrap_or_else(|| "out".into()));
    let root = PathBuf::from(args.root.clone().unwrap_or_else(|| ".".into()));
    let mut list = Vec::new();
    // the packages first: every file that uses a packed struct type comes after them in the list
    if !compiled.packages.is_empty() {
        let target = out_dir.join("_packages.sv");
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("ovc: {}: {e}", out_dir.display()))?;
        std::fs::write(&target, &compiled.packages).map_err(|e| format!("ovc: {}: {e}", target.display()))?;
        list.push(target.to_string_lossy().replace('\\', "/"));
    }
    for (origin, text) in &compiled.by_origin {
        if origin.starts_with('<') {
            continue;
        }
        let source = PathBuf::from(origin);
        let relative = source.strip_prefix(&root).unwrap_or(&source).with_extension("sv");
        let target = out_dir.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("ovc: {}: {e}", parent.display()))?;
        }
        std::fs::write(&target, text).map_err(|e| format!("ovc: {}: {e}", target.display()))?;
        list.push(target.to_string_lossy().replace('\\', "/"));
    }
    if let Some((_, text)) = compiled.by_origin.iter().find(|(o, _)| o.starts_with('<') && o.as_str() == "<stdlib>") {
        let target = out_dir.join("_library.sv");
        std::fs::write(&target, text).map_err(|e| format!("ovc: {}: {e}", target.display()))?;
        list.push(target.to_string_lossy().replace('\\', "/"));
    }
    let file_list = out_dir.join("files.f");
    std::fs::write(&file_list, list.join("\n") + "\n").map_err(|e| format!("ovc: {}: {e}", file_list.display()))?;
    eprintln!("ovc: {} file(s) -> {}; file list: {}", list.len(), out_dir.display(), file_list.display());
    Ok(())
}

fn render(e: &Error, sources: &[(String, String)]) -> String {
    match sources.iter().find(|(name, _)| *name == e.file) {
        Some((_, src)) => e.render(src),
        None => e.to_string(),
    }
}
