//! The standard library: available without an import, emitted only where it is used.

const TOP: &str = "module Top(input clk, input rst, slave Stream<logic [7:0]> inp, master Stream<logic [7:0]> outp, output logic [2:0] count) {
  clock clk;
  reset rst;

  Fifo<logic [7:0], 4> f(.clk, .rst, .in(inp), .out(outp), .count);
}";

#[test]
fn fifo_is_specialised_and_only_used_library_modules_are_emitted() {
    let sv = ovc::compile(TOP, "top.ov").unwrap();
    assert!(sv.contains("module Fifo__logic_7_0_4 ("));
    assert!(sv.contains("  logic [1:0] rd;"), "{sv}");
    assert!(!sv.contains("module Slice"));
    assert!(!sv.contains("module Arbiter"));
}

#[test]
fn a_user_definition_takes_precedence() {
    let src = TOP.to_string() + "\nmodule Fifo<type T = logic [7:0], int DEPTH = 4>(input clk, input rst) { }";
    let sv = ovc::compile(&src, "top.ov").unwrap();
    // the user's (empty) Fifo is specialised: the library's ports are absent
    assert!(sv.contains("module Fifo__logic_7_0_4 (\n  input logic clk,\n  input logic rst\n);"), "{sv}");
    assert!(!sv.contains("in_valid"));
}

#[test]
fn an_unused_library_module_is_not_emitted() {
    let sv = ovc::compile("module M(input logic a, output logic y) { assign y = a; }", "m.ov").unwrap();
    assert!(!sv.contains("module Fifo"));
    assert!(!sv.contains("module Slice"));
}
