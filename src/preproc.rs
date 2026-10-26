//! The SystemVerilog preprocessor of `ovc lint`: `define` (with arguments), `undef`, `ifdef` / `ifndef` / `elsif` /
//! `else` / `endif`, `include` (search paths), and the uses of macros. Comments and `(* attributes *)` are removed; the
//! other compiler directives (`timescale`, `default_nettype`, ...) are dropped. Line numbers are kept: removed lines
//! become empty, and an included file is inlined line by line, so that `origin` maps every output line back to the
//! file and line it was written on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const IGNORED: &[&str] = &[
    "timescale", "default_nettype", "resetall", "celldefine", "endcelldefine", "nounconnected_drive", "unconnected_drive",
    "pragma", "line", "begin_keywords", "end_keywords", "protect", "endprotect",
];

/// A macro: its parameter names (empty for a macro without arguments) and its body.
type Macro = (Vec<String>, String);

/// One level of `ifdef`: whether the lines are kept, whether a branch was taken, and whether the enclosing text is kept.
#[derive(Clone, Copy)]
struct Branch {
    active: bool,
    taken: bool,
    parent_active: bool,
}

pub struct Preprocessor {
    macros: HashMap<String, Macro>,
    incdirs: Vec<PathBuf>,
    depth: usize,
    /// For each output line of the last file: the file and the line it comes from.
    pub origin: Vec<(String, usize)>,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The text without its comments: strings are kept, and so are the newlines (line numbers do not move).
fn strip_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '"' {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(chars.len());
            out.extend(&chars[i..end]);
            i = end;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut j = i + 2;
            while j + 1 < chars.len() && !(chars[j] == '*' && chars[j + 1] == '/') {
                j += 1;
            }
            let end = (j + 2).min(chars.len());
            out.extend(chars[i..end].iter().filter(|&&x| x == '\n'));
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// `(* name ... *)` attributes replaced by spaces (the length is kept).
fn blank_attributes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = chars.clone();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '(' && chars[i + 1] == '*' {
            let mut j = i + 2;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let named = j < chars.len() && (chars[j].is_ascii_alphabetic() || chars[j] == '_');
            let mut k = j;
            while k + 1 < chars.len() && chars[k] != '*' {
                k += 1;
            }
            if named && k + 1 < chars.len() && chars[k + 1] == ')' {
                for x in out.iter_mut().take(k + 2).skip(i) {
                    if *x != '\n' {
                        *x = ' ';
                    }
                }
                i = k + 2;
                continue;
            }
        }
        i += 1;
    }
    out.into_iter().collect()
}

