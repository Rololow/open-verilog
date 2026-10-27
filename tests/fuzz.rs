//! Random expressions: ovc (parse -> fold -> emit) must agree with Questa evaluating the same source text. This catches
//! precedence / parenthesization bugs in the parser and emitter, and constant-folding differences with SystemVerilog
//! semantics. Ported from the Python `tests/test_fuzz.py`; the Questa comparison runs when vsim is on the PATH.

mod common;

use std::fs;

const BINOPS: [&str; 19] = ["+", "-", "*", "/", "%", "<<", ">>", ">>>", "&", "|", "^", "==", "!=", "<", "<=", ">", ">=", "&&", "||"];
const UNOPS: [&str; 6] = ["~", "-", "!", "&", "|", "^"];
const SIGNALS: [&str; 4] = ["x0", "x1", "x2", "x3"];

/// A small deterministic generator (xorshift): the same seed gives the same expressions.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// A number in `0..1`.
    fn unit(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

fn is_atom(s: &str) -> bool {
    SIGNALS.contains(&s) || s.chars().all(|c| c.is_ascii_digit())
}

fn gen(rng: &mut Rng, depth: usize, top: bool) -> String {
    let r = rng.unit();
    if depth == 0 || r < 0.2 {
        return if rng.unit() < 0.6 { rng.pick(&SIGNALS).to_string() } else { (1 + rng.below(9)).to_string() };
    }
    if r < 0.35 {
        let sub = gen(rng, depth - 1, false);
        let op = rng.pick(&UNOPS);
        // SV only allows a primary after a unary operator (`!~x` is illegal), .ov accepts both
        return if is_atom(&sub) { format!("{op}{sub}") } else { format!("{op}({sub})") };
    }
    if r < 0.45 {
        let t = format!("{} ? {} : {}", gen(rng, depth - 1, false), gen(rng, depth - 1, false), gen(rng, depth - 1, false));
        return if top { t } else { format!("({t})") };
    }
    let op = rng.pick(&BINOPS);
    let text = format!("{} {op} {}", gen(rng, depth - 1, false), gen(rng, depth - 1, false));
    if rng.unit() < 0.3 { format!("({text})") } else { text }
}

/// `n` expressions that compile on their own (a constant division by zero, for one, does not).
fn expressions(n: usize, seed: u64) -> Vec<String> {
    let mut rng = Rng(seed.max(1));
    let mut out = Vec::new();
    while out.len() < n {
        let e = gen(&mut rng, 4, true);
        let src = format!(
            "module F(input logic [31:0] x0, input logic [31:0] x1, input logic [31:0] x2, \
             input logic [31:0] x3, output logic [31:0] y) {{ assign y = ({e}).wrap(); }}"
        );
        if ovc::compile(&src, "f.ov").is_ok() {
            out.push(e);
        }
    }
    out
}

/// The shape of an expression: its tree without the positions of its nodes (a `, (line, col)` in the debug text).
fn shape(e: &ovc::ast::Expr) -> String {
    let text = format!("{e:?}");
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(", (") {
            if let Some(end) = text[i..].find(')') {
                let inner = &text[i + 3..i + end];
                if inner.split(", ").count() == 2 && inner.split(", ").all(|p| p.parse::<usize>().is_ok()) {
                    i += end + 1;
                    continue;
                }
            }
        }
        out.push(text[i..].chars().next().unwrap_or(' '));
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

#[test]
fn parse_emit_roundtrip() {
    // emit(parse(text)) parses back to the same tree
    for e in expressions(300, 1) {
        let tree = ovc::parser::parse_expr_text(&e, "f.ov").unwrap_or_else(|err| panic!("{e}: {}", err.message));
        let text = ovc::emit::expr::expr(&tree, -1, false).unwrap_or_else(|err| panic!("{e}: {}", err.message));
        let again = ovc::parser::parse_expr_text(&text, "f.ov").unwrap_or_else(|err| panic!("{text}: {}", err.message));
        assert_eq!(shape(&tree), shape(&again), "{e} -> {text}");
    }
}

#[test]
fn expressions_match_questa() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    let exprs = expressions(150, 7);
    let n = exprs.len();
    let body: String = exprs.iter().enumerate().map(|(i, e)| format!("  assign y[{i}] = ({e}).wrap();\n")).collect();
    let src = format!(
        "module Fuzz(input logic [31:0] x0, input logic [31:0] x1, input logic [31:0] x2, \
         input logic [31:0] x3, output logic [31:0] y [{n}]) {{\n{body}}}\n"
    );
    let sv = ovc::compile(&src, "fuzz.ov").unwrap_or_else(|e| panic!("{e}"));
    let checks: String = exprs
        .iter()
        .enumerate()
        .map(|(i, e)| format!("      e = {e};\n      if (y[{i}] !== e) $fatal(1, \"FAIL #{i}: got %h expected %h\", y[{i}], e);\n"))
        .collect();
    let tb = format!(
        "module fuzz_tb;
  logic [31:0] x0, x1, x2, x3, e;
  logic [31:0] y [{n}];
  Fuzz dut (.x0, .x1, .x2, .x3, .y);
  initial begin
    repeat (40) begin
      x0 = $urandom; x1 = $urandom_range(0, 9); x2 = $urandom; x3 = $urandom_range(1, 5);
      #1;
{checks}    end
    $display(\"PASS\");
    $finish;
  end
endmodule
"
    );
    let dir = common::scratch("fuzz");
    fs::write(dir.join("dut.sv"), sv).unwrap();
    fs::write(dir.join("tb.sv"), tb).unwrap();
    let out = common::simulate(&dir, &["-sv", "dut.sv", "tb.sv"], "fuzz_tb");
    let _ = fs::remove_dir_all(&dir);
    // the failing expression, named by its number in the testbench's message
    let failed = out.find("FAIL #").and_then(|at| out[at + 6..].split(':').next()?.trim().parse::<usize>().ok());
    if let Some(k) = failed {
        panic!("expression #{k} differs from the simulation: {}\n{out}", exprs[k]);
    }
    assert!(out.contains("PASS"), "{out}");
}
