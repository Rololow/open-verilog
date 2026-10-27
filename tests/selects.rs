//! Part-selects of values that are not named signals, and definitions repeated across files (in .ov and in lint).
//! Ported from the Python `tests/test_selects.py`; the simulation runs only when Questa is on the PATH.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use ovc::lint::{self, Options};

const SEL: &str = "
module M(input logic [7:0] a, input logic [7:0] b, input logic [2:0] i, output logic [4:0] hi,
         output logic x, output logic [3:0] k, output logic [1:0] p) {
  assign hi = (a + b)[8:4];
  assign x = (a & b)[i];
  assign k = (8'hA5)[7:4];
  assign p = (a ^ b)[i +: 2];
}
";

fn compiled(src: &str) -> ovc::Compiled {
    ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message))
}

/// Writes the files in `dir` and lints them: each finding as (category, message, line).
fn lint(dir: &Path, files: &[(&str, &str)]) -> Vec<(String, String, usize)> {
    let paths: Vec<String> = files
        .iter()
        .map(|(name, text)| {
            let path = dir.join(name);
            fs::write(&path, text).unwrap();
            path.to_string_lossy().replace('\\', "/")
        })
        .collect();
    let report = lint::run(&Options { files: paths, ..Options::default() });
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    report.findings.iter().map(|f| (f.category.to_string(), f.message.clone(), f.line)).collect()
}

fn scratch(name: &str) -> PathBuf {
    common::scratch(name)
}

#[test]
fn expression_selects_are_lowered_with_the_full_width() {
    let c = compiled(SEL);
    let sv = &c.sv;
    // the carry of a + b is kept
    assert!(sv.contains("assign hi = 5'(9'(a + b) >> 4);"), "{sv}");
    assert!(sv.contains("assign x = 1'(8'(a & b) >> i);"), "{sv}");
    assert!(sv.contains("assign k = 4'ha;"), "{sv}");
    assert!(sv.contains("assign p = 2'(8'(a ^ b) >> i);"), "{sv}");
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
}

#[test]
fn expression_select_out_of_range() {
    let src = SEL.replace("(a + b)[8:4]", "(a + b)[9:4]").replace("logic [4:0] hi", "logic [5:0] hi");
    let err = ovc::compile(&src, "t.ov").expect_err("out of range");
    assert!(err.message.contains("bits [9:4] are out of range for this 9-bit value"), "{}", err.message);
}

