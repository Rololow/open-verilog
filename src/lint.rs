//! `ovc lint`: the checks of the compiler on plain SystemVerilog / Verilog files, without rewriting them.
//!
//! The files are preprocessed in order as one compilation unit (macros carry over), parsed, then checked together. A
//! finding is reported with its category, its file and line (the line of the file as written, not of the preprocessed
//! text). A line can silence a category with `// ovc: ignore <category>`. The parts that cannot be read, preprocessed
//! or parsed are reported as not analyzed, and the lint exits with an error.

use std::collections::HashMap;
use std::path::Path;

use crate::ast::File;
use crate::preproc::Preprocessor;

pub const USAGE: &str = "usage: ovc lint FILE.sv... [-f FILES.f] [+incdir+DIR] [-DNAME[=VALUE]] [--top NAME]
                [--only CAT,...] [--ignore CAT,...] [--werror]";

/// The categories of a finding, with the messages each one takes (the first category that matches is the one).
fn category(msg: &str) -> &'static str {
    let has = |s: &str| msg.contains(s);
    if has("clock-domain crossing") {
        "cdc"
    } else if has("depends combinationally on") && has(".ready") {
        "handshake"
    } else if msg.starts_with("reset '") {
        "reset"
    } else if has("not valid SystemVerilog") {
        "syntax"
    } else if has("is already defined at") {
        "duplicate"
    } else if has("needs") && has(" bits") {
        "carry"
    } else if ["carry is lost", "is evaluated on", "bits wide (an unsized literal", "before the shift"].iter().any(|s| has(s)) {
        "sizing"
    } else if has("bits but") || has("does not fit") || has("are dropped") || (has(" bits wide") && has("is ")) {
        "width"
    } else if has("latch") {
        "latch"
    } else if has("driven") || has("never driven") || has("several places") {
        "driver"
    } else if has("not connected") || (has("port '") && has(" of")) {
        "connection"
    } else if ["blocking assignment", "non-blocking", "initial value", "initializer", "reg '"].iter().any(|s| has(s)) {
        "process"
    } else if has("opposite way") || has("out of range") {
        "slice"
    } else if ["negative", "signed", "unsigned"].iter().any(|s| has(s)) {
        "sign"
    } else {
        "other"
    }
}

/// A finding, in the file and at the line it is written on.
#[derive(Debug, Clone)]
pub struct Finding {
    pub category: &'static str,
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

/// The options of one lint run, as given on the command line or in a file list.
#[derive(Default, Debug)]
pub struct Options {
    pub files: Vec<String>,
    pub incdirs: Vec<String>,
    pub defines: Vec<(String, String)>,
    pub filelists: Vec<String>,
    pub top: Option<String>,
    pub only: Vec<String>,
    pub ignore: Vec<String>,
    pub werror: bool,
}

/// One word of a command line or a file list: an include directory, a define, a simulator option, a directory (every
/// HDL file below it) or a file.
fn word(opts: &mut Options, base: &Path, w: &str) {
    if let Some(dirs) = w.strip_prefix("+incdir+") {
        opts.incdirs.extend(dirs.split('+').filter(|d| !d.is_empty()).map(|d| base.join(d).to_string_lossy().replace('\\', "/")));
    } else if let Some(defs) = w.strip_prefix("+define+") {
        for d in defs.split('+').filter(|d| !d.is_empty()) {
            let (k, v) = d.split_once('=').unwrap_or((d, ""));
            opts.defines.push((k.to_string(), v.to_string()));
        }
    } else if let Some(d) = w.strip_prefix("-D") {
        let (k, v) = d.split_once('=').unwrap_or((d, ""));
        opts.defines.push((k.to_string(), v.to_string()));
    } else if w.starts_with('+') || w.starts_with('-') {
        // the other simulator options are not needed here
    } else {
        // a path given from the current directory keeps the spelling it was given with
        let p = if base == Path::new(".") { Path::new(w).to_path_buf() } else { base.join(w) };
        if p.is_dir() {
            let mut found = Vec::new();
            hdl_files(&p, &mut found);
            opts.files.extend(found);
        } else {
            opts.files.push(p.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// The HDL files below a directory, in a stable order.
fn hdl_files(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            hdl_files(&p, out);
        } else if matches!(p.extension().and_then(|e| e.to_str()), Some("sv" | "v" | "svh" | "vh" | "ov")) {
            out.push(p.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// A file list: one path or option per line, with `//` and `#` comments.
fn read_filelist(path: &str, opts: &mut Options) -> std::result::Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("ovc lint: {path}: {e}"))?;
    let base = Path::new(path).parent().map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf);
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").split('#').next().unwrap_or("");
        for w in line.split_whitespace() {
            word(opts, &base, w);
        }
    }
    Ok(())
}

/// Where a line of a preprocessed file was written: its file and line.
struct Origins(HashMap<String, Vec<(String, usize)>>);

impl Origins {
    fn locate(&self, file: &str, line: usize) -> (String, usize) {
        match self.0.get(file) {
            Some(o) if line >= 1 && line <= o.len() => o[line - 1].clone(),
            _ => (file.to_string(), line),
        }
    }
}

/// Splits a warning of the compiler, `file:line:col: warning: message`, into its parts.
fn parse_warning(text: &str) -> Option<(String, usize, usize, String)> {
    let (head, message) = text.split_once(": warning: ")?;
    let mut parts = head.rsplitn(3, ':');
    let col = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    let file = parts.next()?.to_string();
    Some((file, line, col, message.to_string()))
}

/// The result of a lint run: the findings, and the parts that were not analyzed.
pub struct Report {
    pub findings: Vec<Finding>,
    pub skipped: Vec<Finding>,
    /// The number of files given.
    pub files: usize,
}

/// Preprocesses, parses and checks the files together.
pub fn run(opts: &Options) -> Report {
    let mut report = Report { findings: Vec::new(), skipped: Vec::new(), files: opts.files.len() };
    let mut pre = Preprocessor::new(&opts.defines, &opts.incdirs);
    let mut origins = Origins(HashMap::new());
    let mut parsed: Vec<File> = Vec::new();
    let skip = |report: &mut Report, file: &str, line: usize, col: usize, message: String| {
        report.skipped.push(Finding { category: "not analyzed", file: file.to_string(), line, col, message });
    };
    for f in &opts.files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                skip(&mut report, f, 1, 1, format!("cannot read the file: {e}"));
                continue;
            }
        };
        let pre_text = match pre.run(&text, f) {
            Ok(t) => t,
            Err(e) => {
                skip(&mut report, &e.file, e.line, e.col, e.message);
                continue;
            }
        };
        origins.0.insert(f.clone(), pre.origin.clone());
        match crate::parser::parse_file(&pre_text, f) {
            Ok(file) => parsed.push(file),
            Err(e) => {
                let (file, line) = origins.locate(&e.file, e.line);
                skip(&mut report, &file, line, e.col, e.message);
            }
        }
    }
    let top = match opts.top.as_deref().map(|t| crate::parser::parse_top(t, "<--top>")).transpose() {
        Ok(t) => t,
        Err(e) => {
            skip(&mut report, &e.file, e.line, e.col, e.message);
            return report;
        }
    };
    match crate::lint_parsed(parsed, top.as_ref()) {
        Err(e) => {
            let (file, line) = origins.locate(&e.file, e.line);
            skip(&mut report, &file, line, e.col, e.message);
        }
        Ok((warnings, errors)) => {
            let mut push = |file: &str, line: usize, col: usize, message: String| {
                let (file, line) = origins.locate(file, line);
                report.findings.push(Finding { category: category(&message), file, line, col, message });
            };
            for w in warnings {
                if let Some((file, line, col, message)) = parse_warning(&w) {
                    push(&file, line, col, message);
                }
            }
            for e in errors {
                push(&e.file, e.line, e.col, e.message);
            }
        }
    }
    report
}

