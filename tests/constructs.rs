//! Constructs that remove duplicated logic: match, in, select, for-in tables, struct views, .* connections (ported from
//! the Python `tests/test_features.py`).

fn sv_of(src: &str) -> String {
    let out = ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.render(src)));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    out.sv
}

#[test]
fn a_match_expression_is_a_chain_of_conditionals() {
    let sv = sv_of("module M(input logic [2:0] f, input logic [7:0] a, input logic [7:0] b, output logic [7:0] y) {
  assign y = match (f) { 7: a & b; 0, 1: a | b; 2..5: a ^ b; default: 0 };
}");
    assert!(sv.contains("f == 7 ? a & b : f == 0 || f == 1 ? a | b : f >= 2 && f < 5 ? a ^ b : 0"), "{sv}");
}

#[test]
fn a_match_on_a_constant_keeps_only_one_arm() {
    let sv = sv_of("config K { mode = 2; }
module M<config C = K>(input logic [7:0] a, output logic [7:0] y) {
  assign y = match (C.mode) { 1: a; 2: ~a; default: 0 };
}");
    assert!(sv.contains("assign y = ~a;"), "{sv}");
}

#[test]
fn an_in_set_tests_values_and_ranges() {
    let sv = sv_of("module M(input logic [6:0] op, output logic y) { assign y = op in {7'h33, 7'h13, 0..4}; }");
    assert!(sv.contains("op == 7'h33 || op == 7'h13 || op >= 0 && op < 4"), "{sv}");
}

#[test]
fn a_table_select_and_a_for_in_over_a_table() {
    let sv = sv_of("module M(input clk, input logic [1:0] addr, input logic [7:0] wd, input we, output logic [7:0] rd) {
  clock clk;

  logic [7:0] A = 0, B = 0;
  val regs = [[0, A], [1, B]];
  always_ff @(posedge clk) { if (we) { for (k, r) in regs { if (addr == k) { r <= wd; } } } }
  assign rd = regs.select(addr, 8'hff);
}");
    assert!(sv.contains("if (addr == 0) begin") && sv.contains("B <= wd;"), "{sv}");
    assert!(sv.contains("assign rd = addr == 0 ? A : addr == 1 ? B : 8'hff;"), "{sv}");
}

#[test]
fn a_struct_view_names_the_fields_of_a_word() {
    let sv = sv_of("struct RvI { logic [6:0] f7; logic [4:0] rs2; logic [4:0] rs1; logic [2:0] f3; logic [4:0] rd; logic [6:0] op; }
module M(input logic [31:0] instr, output logic [3:0] rd, output logic sign) {
  val i = instr.as(RvI);
  assign rd = i.rd[3:0];
  assign sign = i.f7[6];
}");
    // the view keeps its signal `i`, typed by the packed struct (the Python version inlined the bits): its fields are
    // slices of the bits of that signal
    assert!(sv.contains("assign i = instr;"), "{sv}");
    assert!(sv.contains("assign rd = i[10:7];") && sv.contains("assign sign = i[31];"), "{sv}");
}

#[test]
fn a_star_connection_connects_the_ports_by_name() {
    let sv = sv_of("module R(input clk, input logic [7:0] d, output logic [7:0] q) {
  clock clk;
 always_ff @(posedge clk) { q <= d; } }
module T(input clk, input logic [7:0] d, output logic [7:0] q) { R r(.*); }");
    assert!(sv.contains("R r (.clk(clk), .d(d), .q(q));"), "{sv}");
}
