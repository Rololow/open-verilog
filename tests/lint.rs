//! `ovc lint`: the preprocessor, the categories of the findings, and the findings of a run.

use std::fs;
use std::path::PathBuf;

use ovc::lint::{self, Options};
use ovc::preproc::Preprocessor;

mod common;

fn scratch(name: &str) -> PathBuf {
    common::scratch(name)
}

fn pre(text: &str, defines: &[(String, String)]) -> Result<String, ovc::error::Error> {
    Preprocessor::new(defines, &[]).run(text, "t.sv")
}

#[test]
fn a_define_is_expanded_and_an_ifdef_chooses_a_branch() {
    let text = "`define W 8\n`ifdef FAST\n`define D 4\n`else\n`define D 2\n`endif\nlogic [`W-1:0] a [`D];\n";
    let out = pre(text, &[]).unwrap();
    assert!(out.contains("logic [8-1:0] a [2];"), "{out}");
    let fast = pre(text, &[("FAST".into(), String::new())]).unwrap();
    assert!(fast.contains("logic [8-1:0] a [4];"), "{fast}");
}

#[test]
fn a_macro_with_arguments_is_expanded_by_parameter() {
    let out = pre("`define BIT(x, i) x[i]\nassign y = `BIT(a, 3);\n", &[]).unwrap();
    assert!(out.contains("assign y = a[3];"), "{out}");
}

#[test]
fn an_undefined_macro_is_an_error_at_its_place() {
    let err = pre("assign y = `NOPE;\n", &[]).unwrap_err();
    assert_eq!((err.line, err.col), (1, 12));
    assert!(err.message.contains("unknown macro `NOPE"));
}

#[test]
fn an_include_is_found_next_to_the_including_file() {
    let dir = scratch("include");
    fs::write(dir.join("defs.sv"), "`define W 16\n").unwrap();
    let main = dir.join("main.sv");
    fs::write(&main, "`include \"defs.sv\"\nassign y = `W;\n").unwrap();
    let text = fs::read_to_string(&main).unwrap();
    let mut p = Preprocessor::new(&[], &[]);
    let out = p.run(&text, &main.to_string_lossy().replace('\\', "/")).unwrap();
    assert!(out.contains("assign y = 16;"), "{out}");
    // the second line of the output is the second line of main.sv, not of the included file
    let line = p.origin.iter().position(|(f, _)| f.ends_with("main.sv")).unwrap();
    assert_eq!(p.origin[line].1, 2);
}

#[test]
fn comments_are_removed_and_the_lines_do_not_move() {
    let out = pre("// a comment\nassign a = b; /* block\ncomment */ assign c = d;\n", &[]).unwrap();
    let lines: Vec<&str> = out.split('\n').collect();
    assert_eq!(lines[0], "");
    assert!(lines[1].contains("assign a = b;"));
    assert!(lines[2].contains("assign c = d;"));
}

#[test]
fn a_clock_crossing_is_a_cdc_finding_at_the_line_written() {
    let dir = scratch("cdc");
    let file = dir.join("x.sv");
    fs::write(
        &file,
        "module x(input logic clk_a, input logic clk_b, input logic a, output logic q);\n  logic r;\n  always @(posedge clk_a) r <= a;\n  always @(posedge clk_b) q <= r;\nendmodule\n",
    )
    .unwrap();
    let opts = Options { files: vec![file.to_string_lossy().replace('\\', "/")], ..Options::default() };
    let report = lint::run(&opts);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let cdc: Vec<_> = report.findings.iter().filter(|f| f.category == "cdc").collect();
    assert_eq!(cdc.len(), 1, "{:?}", report.findings);
    assert_eq!(cdc[0].line, 4);
}

#[test]
fn a_file_that_does_not_parse_is_not_analyzed() {
    let dir = scratch("broken");
    let file = dir.join("b.sv");
    fs::write(&file, "module b(input logic a\n  assign y = a;\n").unwrap();
    let opts = Options { files: vec![file.to_string_lossy().replace('\\', "/")], ..Options::default() };
    let report = lint::run(&opts);
    assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.skipped[0].category, "not analyzed");
}

fn categories_of(name: &str, text: &str) -> Vec<(String, String)> {
    categories_in(name, text, "d.sv")
}

/// The findings of an Open Verilog source (a `.ov` file: its loops over rows are Open Verilog).
fn categories_of_ov(name: &str, text: &str) -> Vec<(String, String)> {
    categories_in(name, text, "d.ov")
}

