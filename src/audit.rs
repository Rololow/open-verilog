//! `ovc audit`: the places where a design asks the compiler to trust the author. Nothing is changed and nothing is an
//! error: the report lists, file by file, the `async` ports and declarations (their values are not checked for a clock
//! crossing), the `same_domain` declarations (the clocks are taken as synchronous), the `// ovc: ignore` comments, the
//! truncations written on purpose (`.wrap()`), and the lint levels that are below their defaults.

use std::fmt::Write;

use crate::ast::{File, Item, ItemKind, Module};
use crate::diag::{Level, Levels};

/// The units of a design: the file name, its text and its syntax tree.
pub type Unit = (String, String, File);

/// One line of the report: the file and its line, and the text that is listed.
struct Entry {
    file: String,
    line: usize,
    text: String,
}

/// The report of the audit, by section.
#[derive(Default)]
struct Report {
    async_ports: Vec<Entry>,
    async_decls: Vec<Entry>,
    same_domain: Vec<Entry>,
    ignores: Vec<Entry>,
    /// The `.wrap()` of each file: (file, count).
    wraps: Vec<(String, usize)>,
}

/// The audit of the units, with the levels in effect (a level below its default is listed).
pub fn report(units: &[Unit], levels: &Levels) -> String {
    let mut r = Report::default();
    for (name, src, file) in units {
        for m in &file.modules {
            scan_module(name, m, &mut r);
        }
        scan_text(name, src, &mut r);
    }
    render(&r, units.len(), levels)
}

fn scan_module(file: &str, m: &Module, r: &mut Report) {
    for p in &m.ports {
        if p.is_async {
            r.async_ports.push(Entry { file: file.into(), line: p.pos.0, text: format!("{}: input {}", m.name, p.name) });
        }
    }
    scan_items(file, &m.name, &m.items, r);
}

/// The declarations of a module, including the ones of its generate loops, conditions and stages.
fn scan_items(file: &str, module: &str, items: &[Item], r: &mut Report) {
    for it in items {
        match &it.kind {
            ItemKind::Decl(d) if d.is_async => {
                let names: Vec<&str> = d.decls.iter().map(|x| x.name.as_str()).collect();
                r.async_decls.push(Entry { file: file.into(), line: it.pos.0, text: format!("{module}: {}", names.join(", ")) });
            }
            ItemKind::SameDomain(names) => {
                r.same_domain.push(Entry { file: file.into(), line: it.pos.0, text: format!("{module}: {}", names.join(", ")) });
            }
            ItemKind::GenFor { body, .. } | ItemKind::Stage { items: body, .. } => scan_items(file, module, body, r),
            ItemKind::GenIf { then, els, .. } => {
                scan_items(file, module, then, r);
                if let Some(e) = els {
                    scan_items(file, module, e, r);
                }
            }
            _ => {}
        }
    }
}

/// The comments and the `.wrap()` calls, read from the text: an ignore comment silences its line, and a `.wrap()` is a
/// truncation written on purpose. A `.wrap()` in a comment is not counted.
fn scan_text(file: &str, src: &str, r: &mut Report) {
    let mut wraps = 0;
    for (i, line) in src.lines().enumerate() {
        if let Some(at) = line.find("ovc: ignore") {
            r.ignores.push(Entry { file: file.into(), line: i + 1, text: line[at..].trim().into() });
        }
        let code = line.split("//").next().unwrap_or("");
        wraps += code.matches(".wrap()").count();
    }
    if wraps > 0 {
        r.wraps.push((file.into(), wraps));
    }
}

fn render(r: &Report, files: usize, levels: &Levels) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "ovc audit: {files} file(s)");
    section(&mut out, "async inputs (the value comes from another clock; it is not checked)", &r.async_ports);
    section(&mut out, "async declarations (storage read in another domain; not checked)", &r.async_decls);
    section(&mut out, "same_domain (the clocks are taken as synchronous; not checked)", &r.same_domain);
    section(&mut out, "ignored findings (// ovc: ignore)", &r.ignores);

    let _ = writeln!(out, "truncations written on purpose (.wrap()):");
    if r.wraps.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for (file, n) in &r.wraps {
        let _ = writeln!(out, "  {file}: {n}");
    }

    let lowered = lowered_levels(levels);
    let _ = writeln!(out, "lint levels below their defaults:");
    if lowered.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for line in &lowered {
        let _ = writeln!(out, "  {line}");
    }
    out
}

fn section(out: &mut String, title: &str, entries: &[Entry]) {
    let _ = writeln!(out, "{title}:");
    if entries.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for e in entries {
        let _ = writeln!(out, "  {}:{}  {}", e.file, e.line, e.text);
    }
}

/// The categories whose level is not the default one, when the default is `deny` and the level is lower.
fn lowered_levels(levels: &Levels) -> Vec<String> {
    let defaults = Levels::default();
    let mut out = Vec::new();
    for cat in crate::diag::ALL {
        let (now, def) = (levels.get(cat), defaults.get(cat));
        if now != def && rank(now) < rank(def) {
            out.push(format!("{} = {} (default: {})", cat.as_str(), name(now), name(def)));
        }
    }
    out
}

fn rank(level: Level) -> u8 {
    match level {
        Level::Allow => 0,
        Level::Warn => 1,
        Level::Deny => 2,
    }
}

fn name(level: Level) -> &'static str {
    match level {
        Level::Allow => "allow",
        Level::Warn => "warn",
        Level::Deny => "deny",
    }
}
