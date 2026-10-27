//! Connections across the hierarchy: `a.b.sig` reads and `assign a.b.port = x` drives, through generated ports.
//! Ported from the Python `tests/test_hier.py`; the simulation runs only when Questa is on the PATH.

mod common;

use ovc::Options;

const LIB: &str = "
module Leaf(input clk, input logic [7:0] d, input stall = 0, output logic [7:0] q) {
  clock clk;

  logic [7:0] secret;
  logic [7:0] mem [4];
  always_ff @(posedge clk) { if (!stall) { secret <= d; q <= secret; mem[d[1:0]] <= d; } }
}
module Mid(input clk, input logic [7:0] d, output logic [7:0] q) { Leaf leaf(.clk, .d, .q); }
";

fn compile(src: &str, top: Option<&str>) -> Result<ovc::Compiled, ovc::error::Error> {
    let options = Options { asserts: true, top: top.map(String::from), cover: false };
    ovc::compile_with(src, "t.ov", &options)
}

fn sv_of(src: &str, top: Option<&str>) -> String {
    let c = compile(&format!("{LIB}{src}"), top).unwrap_or_else(|e| panic!("{}", e.message));
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    c.sv
}

fn error_of(src: &str) -> String {
    match compile(&format!("{LIB}{src}"), None) {
        Ok(_) => panic!("the design must be refused:\n{src}"),
        Err(e) => e.message,
    }
}

#[test]
fn read_deep_signal_threads_output_ports() {
    let sv = sv_of(
        "
module Top(input clk, input logic [7:0] d, output logic [7:0] q, output logic [7:0] probe) {
  clock clk;

  Mid mid(.clk, .d, .q);
  assign probe = mid.leaf.secret;
}",
        None,
    );
    assert!(sv.contains("output logic [7:0] ov_tap_secret") && sv.contains("assign ov_tap_secret = secret;"), "{sv}");
    assert!(sv.contains("output logic [7:0] ov_tap_leaf_secret") && sv.contains(".ov_tap_secret(ov_tap_leaf_secret)"), "{sv}");
    assert!(sv.contains(".ov_tap_leaf_secret(ov_tap_mid_leaf_secret)") && sv.contains("assign probe = ov_tap_mid_leaf_secret;"), "{sv}");
}

#[test]
fn same_tap_twice_and_from_a_process_is_threaded_once() {
    let sv = sv_of(
        "
module Top(input clk, input logic [7:0] d, output logic [7:0] q, output logic [7:0] a, output logic [7:0] b) {
  clock clk;

  Mid mid(.clk, .d, .q);
  assign a = mid.leaf.secret;
  always_ff @(posedge clk) { b <= mid.leaf.secret + 1; }
}",
        None,
    );
    assert_eq!(sv.matches("output logic [7:0] ov_tap_leaf_secret").count(), 1, "{sv}");
    assert_eq!(sv.matches("logic [7:0] ov_tap_mid_leaf_secret;").count(), 1, "{sv}");
}

#[test]
fn memory_element_can_be_read() {
    let sv = sv_of(
        "
module Top(input clk, input logic [7:0] d, output logic [7:0] q, output logic [7:0] m1) {
  Mid mid(.clk, .d, .q);
  assign m1 = mid.leaf.mem[1];
}",
        None,
    );
    assert!(sv.contains("output logic [7:0] ov_tap_mem [4]") && sv.contains("assign m1 = ov_tap_mid_leaf_mem[1];"), "{sv}");
}

#[test]
fn drive_deep_input_with_default() {
    let sv = sv_of(
        "
module Top(input clk, input logic [7:0] d, input halt, output logic [7:0] q) {
  Mid mid(.clk, .d, .q);
  Mid spare(.clk, .d, .q());
  assign mid.leaf.stall = halt;
}",
        None,
    );
    assert!(sv.contains("input logic stall = 0") && sv.contains("input logic ov_drv_leaf_stall = 0"), "{sv}");
    assert!(sv.contains(".stall(ov_drv_leaf_stall)") && sv.contains(".ov_drv_leaf_stall(halt)"), "{sv}");
    assert!(sv.contains("Mid spare (.clk(clk), .d(d), .q());"), "{sv}");
}

