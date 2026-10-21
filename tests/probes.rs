//! Debug probes: a module per probe and a top module that binds it into every instance of its target.

const SRC: &str = "module Pe(input clk, input rst, input logic [1:0] mode, input logic [7:0] din, output logic [7:0] acc) {
  always_ff @(posedge clk) { if (rst) { acc <= 0; } else if (mode == 2) { acc <= (acc + din).wrap(); } }
}
module Top(input clk, input rst, input logic [1:0] mode, input logic [7:0] din, output logic [7:0] a0) {
  Pe p0(.clk, .rst, .mode, .din, .acc(a0));
}
probe pe_acc on Pe when (mode == 2) limit 3 {
  print(\"din=%0d acc=%0d\", din, acc);
}";

#[test]
fn probe_module_and_bind() {
    let p = ovc::compile_full(SRC, "p.ov").unwrap().probes_sv.expect("the source has a probe");
    assert!(p.contains("module ovc_probe_0 (input logic [7:0] acc, input logic clk, input logic [7:0] din, input logic [1:0] mode);"));
    assert!(p.contains("if (on && (mode == 2) && n < 3) begin"));
    assert!(p.contains("$display(\"[pe_acc] %m t=%0t din=%0d acc=%0d\", $time, din, acc);"));
    assert!(p.contains("bind Pe ovc_probe_0 ovc_probe_0_i (.*);"));
}

#[test]
fn no_probe_no_file() {
    let src = "module M(input clk, input logic a, output logic y) { assign y = a; }";
    assert!(ovc::compile_full(src, "m.ov").unwrap().probes_sv.is_none());
}

#[test]
fn probe_needs_a_clock() {
    let src = "module M(input logic a, output logic y) { assign y = a; }
probe p on M when (a) { print(\"a=%0d\", a); }";
    let e = ovc::compile_full(src, "m.ov").unwrap_err();
    assert!(e.message.contains("has no clock input"), "{}", e.message);
}
