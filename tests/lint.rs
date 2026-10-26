//! `ovc lint`: the preprocessor, the categories of the findings, and the findings of a run.

use std::fs;
use std::path::PathBuf;

use ovc::lint::{self, Options};
use ovc::preproc::Preprocessor;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ovc_lint_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
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
    assert!(err.message.contains("macro `NOPE is not defined"));
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
    let dir = scratch(name);
    let file = dir.join("d.sv");
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
