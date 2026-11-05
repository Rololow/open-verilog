//! `ovc fmt`: the layout of `.ov` files. Only the whitespace changes: the indentation follows the brackets (two spaces a
//! level), the trailing spaces go, and there is at most one blank line in a row. The tokens are checked to be the same
//! before a file is written, so a formatter bug cannot change the design. Formatting is idempotent.

use std::fs;
use std::path::{Path, PathBuf};

use crate::lexer::lex;

/// The words that open a block without brackets, and the words that close one. `module`, `interface` and `class` are not
/// here: a module of `.ov` opens with a bracket (`module M(...) {`), and counting its keyword too would count it twice.
const OPENERS: &[&str] = &[
    "package", "function", "task", "begin", "case", "casex", "casez", "generate", "fork", "covergroup", "program",
];
const CLOSERS: &[&str] = &[
    "endpackage", "endfunction", "endtask", "end", "endcase", "endgenerate", "join", "join_any", "join_none",
    "endprogram", "endgroup", "endmodule",
];

/// For each line, whether it starts a SystemVerilog module (its header ends with `;`, not with `{`): such a module closes
/// with `endmodule`, and its body is indented. The header is read up to the character that ends it.
fn sv_modules(lines: &[&str]) -> Vec<bool> {
    let codes: Vec<String> = lines.iter().map(|l| code_of(l).0).collect();
    (0..lines.len())
        .map(|i| {
            let first = codes[i].split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).find(|w| !w.is_empty());
            if first != Some("module") {
                return false;
            }
            let mut depth = 0i64;
            for code in &codes[i..] {
                for c in code.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        '{' if depth == 0 => return false,
                        ';' if depth == 0 => return true,
                        _ => {}
                    }
                }
            }
            false
        })
        .collect()
}

/// What a line of code does to the blocks: how many it opens and closes, the closers it starts with, and whether it
/// leaves a block comment open.
struct Blocks {
    opens: i64,
    closes: i64,
    leading_closes: i64,
    opens_block_comment: bool,
}

/// The code of a line: its text without the comments, with the strings blanked out (so their brackets and words do not
/// count), and whether a block comment is left open at its end.
fn code_of(line: &str) -> (String, bool) {
    let b = line.as_bytes();
    let mut code = String::new();
    let mut in_string = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_string {
            if c == b'\\' {
                i += 1;
            } else if c == b'"' {
                in_string = false;
            }
            code.push(' ');
        } else if c == b'"' {
            in_string = true;
            code.push(' ');
        } else if b[i..].starts_with(b"//") {
            break;
        } else if b[i..].starts_with(b"/*") {
            match line[i + 2..].find("*/") {
                Some(end) => {
                    code.push(' ');
                    i += end + 4;
                    continue;
                }
                None => return (code, true),
            }
        } else {
            code.push(c as char);
        }
        i += 1;
    }
    (code, false)
}

/// The blocks a line opens and closes. `sv_module`: the line starts a SystemVerilog module (see `sv_modules`).
fn blocks(line: &str, sv_module: bool) -> Blocks {
    let (code, opens_block_comment) = code_of(line);
    // the tokens of the code: a word (an identifier) or a single other character
    let mut tokens: Vec<&str> = Vec::new();
    let mut start = None;
    for (i, c) in code.char_indices() {
        let word_char = c.is_ascii_alphanumeric() || c == '_' || c == '$';
        match (start, word_char) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                tokens.push(&code[s..i]);
                start = None;
                if !c.is_whitespace() {
                    tokens.push(&code[i..i + c.len_utf8()]);
                }
            }
            (None, false) if !c.is_whitespace() => tokens.push(&code[i..i + c.len_utf8()]),
            _ => {}
        }
    }
    if let Some(s) = start {
        tokens.push(&code[s..]);
    }
    let is_closer = |t: &str| matches!(t, ")" | "]" | "}") || CLOSERS.contains(&t);
    // a line that opens a brace (`case (op) {`) is closed by the brace: its keyword does not open a second block
    let braced = tokens.contains(&"{");
    let mut opens = 0;
    let mut closes = 0;
    for t in &tokens {
        let keyword_opener = (OPENERS.contains(t) && !braced) || (sv_module && *t == "module");
        if matches!(*t, "(" | "[" | "{") || keyword_opener {
            opens += 1;
        } else if is_closer(t) {
            closes += 1;
        }
    }
    // the closers at the start of the line: they are read before the line is indented
    let leading_closes = tokens.iter().take_while(|t| is_closer(t)).count() as i64;
    Blocks { opens, closes, leading_closes, opens_block_comment }
}