/// Whether the line of a finding says `// ovc: ignore <category>` (or `all`).
fn silenced(f: &Finding, sources: &mut HashMap<String, Vec<String>>) -> bool {
    let lines = sources.entry(f.file.clone()).or_insert_with(|| {
        std::fs::read_to_string(&f.file).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default()
    });
    let Some(line) = f.line.checked_sub(1).and_then(|i| lines.get(i)) else { return false };
    let Some(at) = line.find("ovc: ignore") else { return false };
    let names = &line[at + "ovc: ignore".len()..];
    names.split(|c: char| c == ',' || c.is_whitespace()).filter(|n| !n.is_empty()).any(|n| n == f.category || n == "all")
}

/// `ovc lint` on the command line: the findings, then a summary. The exit code is 1 when a part was not analyzed, or
/// when `--werror` and there are findings.
pub fn main(args: &[String]) -> i32 {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-f" | "--filelist" => opts.filelists.push(it.next().cloned().unwrap_or_default()),
            "--top" => opts.top = it.next().cloned(),
            "--only" => opts.only = it.next().map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_default(),
            "--ignore" => opts.ignore = it.next().map(|s| s.split(',').filter(|c| !c.is_empty()).map(str::to_string).collect()).unwrap_or_default(),
            "--werror" => opts.werror = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            _ => word(&mut opts, Path::new("."), arg),
        }
    }
    for fl in opts.filelists.clone() {
        if let Err(msg) = read_filelist(&fl, &mut opts) {
            eprintln!("{msg}");
            return 2;
        }
    }
    opts.files.retain(|f| matches!(Path::new(f).extension().and_then(|e| e.to_str()), Some("sv" | "v" | "svh" | "vh" | "ov")));
    if opts.files.is_empty() {
        eprintln!("ovc lint: no .sv / .v file given\n{USAGE}");
        return 2;
    }
    let report = run(&opts);
    let mut sources = HashMap::new();
    let mut kept: Vec<Finding> = Vec::new();
    for f in report.findings {
        let wanted = (opts.only.is_empty() || opts.only.iter().any(|c| c == f.category)) && !opts.ignore.iter().any(|c| c == f.category);
        if wanted && !silenced(&f, &mut sources) && !kept.iter().any(|k| k.message == f.message && k.file == f.file && k.line == f.line) {
            kept.push(f);
        }
    }
    kept.sort_by(|a, b| (a.file.as_str(), a.line).cmp(&(b.file.as_str(), b.line)));
    for f in &kept {
        println!("[{}] {}:{}:{}: {}", f.category, f.file, f.line, f.col, f.message);
    }
    for f in &report.skipped {
        println!("[not analyzed] {}:{}:{}: {}", f.file, f.line, f.col, f.message);
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for f in &kept {
        *counts.entry(f.category).or_insert(0) += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let summary = if counts.is_empty() {
        "no finding".to_string()
    } else {
        counts.iter().map(|(c, n)| format!("{n} {c}")).collect::<Vec<_>>().join(", ")
    };
    let skipped = if report.skipped.is_empty() { String::new() } else { format!("; {} part(s) not analyzed", report.skipped.len()) };
    println!("\n{} file(s): {summary}{skipped}", report.files);
    if !report.skipped.is_empty() || (opts.werror && !kept.is_empty()) {
        1
    } else {
        0
    }
}