fn categories_in(name: &str, text: &str, file_name: &str) -> Vec<(String, String)> {
    let dir = scratch(name);
    let file = dir.join(file_name);
    fs::write(&file, text).unwrap();
    let opts = Options { files: vec![file.to_string_lossy().into_owned()], ..Options::default() };
    let report = lint::run(&opts);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    report.findings.iter().map(|f| (f.category.to_string(), f.message.clone())).collect()
}

#[test]
fn a_combinational_signal_not_assigned_on_every_path_is_a_latch() {
    let found = categories_of("latch", "module l(input logic en, input logic d, output logic q);\n  always_comb begin\n    if (en) q = d;\n  end\nendmodule\n");
    assert!(found.iter().any(|(c, m)| c == "latch" && m.contains("'q' is not assigned on every path")), "{found:?}");
}

#[test]
fn a_default_value_removes_the_latch() {
    let found = categories_of("nolatch", "module l(input logic en, input logic d, output logic q);\n  always_comb begin\n    q = 0;\n    if (en) q = d;\n  end\nendmodule\n");
    assert!(!found.iter().any(|(c, _)| c == "latch"), "{found:?}");
}

#[test]
fn an_if_chain_on_every_value_of_a_one_bit_signal_is_not_a_latch() {
    // `tr` is 0 or 1: one of the two tests always holds, so `q` is assigned on every path
    let src = "module l(input logic tr, input logic d, output logic q);\n  always_comb begin\n    if (tr == 0) q = d;\n    else if (tr == 1) q = ~d;\n  end\nendmodule\n";
    let found = categories_of("chain1", src);
    assert!(!found.iter().any(|(c, _)| c == "latch"), "{found:?}");
}

#[test]
fn an_if_chain_missing_a_value_is_still_a_latch() {
    // `s` has four values and the chain tests two of them
    let src = "module l(input logic [1:0] s, input logic d, output logic q);\n  always_comb begin\n    if (s == 0) q = d;\n    else if (s == 1) q = ~d;\n  end\nendmodule\n";
    let found = categories_of("chain2", src);
    assert!(found.iter().any(|(c, m)| c == "latch" && m.contains("'q'")), "{found:?}");
}

#[test]
fn an_exhaustive_if_chain_with_a_branch_not_assigning_is_still_a_latch() {
    let src = "module l(input logic tr, input logic d, output logic q);\n  always_comb begin\n    if (tr == 0) q = d;\n    else if (tr == 1) begin end\n  end\nendmodule\n";
    let found = categories_of("chain3", src);
    assert!(found.iter().any(|(c, m)| c == "latch" && m.contains("'q'")), "{found:?}");
}

#[test]
fn two_ifs_in_sequence_that_test_every_value_of_a_one_bit_signal_are_not_a_latch() {
    let src = "module l(input logic tr, input logic d, output logic q);\n  always_comb begin\n    if (tr == 0) q = d;\n    if (tr == 1) q = ~d;\n  end\nendmodule\n";
    let found = categories_of("seq1", src);
    assert!(!found.iter().any(|(c, _)| c == "latch"), "{found:?}");
}

#[test]
fn two_ifs_in_sequence_that_miss_a_value_are_still_a_latch() {
    // `s` has four values: the two tests do not cover them
    let src = "module l(input logic [1:0] s, input logic d, output logic q);\n  always_comb begin\n    if (s == 0) q = d;\n    if (s == 1) q = ~d;\n  end\nendmodule\n";
    let found = categories_of("seq2", src);
    assert!(found.iter().any(|(c, m)| c == "latch" && m.contains("'q'")), "{found:?}");
}

#[test]
fn a_loop_that_assigns_every_element_is_not_a_latch() {
    // the loop runs at least once, and every element of `y` is assigned by it
    let src = "module l(input logic d, output logic [1:0] y [2]) {\n  always_comb {\n    for i in 0..2 { y[i] = d; }\n  }\n}\n";
    let found = categories_of("loopfull", src);
    assert!(!found.iter().any(|(c, _)| c == "latch"), "{found:?}");
}

#[test]
fn a_loop_over_rows_that_test_every_value_is_not_a_latch() {
    // each row tests the one-bit `tr` against its constant: the two rows together assign `y` on every path
    let src = "module R<int N = 3>(input logic tr, output logic [7:0] y [N]) {\n  always_comb {\n    for (k, lanes) in [[1, 0..N], [0, 0..N]] {\n      if (tr == k) { for i in lanes { y[i] = i; } }\n    }\n  }\n}\n";
    let found = categories_of_ov("rowsfull", src);
    assert!(!found.iter().any(|(c, _)| c == "latch"), "{found:?}");
}

