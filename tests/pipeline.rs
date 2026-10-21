//! Pipeline lowering: registers, carried values, valid bits, control, the stage table and diagnostics.

const MAC: &str = "pipeline Mac(input clk, input rst, input logic valid_in,
  input logic [7:0] a, input logic [7:0] b, input logic [7:0] c,
  output logic [15:0] y, output logic valid_out) {
  stage mul { logic [15:0] p = a * b; }
  stage add { logic [15:0] s = (p + add.c).wrap(); }
  stage out { assign y = s; }
}";

#[test]
fn mac_lowers_to_registers_and_valid_chain() {
    let sv = ovc::compile(MAC, "mac.ov").unwrap();
    assert!(sv.contains("module Mac ("));
    assert!(sv.contains("assign ov_valid_mul = valid_in;"));
    assert!(sv.contains("ov_valid_add <= ov_valid_mul;"));
    assert!(sv.contains("ov_valid_out <= ov_valid_add;"));
    assert!(sv.contains("assign valid_out = ov_valid_out;"));
    assert!(sv.contains("c__s1 <= c;"));
    assert!(sv.contains("p__s1 <= p;"));
    assert!(sv.contains("s__s2 <= s;"));
    assert!(sv.contains("assign s = p__s1 + c__s1;"));
    assert!(!sv.contains("pipeline Mac"));
}

#[test]
fn mac_table_reports_latency_and_carries() {
    let table = ovc::compile_full(MAC, "mac.ov").unwrap().pipelines.remove(0);
    assert!(table.starts_with("pipeline Mac: 3 stages, latency 2 cycles (valid_in -> valid_out)"));
    assert!(table.contains("c: logic [7:0], p: logic [15:0]"));
    assert!(table.contains("out         y"));
}

#[test]
fn single_stage_has_no_registers() {
    let src = "pipeline One(input clk, input rst, input logic [3:0] d, output logic [3:0] y) {
      stage only { assign y = d + 1; }
    }";
    let out = ovc::compile_full(src, "one.ov").unwrap();
    assert!(out.sv.contains("assign ov_valid_only = 1'b1;"));
    assert!(out.pipelines[0].starts_with("pipeline One: 1 stage, latency 0 cycles"));
}

#[test]
fn reading_a_later_stage_points_at_the_name() {
    let src = "pipeline Bad(input clk, input rst, input logic [7:0] d, output logic [7:0] y) {
  stage s0 { logic [7:0] a = d; }
  stage s1 { logic [7:0] b = c + a; }
  stage s2 { logic [7:0] c = a; assign y = b; }
}";
    let e = ovc::compile(src, "bad.ov").unwrap_err();
    assert_eq!((e.line, e.col), (3, 30));
    assert!(e.message.contains("write 's2.c' for explicit forwarding"));
}

#[test]
fn stall_holds_earlier_stages_and_gates_ready() {
    let src = "pipeline Slow(input clk, input rst, input valid_in, input logic [7:0] d, input logic stall,
  output logic [7:0] y, output logic valid_out, output logic ready_out) {
  stage s0 { logic [7:0] a = (d + 1).wrap(); }
  stage s1 { logic [7:0] b = (a + 1).wrap(); stall_when(pipe.stall); }
  stage s2 { assign y = (b + 1).wrap(); }
}";
    let out = ovc::compile_full(src, "slow.ov").unwrap();
    assert!(out.sv.contains("assign ov_stall_s1 = ov_valid_s1 && stall;"));
    assert!(out.sv.contains("assign ov_hold_s0 = ov_hold_s1;"));
    assert!(out.sv.contains("assign ready_out = ~ov_hold_s0;"));
    assert!(out.sv.contains("ov_valid_s2 <= ov_valid_s1 & ~ov_stall_s1;"));
    assert!(out.warnings.is_empty());
}

#[test]
fn stall_without_ready_out_warns() {
    let src = "pipeline NoReady(input clk, input rst, input valid_in, input logic [7:0] d, input logic stall,
  output logic [7:0] y) {
  stage s0 { logic [7:0] a = d; stall_when(pipe.stall); }
  stage s1 { assign y = a; }
}";
    let out = ovc::compile_full(src, "noready.ov").unwrap();
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("no 'ready_out' port"));
}

#[test]
fn flush_kills_the_entering_instruction() {
    let src = "pipeline Fl(input clk, input rst, input valid_in, input logic [7:0] d, input logic kill,
  output logic [7:0] y, output logic valid_out) {
  stage s0 { logic [7:0] a = d; }
  stage s1 { logic [7:0] b = a; flush_when(pipe.kill); }
  stage s2 { assign y = b; }
}";
    let sv = ovc::compile(src, "fl.ov").unwrap();
    assert!(sv.contains("assign ov_kill_s2 = ov_flush_s1;"));
    assert!(sv.contains("if (ov_kill_s2) begin"));
}

#[test]
fn control_must_be_inside_a_stage() {
    let src = "pipeline Bad(input clk, input rst, input logic [7:0] d, input logic go, output logic [7:0] y) {
  stage s0 { logic [7:0] a = d; }
  stall_when(go);
  stage s1 { assign y = a; }
}";
    let e = ovc::compile(src, "bad.ov").unwrap_err();
    assert!(e.message.contains("'stall_when' must be inside a stage"));
}

#[test]
fn checks_are_emitted_unless_disabled() {
    let on = ovc::compile_with(MAC, "mac.ov", &ovc::Options { asserts: true, top: None }).unwrap().sv;
    assert!(on.contains("`ifndef SYNTHESIS"));
    assert!(on.contains("ov_a_advance_add: assert property (@(posedge clk) disable iff (rst) ov_valid_mul |=> ov_valid_add)"));
    assert!(on.contains("ov_a_valid_known: assert property"));
    let off = ovc::compile_with(MAC, "mac.ov", &ovc::Options { asserts: false, top: None }).unwrap().sv;
    assert!(!off.contains("ov_a_"));
    assert!(!off.contains("`ifndef"));
}

#[test]
fn pipeline_arrays_are_scalarized_and_chained() {
    let src = "pipeline Chain<int N = 2>(input clk, input rst, input logic valid_in, input logic [7:0] d,
  output logic [7:0] y, output logic valid_out) {
  logic [7:0] v [N+1];
  stage first { assign v[0] = d; }
  for i in 0..N {
    stage step { assign v[i+1] = (v[i] + 1).wrap(); }
  }
  stage last { assign y = v[N]; }
}";
    let sv = ovc::compile(src, "chain.ov").unwrap();
    assert!(sv.contains("assign v_0 = d;"));
    assert!(sv.contains("assign v_2 = v_1__s2 + 1;"));
    assert!(sv.contains("assign y = v_2__s3;"));
    assert_eq!(sv.matches("logic [7:0] v_0;").count(), 1);
}