/// The end labels (`endmodule : name`) without their label.
fn drop_end_labels(line: &str) -> String {
    const KINDS: &[&str] = &["module", "package", "interface", "function", "task", "generate", "case", ""];
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let at_word_start = i == 0 || !is_word(chars[i - 1]);
        if at_word_start && chars[i..].iter().take(3).collect::<String>() == "end" {
            let mut j = i + 3;
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let kind: String = chars[i + 3..j].iter().collect();
            if KINDS.contains(&kind.as_str()) {
                let mut k = j;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                if k < chars.len() && chars[k] == ':' {
                    let mut m = k + 1;
                    while m < chars.len() && chars[m].is_whitespace() {
                        m += 1;
                    }
                    let start = m;
                    while m < chars.len() && is_word(chars[m]) {
                        m += 1;
                    }
                    if m > start {
                        out.extend(&chars[i..j]);
                        i = m;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The directive a line starts with (`` `ifdef X ``): its name, and the rest of the line.
fn directive(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix('`')?;
    let name: String = rest.chars().take_while(|&c| is_word(c)).collect();
    if name.is_empty() {
        return None;
    }
    let after = rest[name.len()..].trim().to_string();
    Some((name, after))
}

/// Whole-word replacement of a parameter by its argument.
fn replace_word(text: &str, word: &str, by: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let target: Vec<char> = word.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let matches = chars[i..].starts_with(&target)
            && (i == 0 || !is_word(chars[i - 1]))
            && chars.get(i + target.len()).is_none_or(|&c| !is_word(c));
        if matches {
            out.push_str(by);
            i += target.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

impl Preprocessor {
    /// A preprocessor with the defines of the command line (`-DNAME=value`) and the include directories.
    pub fn new(defines: &[(String, String)], incdirs: &[String]) -> Self {
        Preprocessor {
            macros: defines.iter().map(|(k, v)| (k.clone(), (Vec::new(), v.clone()))).collect(),
            incdirs: incdirs.iter().map(PathBuf::from).collect(),
            depth: 0,
            origin: Vec::new(),
        }
    }

    /// The preprocessed text of one file; the macros it defines stay for the files after it.
    pub fn run(&mut self, text: &str, file: &str) -> Result<String> {
        if self.depth == 0 {
            self.origin.clear();
        }
        Ok(self.run_lines(text, file)?.join("\n"))
    }

    fn run_lines(&mut self, text: &str, file: &str) -> Result<Vec<String>> {
        let text = blank_attributes(&strip_comments(text));
        let lines: Vec<String> = text.split('\n').map(drop_end_labels).collect();
        let mut out: Vec<String> = Vec::new();
        let mut stack: Vec<Branch> = Vec::new();
        let mut k = 0;
        while k < lines.len() {
            let mut line = lines[k].clone();
            // a `define continued on the next lines
            while line.trim_end().ends_with('\\') && k + 1 < lines.len() && line.contains("`define") {
                line = format!("{} {}", line.trim_end().trim_end_matches('\\'), lines[k + 1]);
                k += 1;
                self.emit(&mut out, String::new(), file, k + 1);
            }
            let active = stack.iter().all(|b| b.active);
            if let Some((name, rest)) = directive(&line) {
                if matches!(name.as_str(), "ifdef" | "ifndef" | "elsif" | "else" | "endif") {
                    let macro_name: String = rest.chars().take_while(|&c| is_word(c)).collect();
                    match name.as_str() {
                        "ifdef" | "ifndef" => {
                            let cond = self.macros.contains_key(&macro_name) == (name == "ifdef");
                            stack.push(Branch { active: active && cond, taken: cond, parent_active: active });
                        }
                        _ if stack.is_empty() => {
                            return Err(Error::new(format!("`{name} without `ifdef"), file, k + 1, 1));
                        }
                        "elsif" => {
                            let top = stack.last_mut().expect("checked above");
                            let cond = !top.taken && self.macros.contains_key(&macro_name);
                            top.active = top.parent_active && cond;
                            top.taken |= cond;
                        }
                        "else" => {
                            let top = stack.last_mut().expect("checked above");
                            top.active = top.parent_active && !top.taken;
                            top.taken = true;
                        }
                        _ => {
                            stack.pop();
                        }
                    }
                    self.emit(&mut out, String::new(), file, k + 1);
                    k += 1;
                    continue;
                }
            }
            if !active {
                self.emit(&mut out, String::new(), file, k + 1);
                k += 1;
                continue;
            }
            if let Some(rest) = line.trim_start().strip_prefix("`define") {
                if rest.starts_with(char::is_whitespace) {
                    self.define(rest.trim_start());
                    self.emit(&mut out, String::new(), file, k + 1);
                    k += 1;
                    continue;
                }
            }
            if let Some(rest) = line.trim_start().strip_prefix("`undef") {
                let name: String = rest.trim_start().chars().take_while(|&c| is_word(c)).collect();
                self.macros.remove(&name);
                self.emit(&mut out, String::new(), file, k + 1);
                k += 1;
                continue;
            }
            if let Some(rest) = line.trim_start().strip_prefix("`include") {
                let rest = rest.trim_start();
                if let Some(name) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
                    let included = self.include(name, file, k + 1)?;
                    out.extend(included);
                    k += 1;
                    continue;
                }
            }
            let expanded = self.expand(&line, file, k + 1, 0)?;
            self.emit(&mut out, expanded, file, k + 1);
            k += 1;
        }
        if !stack.is_empty() {
            return Err(Error::new("`ifdef without `endif", file, lines.len(), 1));
        }
        Ok(out)
    }

    fn emit(&mut self, out: &mut Vec<String>, line: String, file: &str, at: usize) {
        out.push(line);
        self.origin.push((file.to_string(), at));
    }

    /// `define NAME [(params)] body`.
    fn define(&mut self, text: &str) {
        let name: String = text.chars().take_while(|&c| is_word(c)).collect();
        let after = &text[name.len()..];
        let (params, body) = match after.strip_prefix('(') {
            Some(rest) => {
                let (list, body) = rest.split_once(')').unwrap_or((rest, ""));
                let params = list.split(',').map(|p| p.split('=').next().unwrap_or("").trim().to_string()).collect();
                (params, body)
            }
            None => (Vec::new(), after),
        };
        self.macros.insert(name, (params, body.replace('\n', " ").trim().to_string()));
    }

    /// The lines of an included file, found next to the including file or in the include directories.
    fn include(&mut self, name: &str, file: &str, line: usize) -> Result<Vec<String>> {
        let base = if file.starts_with('<') { PathBuf::from(".") } else { Path::new(file).parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf) };
        let mut dirs = vec![base];
        dirs.extend(self.incdirs.iter().cloned());
        for d in dirs {
            let p = d.join(name);
            if p.exists() {
                if self.depth > 20 {
                    return Err(Error::new(format!("`include nested too deeply ({name})"), file, line, 1));
                }
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                self.depth += 1;
                let result = self.run_lines(&text, &p.to_string_lossy().replace('\\', "/"));
                self.depth -= 1;
                return result;
            }
        }
        Err(Error::new(format!("`include file '{name}' not found (add +incdir+<dir>)"), file, line, 1))
    }

    /// The line with its macro uses replaced by their bodies.
    fn expand(&mut self, line: &str, file: &str, lineno: usize, depth: usize) -> Result<String> {
        if !line.contains('`') || depth > 30 {
            return Ok(line.to_string());
        }
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] != '`' {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && is_word(chars[j]) {
                j += 1;
            }
            let name: String = chars[start..j].iter().collect();
            if IGNORED.contains(&name.as_str()) {
                // the rest of the line is the directive's argument
                break;
            }
            let (params, body) = match self.macros.get(&name) {
                Some(m) => m.clone(),
                None => return Err(Error::new(format!("macro `{name} is not defined"), file, lineno, i + 1)),
            };
            let mut body = body;
            if !params.is_empty() {
                let (args, after) = arguments(&chars, j, file, lineno)?;
                for (p, a) in params.iter().zip(args.iter().map(String::as_str).chain(std::iter::repeat(""))) {
                    body = replace_word(&body, p, a.trim());
                }
                out.push_str(&self.expand(&body.replace("``", ""), file, lineno, depth + 1)?);
                i = after;
                continue;
            }
            out.push_str(&self.expand(&body.replace("``", ""), file, lineno, depth + 1)?);
            i = j;
        }
        Ok(out)
    }
}

/// The arguments of a macro use: the text between the parentheses after the name, split at the top-level commas.
fn arguments(chars: &[char], at: usize, file: &str, lineno: usize) -> Result<(Vec<String>, usize)> {
    let mut j = at;
    while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
        j += 1;
    }
    if chars.get(j) != Some(&'(') {
        return Err(Error::new("macro arguments expected", file, lineno, j + 1));
    }
    let mut depth = 0;
    let mut args = Vec::new();
    let mut cur = String::new();
    for (k, &c) in chars.iter().enumerate().skip(j + 1) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 && c == ')' => {
                args.push(cur);
                return Ok((args, k + 1));
            }
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                args.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    Err(Error::new("unterminated macro arguments", file, lineno, j + 1))
}