#[test]
fn a_loop_over_rows_with_an_empty_row_is_still_a_latch() {
    // the row for `tr == 0` assigns nothing
    let src = "module R<int N = 3>(input logic tr, output logic [7:0] y [N]) {\n  always_comb {\n    for (k, lanes) in [[1, 0..N], [0, 0..0]] {\n      if (tr == k) { for i in lanes { y[i] = i; } }\n    }\n  }\n}\n";
    let found = categories_of_ov("rowsempty", src);
    assert!(found.iter().any(|(c, m)| c == "latch" && m.contains("'y'")), "{found:?}");
}

#[test]
fn a_signal_that_depends_on_itself_through_assignments_is_a_loop() {
    let found = categories_of("loop2", "module l(input logic a, output logic y);\n  logic b;\n  assign b = y ^ a;\n  assign y = b;\nendmodule\n");
    assert!(found.iter().any(|(c, m)| c == "loop" && m.contains("'b'") && m.contains("'y'")), "{found:?}");
}

#[test]
fn a_signal_that_reads_itself_in_an_assignment_is_a_loop() {
    let found = categories_of("loop1", "module l(input logic a, output logic y);\n  assign y = y & a;\nendmodule\n");
    assert!(found.iter().any(|(c, m)| c == "loop" && m.contains("'y'")), "{found:?}");
}

#[test]
fn fields_of_a_structure_are_different_signals() {
    // `s.to` reads `s.from`: no loop, although both are fields of `s`
    let found = categories_of("loopfields", "module l(input logic [3:0] x, output logic [3:0] y);\n  logic [3:0] p;\n  logic [3:0] q;\n  assign p = x;\n  assign q = p;\n  assign y = q;\nendmodule\n");
    assert!(!found.iter().any(|(c, _)| c == "loop"), "{found:?}");
}

#[test]
fn a_block_that_sets_a_signal_before_reading_it_is_not_a_loop() {
    let src = "module l(input logic a, output logic [3:0] y);\n  logic [3:0] t;\n  always_comb begin\n    t = 0;\n    if (a) t = t + 1;\n    y = t;\n  end\nendmodule\n";
    let found = categories_of("loopblock", src);
    assert!(!found.iter().any(|(c, _)| c == "loop"), "{found:?}");
}

#[test]
fn elements_of_an_array_are_not_one_signal() {
    // `st[1]` reads `st[0]`, a chain and not a loop; the elements are not followed by their index, so no loop is invented
    let src = "module l(input logic a, output logic y);\n  logic [1:0] st [2];\n  assign st[0] = a;\n  assign st[1] = st[0];\n  assign y = st[1];\nendmodule\n";
    let found = categories_of("loopark", src);
    assert!(!found.iter().any(|(c, _)| c == "loop"), "{found:?}");
}