/// The source, formatted. Idempotent: the result formats to itself.
///
/// A line that starts in a bracket is aligned on the column after the bracket (`module M(input a,` and the next line
/// under `input`), or two spaces deeper when the bracket ends its line; a closing bracket at the start of a line goes
/// back to the indentation of the line that opened it. Other lines follow the blocks.
pub fn format(src: &str) -> String {
    let crlf = src.contains("\r\n");
    let text = src.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let sv = sv_modules(&lines);
    let mut out: Vec<String> = Vec::new();
    // the open brackets: (indent of the lines they hold, indent of the line that closes them)
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0i64;
    let mut in_block_comment = false;
    for (n, raw) in lines.iter().enumerate() {
        if in_block_comment {
            // the lines of a block comment keep their text: only the trailing spaces go
            out.push(raw.trim_end().to_string());
            if raw.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
            continue;
        }
        let b = blocks(line, sv[n]);
        let (code, _) = code_of(line);
        let closes_bracket = matches!(line.as_bytes()[0], b')' | b']');
        let indent = match stack.last() {
            Some(&(inside, closing)) => {
                if closes_bracket {
                    closing
                } else {
                    inside
                }
            }
            None => ((depth - b.leading_closes).max(0) * 2) as usize,
        };
        out.push(format!("{}{}", " ".repeat(indent), line));
        for (k, c) in code.char_indices() {
            match c {
                '(' | '[' => {
                    let hanging = code[k + 1..].trim().is_empty();
                    let inside = if hanging { indent + 2 } else { indent + k + 1 };
                    stack.push((inside, indent));
                }
                ')' | ']' => {
                    stack.pop();
                }
                _ => {}
            }
        }
        depth = (depth + b.opens - b.closes).max(0);
        in_block_comment = b.opens_block_comment;
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut result = out.join(eol);
    result.push_str(eol);
    result
}

/// The tokens of a source, without its layout: what the compiler reads.
fn tokens(src: &str) -> Option<Vec<String>> {
    lex(src, "").ok().map(|ts| ts.into_iter().map(|t| t.text).collect())
}

/// The formatted source, if it has the same tokens as the original (`None` otherwise, and for a source that does not
/// lex: such a file is not touched).
pub fn checked_format(src: &str) -> Option<String> {
    let out = format(src);
    (tokens(src)? == tokens(&out)?).then_some(out)
}

/// The `.ov` files under `paths` (directories are searched; build and hidden folders are skipped).
fn collect(paths: &[String]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !name.starts_with('.') && !matches!(name.as_str(), "target" | "out" | "node_modules") {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "ov") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_dir() {
            walk(&path, &mut out);
        } else {
            out.push(path);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `ovc fmt [--check] PATH...`: formats the files in place. With `--check`, lists the files that would change and fails
/// if there are some (for the continuous integration). Returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let check = args.iter().any(|a| a == "--check");
    let paths: Vec<String> = args.iter().filter(|a| *a != "--check").cloned().collect();
    if paths.is_empty() {
        eprintln!("usage: ovc fmt [--check] PATH...   (the .ov files, or the folders that hold them)");
        return 2;
    }
    let mut failed = false;
    let mut changed = 0;
    for path in collect(&paths) {
        let src = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ovc fmt: {}: {e}", path.display());
                failed = true;
                continue;
            }
        };
        let Some(out) = checked_format(&src) else {
            eprintln!("ovc fmt: {}: not formatted (the tokens would change, or the file does not lex)", path.display());
            failed = true;
            continue;
        };
        if out == src {
            continue;
        }
        changed += 1;
        if check {
            println!("would reformat: {}", path.display());
        } else if let Err(e) = fs::write(&path, out) {
            eprintln!("ovc fmt: {}: {e}", path.display());
            failed = true;
        } else {
            println!("formatted: {}", path.display());
        }
    }
    if failed || (check && changed > 0) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_indentation_follows_the_brackets() {
        let src = "module M(\ninput logic a,\noutput logic y\n) {\nassign y = a;\nalways_comb {\nif (a) {\ny = 1;\n} else {\ny = 0;\n}\n}\n}\n";
        let expect = "module M(\n  input logic a,\n  output logic y\n) {\n  assign y = a;\n  always_comb {\n    if (a) {\n      y = 1;\n    } else {\n      y = 0;\n    }\n  }\n}\n";
        assert_eq!(format(src), expect);
    }

    #[test]
    fn the_blank_lines_are_at_most_one_and_the_file_ends_with_one_newline() {
        assert_eq!(format("\n\na\n\n\n\nb   \n\n"), "a\n\nb\n");
    }

    #[test]
    fn a_comment_keeps_its_text_and_its_brackets_do_not_count() {
        let src = "module M() {\n// a note with ( open\nassign y = a; // ) not a closer\n}\n";
        assert_eq!(format(src), "module M() {\n  // a note with ( open\n  assign y = a; // ) not a closer\n}\n");
        let block = "module M() {\n/* a block\n    kept as it is  \n*/\nassign y = a;\n}\n";
        assert_eq!(format(block), "module M() {\n  /* a block\n    kept as it is\n*/\n  assign y = a;\n}\n");
    }

    #[test]
    fn the_keyword_blocks_are_indented_too() {
        let src = "package Defs;\ntypedef logic [7:0] b_t;\nendpackage\nmodule M(input logic a) {\nfunction automatic logic f(input logic x);\nreturn ~x;\nendfunction\nalways_comb begin\ncase (a)\n1: y = 1;\ndefault: y = 0;\nendcase\nend\n}\n";
        let expect = "package Defs;\n  typedef logic [7:0] b_t;\nendpackage\nmodule M(input logic a) {\n  function automatic logic f(input logic x);\n    return ~x;\n  endfunction\n  always_comb begin\n    case (a)\n      1: y = 1;\n      default: y = 0;\n    endcase\n  end\n}\n";
        assert_eq!(format(src), expect);
    }

    #[test]
    fn a_systemverilog_module_is_indented_up_to_its_endmodule() {
        let src = "module Pack (input logic a,\noutput logic y);\nassign y = a;\nendmodule\nmodule Q(input logic b) {\nassign y = b;\n}\n";
        let expect = "module Pack (input logic a,\n             output logic y);\n  assign y = a;\nendmodule\nmodule Q(input logic b) {\n  assign y = b;\n}\n";
        assert_eq!(format(src), expect);
    }

    #[test]
    fn a_case_with_braces_is_closed_by_the_brace() {
        let src = "module M(input logic [1:0] op) {
always_comb {
case (op) {
0: y = 1;
default: y = 0;
}
}
}
";
        let expect = "module M(input logic [1:0] op) {
  always_comb {
    case (op) {
      0: y = 1;
      default: y = 0;
    }
  }
}
";
        assert_eq!(format(src), expect);
    }

    #[test]
    fn a_string_with_brackets_does_not_count() {
        let src = "module M() {\nfn s() { $display(\"(\"); }\n}\n";
        assert_eq!(format(src), "module M() {\n  fn s() { $display(\"(\"); }\n}\n");
    }

    #[test]
    fn formatting_is_idempotent() {
        let src = "module M(input logic a) {\n      assign y=a;\n\n\n  always_comb { if (a) { y=1; } }\n}\n";
        let once = format(src);
        assert_eq!(format(&once), once);
    }

    #[test]
    fn the_line_endings_stay() {
        assert_eq!(format("a\r\n  b\r\n"), "a\r\nb\r\n");
    }

    #[test]
    fn the_tokens_are_the_same_or_the_format_is_refused() {
        assert!(checked_format("module M() {}\n").is_some());
        assert_eq!(tokens("a = b;"), tokens("a   =\n  b ;"));
    }
}
