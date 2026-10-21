//! Top-level constants and enums: members become localparams in the modules that use them.

#[test]
fn enum_members_are_declared_in_each_user() {
    let src = "enum Op : logic [1:0] { ADD = 0, SUB = 1, AND = 3 }
module Alu(input Op op, input logic [7:0] a, input logic [7:0] b, output logic [7:0] y) {
  always_comb {
    case (op) {
      Op.ADD: y = (a + b).wrap();
      Op.AND: y = a & b;
      default: y = 0;
    }
  }
}";
    let sv = ovc::compile(src, "alu.ov").unwrap();
    assert!(sv.contains("  localparam logic [1:0] Op_ADD = 0;\n  localparam logic [1:0] Op_SUB = 1;\n  localparam logic [1:0] Op_AND = 3;\n"));
    assert!(sv.contains("  input logic [1:0] op,"));
    assert!(sv.contains("      Op_ADD: begin\n        y = a + b;"));
}

#[test]
fn enum_without_type_is_just_wide_enough() {
    let src = "enum State { IDLE, BUSY, DONE }
module M(input State s, output logic y) { assign y = s == State.DONE; }";
    let sv = ovc::compile(src, "m.ov").unwrap();
    assert!(sv.contains("localparam logic [1:0] State_DONE = 2;"));
    assert!(sv.contains("input logic [1:0] s,"));
}

#[test]
fn unused_enum_is_not_declared() {
    let src = "enum Unused { A, B }
module M(input logic x, output logic y) { assign y = x; }";
    let sv = ovc::compile(src, "m.ov").unwrap();
    assert!(!sv.contains("localparam"));
}

#[test]
fn top_level_constants_fold_into_widths() {
    let src = "const N = 4, W = 3;
module M(input logic [W-1:0] a, output logic [N-1:0] y) { assign y = a; }";
    let sv = ovc::compile(src, "c.ov").unwrap();
    assert!(sv.contains("input logic [2:0] a,"));
    assert!(sv.contains("output logic [3:0] y"));
    assert!(!sv.contains("const"));
}

#[test]
fn member_access_errors_point_at_the_name() {
    let src = "module M(input logic [7:0] a, output logic [7:0] y) { assign y = a.foo(1); }";
    let e = ovc::compile(src, "m.ov").unwrap_err();
    assert!(e.message.contains("method 'foo' is not supported yet"));
    assert_eq!((e.line, e.col), (1, 68));
}

#[test]
fn pure_functions_are_inlined() {
    let src = "module G(input logic [4:0] w, output logic [4:0] g, output logic f) {
  fn gray(b) { return b ^ (b >> 1); }
  fn wide(logic [3:0] x) -> logic [9:0] { return x + x; }
  assign g = gray(w);
  assign f = wide(w[3:0]) == 0;
}";
    let sv = ovc::compile(src, "g.ov").unwrap();
    assert!(sv.contains("assign g = w ^ w >> 1;"));
    assert!(sv.contains("assign f = 10'(w[3:0] + w[3:0]) == 0;"));
    assert!(!sv.contains("fn "));
}

#[test]
fn reg_resets_in_an_implicit_always_ff() {
    let src = "module Counter2<int W = 8>(input clk, input rst, input en, output logic [W-1:0] q) {
  reg [W-1:0] cnt = 0;
  always_ff { if (en) { cnt <= cnt + 1; } }
  assign q = cnt;
}";
    let sv = ovc::compile(src, "c2.ov").unwrap();
    assert!(sv.contains("  logic [7:0] cnt;\n  always_ff @(posedge clk) begin\n    if (rst) begin\n      cnt <= 0;\n    end else begin\n      if (en) begin\n        cnt <= cnt + 1;"));
}

#[test]
fn reset_directive_sets_polarity_and_async() {
    let src = "module Flag(input clk, input rstn, input set, output logic f) {
  reset rstn low async;
  reg r = 0;
  always_ff { if (set) { r <= 1; } }
  assign f = r;
}";
    let sv = ovc::compile(src, "flag.ov").unwrap();
    assert!(sv.contains("  always_ff @(posedge clk or negedge rstn) begin\n    if (!rstn) begin\n      r <= 0;"));
}