#[test]
#[ignore = "not implemented: lint has no [syntax] check for a select of an expression in SV source"]
fn sv_lint_reports_expression_selects_but_not_parameters() {
    let dir = scratch("selects-lint-params");
    let found = lint(&dir, &[(
        "m.sv",
        "
module m(input logic [7:0] a, input logic [7:0] b, output logic [3:0] q, output logic [3:0] r);
  localparam logic [7:0] P = 8'h5a;
  assign q = (a & b)[3:0];
  assign r = P[7:4];
endmodule
",
    )]);
    let syntax: Vec<_> = found.iter().filter(|(c, _, _)| c == "syntax").collect();
    assert_eq!(syntax.len(), 1, "{found:?}");
    assert_eq!(syntax[0].2, 4, "the finding is on the assign of q: {found:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn definitions_repeated_in_one_file_are_errors() {
    let err = ovc::compile(
        "\nfn f(x) { return x; }\nfn f(x) { return x + 1; }\nmodule M(output logic [3:0] q) { assign q = f(1); }\n",
        "t.ov",
    )
    .expect_err("a repeated function");
    assert!(err.message.contains("'f' is already defined at t.ov:2"), "{}", err.message);
}

#[test]
#[ignore = "not implemented: lint has no [duplicate] check for types defined in two files"]
fn definitions_repeated_across_files_are_lint_findings() {
    let dir = scratch("selects-duplicates");
    let pkg = "package p; localparam int W = 4; endpackage\n";
    let found = lint(
        &dir,
        &[
            ("p.sv", pkg),
            ("a.sv", "import p::*;\nmodule a(output logic [W-1:0] q); assign q = 0; endmodule\n"),
            ("b.sv", "import p::*;\nmodule b(output logic [W-1:0] q); assign q = 1; endmodule\n"),
            ("c.sv", "typedef logic [3:0] nib_t;\nmodule c(output nib_t q); assign q = 0; endmodule\n"),
            ("d.sv", "typedef logic [7:0] nib_t;\nmodule d(output nib_t q); assign q = 0; endmodule\n"),
        ],
    );
    assert!(found.iter().all(|(_, m, _)| !m.contains("not analyzed")), "{found:?}");
    let dup: Vec<_> = found.iter().filter(|(c, _, _)| c == "duplicate").collect();
    assert_eq!(dup.len(), 1, "{found:?}");
    assert!(dup[0].1.contains("'nib_t' is already defined at"), "{found:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lowered_selects_simulate() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    let src = format!(
        "{SEL}
test \"selects\" for M {{
  a = 200; b = 100; i = 2;
  expect(hi == 18, \"300 >> 4: the carry is kept\");
  expect(x == 0 && p == 3);         // a ^ b = 0xac
  a = 'hff; b = 'h0c;
  expect(x == 1 && p == 0 && k == 'ha);   // a ^ b = 0xf3
}}
"
    );
    let c = ovc::compile_full(&src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message));
    let dir = scratch("selects-sim");
    fs::write(dir.join("dut.sv"), &c.sv).unwrap();
    fs::write(dir.join("tests.sv"), c.tests_sv.clone().expect("a testbench")).unwrap();
    let out = common::simulate(&dir, &["-sv", "dut.sv", "tests.sv"], "ovc_tests");
    assert!(out.contains("ovc tests: 1 passed, 0 failed"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn wrap_of_a_sum_with_a_signed_operand_keeps_its_carries() {
    // the cast goes to the signed leaf: a cast around the whole sum would size the sum by its own operands and lose
    // its carries (reported from the mem1 V3 colend)
    let c = compiled(
        "
module M(input logic [7:0] log, input logic [7:0] mx, input logic [7:0] off, input logic [24:0] sc, output logic [24:0] y) {
  wire signed [24:0] split = (log + mx + $signed(off) - sc).wrap();
  assign y = split;
}",
    );
    assert!(c.sv.contains("25'($signed(off))"), "{}", c.sv);
    assert!(!c.sv.contains("25'(log + mx"), "{}", c.sv);
}

#[test]
fn wrapped_parts_of_cat_are_cast() {
    let c = compiled(
        "
module M(input logic [5:0] lz, input logic s, output logic [8:0] y) {
  assign y = {s, (158 - lz.zext(8)).wrap()};
}",
    );
    // SV would size it on 32 bits and push the sign out
    assert!(c.sv.contains("8'(158 - {{2{1'b0}}, lz})"), "{}", c.sv);
}

#[test]
fn products_are_shifted_on_all_their_bits() {
    let c = compiled(
        "
module M(input logic [7:0] a, input logic [7:0] b, output logic [7:0] y) {
  assign y = ((a * b) >> 8).wrap();
}",
    );
    assert!(c.sv.contains("16'(a * b) >> 8"), "{}", c.sv);
}

#[test]
#[ignore = "not implemented: lint has no [sizing] category yet (the width finding is reported instead)"]
fn sv_products_truncated_before_a_shift_are_reported() {
    let dir = scratch("selects-sizing");
    let found = lint(
        &dir,
        &[(
            "m.sv",
            "
module m(input logic [7:0] a, input logic [7:0] b, output logic [7:0] y, output logic [31:0] z);
  int i, j;
  assign y = (a * b) >> 8;
  assign z = (i * j) >> 4;
endmodule
",
        )],
    );
    let sizing: Vec<_> = found.iter().filter(|(c, _, _)| c == "sizing").collect();
    assert_eq!(sizing.len(), 1, "{found:?}");
    assert!(sizing[0].1.contains("'a * b' is evaluated on 8 bits before '>> 8'"), "{found:?}");
    let _ = fs::remove_dir_all(&dir);
}