#[test]
fn an_input_and_a_register_of_it_in_one_expression_are_reported_as_latency() {
    let found = categories_of_ov("latbad", "module L(input clk, input logic a, output logic y);
  logic r;
  always_ff @(posedge clk) r <= a;
  assign y = a & r;
endmodule
");
    assert!(found.iter().any(|(c, m)| c == "latency" && m.contains("'a' is 0 register(s) from 'a'")), "{found:?}");
}

#[test]
fn a_delayed_input_in_one_expression_is_not_reported_as_latency() {
    let found = categories_of_ov("latdelay", "module L(input clk, input logic a, output logic y);
  clock clk;
  logic r;
  always_ff @(posedge clk) r <= a;
  assign y = a.delay(1) & r;
endmodule
");
    assert!(!found.iter().any(|(c, _)| c == "latency"), "{found:?}");
}

#[test]
fn a_signal_on_a_loop_of_registers_has_no_latency_to_compare() {
    let found = categories_of_ov("latloop", "module L(input clk, input logic a, output logic y);
  logic r;
  always_ff @(posedge clk) r <= r + a;
  assign y = a & r;
endmodule
");
    assert!(!found.iter().any(|(c, _)| c == "latency"), "{found:?}");
}

#[test]
fn a_blocking_assignment_in_always_ff_is_reported() {
    let found = categories_of("ffblocking", "module b(input clk, input logic d, output logic q);
  logic r;
  always_ff @(posedge clk) begin
    r = d;
    q <= r;
  end
endmodule
");
    assert!(found.iter().any(|(c, m)| c == "process" && m.contains("blocking assignment (=) to 'r' inside always_ff")), "{found:?}");
}

#[test]
fn a_non_blocking_assignment_in_always_ff_is_not_reported() {
    let found = categories_of("ffnonblocking", "module b(input clk, input logic d, output logic q);
  logic r;
  always_ff @(posedge clk) begin
    r <= d;
    q <= r;
  end
endmodule
");
    assert!(!found.iter().any(|(c, _)| c == "process"), "{found:?}");
}

#[test]
fn two_assignments_drive_the_same_signal() {
    let found = categories_of("driver", "module d(input logic a, output logic y);\n  assign y = a;\n  assign y = ~a;\nendmodule\n");
    assert!(found.iter().any(|(c, m)| c == "driver" && m.contains("'y' is driven from several places (assign and assign)")), "{found:?}");
}

#[test]
fn an_output_that_nothing_drives_is_reported() {
    let found = categories_of("undriven", "module u(input logic a, output logic y, output logic z);\n  assign y = a;\nendmodule\n");
    assert!(found.iter().any(|(c, m)| c == "driver" && m.contains("output port 'z' of 'u' is never driven")), "{found:?}");
}

#[test]
fn an_input_without_default_that_is_not_connected_is_reported() {
    let found = categories_of("unconnected", "module c(input logic a, input logic b, output logic y);\n  assign y = a & b;\nendmodule\nmodule t(input logic x, output logic z);\n  c u (.a(x), .y(z));\nendmodule\n");
    assert!(found.iter().any(|(c, m)| c == "connection" && m.contains("input port 'b' of 'c' is not connected")), "{found:?}");
}

#[test]
fn assignment_style_in_the_wrong_block_is_reported() {
    let found = categories_of("style", "module s(input logic clk, input logic d, output logic p);\n  always_comb begin\n    p <= d;\n  end\nendmodule\n");
    assert!(found.iter().any(|(c, m)| c == "process" && m.contains("non-blocking assignment (<=) inside always_comb")), "{found:?}");
}

#[test]
fn an_error_in_one_module_does_not_hide_the_findings_of_the_others() {
    let dir = scratch("isolation");
    let good = dir.join("good.sv");
    let bad = dir.join("bad.sv");
    fs::write(&good, "module good(input logic en, input logic d, output logic q);\n  always_comb begin\n    if (en) q = d;\n  end\nendmodule\n").unwrap();
    fs::write(&bad, "module bad(input logic a, output logic y);\n  assign y = b;\nendmodule\n").unwrap();
    let opts = Options {
        files: vec![good.to_string_lossy().into_owned(), bad.to_string_lossy().into_owned()],
        ..Options::default()
    };
    let report = lint::run(&opts);
    assert!(report.findings.iter().any(|f| f.category == "latch"), "{:?}", report.findings);
    assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
    assert!(report.skipped[0].message.contains("unknown identifier 'b'"));
}

/// A plain SystemVerilog file linted on its own: the findings and what was skipped, as messages.
fn lint_sv_file(name: &str, src: &str) -> (Vec<String>, Vec<String>) {
    let dir = scratch(name);
    let file = dir.join(format!("{name}.sv"));
    fs::write(&file, src).unwrap();
    let opts = Options { files: vec![file.to_string_lossy().replace('\\', "/")], ..Options::default() };
    let report = lint::run(&opts);
    (report.findings.iter().map(|f| f.message.clone()).collect(), report.skipped.iter().map(|f| f.message.clone()).collect())
}

#[test]
fn a_break_in_a_sv_loop_is_analysed() {
    let (_, skipped) = lint_sv_file(
        "break_sv",
        "module P(input logic clk, input logic rst, input logic [3:0] req, output logic [1:0] who);
  // ovc: clock clk
  // ovc: reset rst
  always @(posedge clk) begin
    if (rst) who <= 0;
    else begin
      for (int j = 0; j < 4; j = j + 1) begin
        if (req[j]) begin
          who <= j;
          break;
        end
      end
    end
  end
endmodule
",
    );
    assert!(skipped.is_empty(), "{skipped:?}");
}

#[test]
fn a_localparam_in_a_generate_loop_is_analysed() {
    let (_, skipped) = lint_sv_file(
        "localparam_gen",
        "module L #(parameter T = 2) (input wire clk, input wire [7:0] d [T-1:0], output reg [7:0] q [2*T-1:0]);
  // ovc: clock clk
  genvar j;
  for (j = 0; j < T; j = j + 1) begin
    localparam idx = 1 * T + j;
    always @(posedge clk) begin
      q[idx] <= d[j];
    end
  end
endmodule
",
    );
    assert!(skipped.is_empty(), "{skipped:?}");
}