#[test]
fn errors_name_the_mistake() {
    let cases = [
        ("assign q = mid.lef.secret;", "module 'Mid' has no instance 'lef' (instances: leaf)"),
        ("assign q = mid.leaf.secrt;", "module 'Leaf' has no signal 'secrt'"),
        ("assign q = mid.leaf;", "'leaf' is an instance of 'Mid', not a signal"),
        ("assign mid.leaf.secret = d;", "can only be driven if it is an input port of 'Leaf'"),
        ("assign mid.leaf.d = d;", "input 'd' of 'Leaf' needs a default value"),
        ("always_ff @(posedge clk) { mid.leaf.stall <= 1; }", "is driven inside a process"),
    ];
    for (body, text) in cases {
        let msg = error_of(&format!(
            "
module Top(input clk, input logic [7:0] d, output logic [7:0] q) {{
  Mid mid(.clk, .d, .q());
  {body}
}}"
        ));
        assert!(msg.contains(text), "'{msg}' does not contain '{text}'");
    }
}

#[test]
fn direct_child_input_already_connected() {
    let msg = error_of(
        "
module Top(input clk, input logic [7:0] d, output logic [7:0] q) {
  Leaf l(.clk, .d, .stall(1'b0), .q);
  assign l.stall = 1;
}",
    );
    assert!(msg.contains("port 'stall' of instance 'l' in 'Top' is already connected"), "{msg}");
}

#[test]
fn tap_across_clock_domains_is_still_checked() {
    let msg = error_of(
        "
module Top(input clk_a, input clk_b, input logic [7:0] d, output logic [7:0] q, output logic [7:0] p) {
  Mid mid(.clk(clk_a), .d, .q);
  always_ff @(posedge clk_b) { p <= mid.leaf.secret; }
}",
    );
    assert!(msg.contains("clock-domain crossing") && msg.contains("'clk_a'"), "{msg}");
}

#[test]
fn simulates() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    let sv = sv_of(
        "
module Top(input clk, input logic [7:0] d, input halt, output logic [7:0] q, output logic [7:0] probe) {
  clock clk;

  Mid mid(.clk, .d, .q);
  assign probe = mid.leaf.secret;
  assign mid.leaf.stall = halt;
}",
        Some("Top"),
    );
    let dir = common::scratch("hier");
    std::fs::write(dir.join("dut.sv"), sv).unwrap();
    std::fs::write(dir.join("tb.sv"), TB).unwrap();
    let out = common::simulate(&dir, &["-sv", "dut.sv", "tb.sv"], "tb");
    assert!(out.contains("PASS"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

const TB: &str = "module tb;
  logic clk = 0, halt = 0; logic [7:0] d = 0, q, probe;
  Top dut (.*);
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) d = 42; @(negedge clk) if (probe !== 42) $fatal(1, \"FAIL probe %0d\", probe);
    halt = 1; d = 7; @(negedge clk) if (probe !== 42) $fatal(1, \"FAIL stall %0d\", probe);
    halt = 0; @(negedge clk) if (probe !== 7) $fatal(1, \"FAIL resume %0d\", probe);
    $display(\"PASS\"); $finish;
  end
endmodule
";

#[test]
fn loop_instances_are_reachable_by_index() {
    let sv = sv_of(
        "
module Top(input clk, input logic [7:0] d, input halt, output logic [7:0] q [2], output logic [7:0] p) {
  for i in 0..2 { Mid m(.clk, .d, .q(q[i])); }
  for i in 0..2 { for j in 0..2 { Leaf g(.clk, .d, .q()); } }
  assign p = m[1].leaf.secret ^ g[1][0].secret;
  assign m[0].leaf.stall = halt;
}",
        None,
    );
    assert!(sv.contains("assign p = ov_tap_m_1_leaf_secret ^ ov_tap_g_1_0_secret;"), "{sv}");
    assert!(sv.contains(".ov_drv_leaf_stall(halt)") && sv.contains("Mid m_1 (.clk(clk), .d(d), .q(q[1]), .ov_tap_leaf_secret"), "{sv}");
}

#[test]
fn tapping_a_connected_output_reuses_its_connection() {
    let sv = sv_of(
        "
module Top(input clk, input logic [7:0] d, output logic [7:0] p) {
  logic [7:0] qs [2];
  for i in 0..2 { Leaf l(.clk, .d, .q(qs[i])); }
  assign p = l[1].q;
}",
        None,
    );
    assert!(sv.contains("assign p = qs[1];") && !sv.contains("ov_tap"), "{sv}");
}