#[test]
fn list_methods_on_arrays_and_vectors() {
    let src = "module L(input logic [3:0] in [4], input logic [7:0] a, output logic [3:0] s, output logic [7:0] r, output logic [7:0] f) {
  assign s = in.list().tree_reduce(+);
  assign r = a.reverse();
  assign f = a.fill(2);
}";
    let sv = ovc::compile(src, "l.ov").unwrap();
    assert!(sv.contains("assign s = in[0] + in[1] + (in[2] + in[3]);"));
    assert!(sv.contains("assign r = {a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]};"));
    assert!(sv.contains("assign f = {2{a}};"));
}

#[test]
fn val_widths_are_lossless() {
    let src = "module Mac3(input logic [7:0] a, input logic [7:0] b, input logic [7:0] c, output logic [16:0] y) {
  val p = a * b;
  val s = p + c;
  assign y = s;
}";
    let sv = ovc::compile(src, "mac3.ov").unwrap();
    assert!(sv.contains("  logic [15:0] p;\n  logic [16:0] s;\n"));
    assert!(sv.contains("  assign p = a * b;\n  assign s = p + c;\n"));
}

#[test]
fn a_val_of_unknown_width_is_an_inline_alias() {
    // a shift by a variable has no lossless width: the val is its expression, with no signal of its own
    let src = "module M(input logic [7:0] a, input logic [2:0] b, output logic [7:0] y) { val t = (a << b).wrap(); assign y = t; }";
    let sv = ovc::compile(src, "m.ov").unwrap();
    assert!(sv.contains("assign y = a << b;"), "{sv}");
    assert!(!sv.contains("logic [7:0] t;"), "{sv}");
}

#[test]
fn an_unknown_identifier_is_an_error() {
    let src = "module M(input logic [7:0] a, output logic [7:0] y) { assign y = a + zz; }";
    let e = ovc::compile(src, "m.ov").unwrap_err();
    assert!(e.message.contains("unknown identifier 'zz'"), "{}", e.message);
}

#[test]
fn hierarchical_drive_needs_a_default() {
    let src = "module Leaf(input logic x, output logic y) { assign y = x; }
module Mid(input logic x, output logic y) { Leaf l(.x, .y); }
module Top(input logic a, output logic y) {
  Mid m(.x(a), .y);
  assign m.l.x = a;
}";
    let e = ovc::compile(src, "top.ov").unwrap_err();
    assert!(e.message.contains("can only be driven if it is an input port") || e.message.contains("needs a default value"), "{}", e.message);
}

#[test]
fn hierarchical_read_of_a_missing_signal_is_an_error() {
    let src = "module Leaf(input logic x, output logic y) { assign y = x; }
module Top(input logic a, output logic y) {
  Leaf l(.x(a), .y);
  assign y = l.nope;
}";
    let e = ovc::compile(src, "top.ov").unwrap_err();
    assert!(e.message.contains("has no signal 'nope'"), "{}", e.message);
}

#[test]
fn struct_methods_and_fields() {
    let src = "struct Pixel {
  logic [7:0] r;
  logic [7:0] g;
  logic [7:0] b;
  fn sum() -> logic [9:0] { return r + g + b; }
  fn brighter(logic [7:0] k) -> logic { return sum() > k; }
}
module Luma(input Pixel p, input logic [7:0] k, output logic [9:0] s, output logic bright) {
  assign s = p.sum();
  assign bright = p.brighter(k);
}";
    let sv = ovc::compile(src, "luma.ov").unwrap();
    assert!(sv.contains("  input pixel_pkg::pixel_t p,"));
    assert!(sv.contains("    logic [7:0] r;"));
    assert!(sv.contains("  assign s = 10'(p.r + p.g + p.b);"));
    assert!(sv.contains("  assign bright = 10'(p.r + p.g + p.b) > k;"));
}
