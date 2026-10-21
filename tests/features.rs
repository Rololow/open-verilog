//! Language features of the SoC: register maps, memory maps, vectors and nesting of bundles, procedural functions,
//! tasks, manifests, and the test commands that drive bundles.

fn sv(src: &str) -> String {
    ovc::compile(src, "f.ov").unwrap_or_else(|e| panic!("{}", e.render(src)))
}

#[test]
fn a_regmap_is_a_module_with_a_port_per_field() {
    let out = sv("regmap Uart \"UART\" {
  CTRL @ 'h00 \"Control\" { en : rw [0] = 0 \"Enable\"; baud : rw [15:4] = 434 \"Divider\"; }
  STATUS @ 'h04 { busy : ro [0]; }
  IRQ @ 'h08 strobe { done : w1c [0]; }
  ID @ 'h0c { id : const [31:0] = 'h4f564e50; }
}");
    assert!(out.contains("module Uart ("));
    assert!(out.contains("output logic [11:0] ctrl_baud"));
    assert!(out.contains("input logic status_busy"));
    assert!(out.contains("input logic irq_done_set"));
    assert!(out.contains("output logic irq_written"));
    assert!(out.contains("rdata[31:0] <= 32'h4f564e50;"));
}

#[test]
fn regmap_checks_overlapping_fields() {
    let e = ovc::compile("regmap R { A @ 0 { x : rw [3:0]; y : rw [2:1]; } }", "r.ov").unwrap_err();
    assert!(e.message.contains("overlap"), "{}", e.message);
}

#[test]
fn memmap_gives_indices_bases_and_the_interrupt_vector() {
    let out = sv("module M(input logic u_irq, output logic [31:0] base, output logic [1:0] idx, output logic [31:0] irqs) {
  memmap io \"devices\" {
    A @ 'h1000 size 'h100 \"a\";
    B @ 'h2000 size 'h100 irq 3 = u_irq;
  }
  assign base = io.B.base;
  assign idx = io.B;
  assign irqs = io.irqs;
}");
    assert!(out.contains("assign base = 8192;"), "{out}");
    assert!(out.contains("assign idx = 1;"), "{out}");
    assert!(out.contains("u_irq"));
}

#[test]
fn memmap_rejects_overlapping_devices() {
    let e = ovc::compile("module M(output logic y) { memmap io { A @ 0 size 16; B @ 8 size 16; } assign y = 0; }", "m.ov").unwrap_err();
    assert!(e.message.contains("overlap"), "{}", e.message);
}

#[test]
fn a_vector_of_bundles_is_one_bundle_per_element() {
    let out = sv("module M<int N = 2>(input clk, input rst, slave Vec<Stream<logic [7:0]>, N> in, master Stream<logic [7:0]> out) {
  assign out.valid = in.valid[1];
  assign out.data = in.data[1];
  for i in 0..N { assign in[i].ready = out.ready; }
}");
    // each field of the vector is one signal: its bits are the elements' (valid), or an array (data)
    assert!(out.contains("input logic [1:0] in_valid"), "{out}");
    assert!(out.contains("assign out_valid = in_valid[1];"), "{out}");
    assert!(out.contains("assign in_ready[0] = out_ready;"), "{out}");
}

#[test]
fn bundles_nest_and_flip_with_the_role() {
    let out = sv("bundle Pair { output Stream<logic [3:0]> a; input Stream<logic [3:0]> b; }
module M(input clk, input rst, slave Pair p) { assign p.b.valid = p.a.valid; assign p.a.ready = 1; assign p.b.data = p.a.data; }");
    assert!(out.contains("input logic p_a_valid"));
    assert!(out.contains("output logic p_a_ready"));
    assert!(out.contains("output logic p_b_valid"));
    assert!(out.contains("input logic p_b_ready"));
}

#[test]
fn a_procedural_function_is_expanded_at_its_calls() {
    let out = sv("module M(input clk, input rst, input logic go, output logic [3:0] q) {
  reg [3:0] a, b;
  fn load(x, y) { a <= x; b <= y; }
  always_ff { if (go) { load(1, 2); } }
  assign q = a + b;
}");
    assert!(out.contains("a <= 1;"), "{out}");
    assert!(out.contains("b <= 2;"));
}

#[test]
fn module_enums_are_local_to_their_module() {
    let out = sv("module A(input clk, input rst, output logic y) { enum St { X, Y } reg [0:0] s; always_ff { s <= St.Y; } assign y = s; }
module B(input clk, input rst, output logic y) { enum St { P, Q } reg [0:0] s; always_ff { s <= St.Q; } assign y = s; }");
    assert!(out.contains("St_Y"));
    assert!(out.contains("St_Q"));
}

#[test]
fn constant_tables_are_read_by_index_and_by_signal() {
    let out = sv("val T = [10, 20, 30];
module M(input logic [1:0] i, output logic [7:0] a, output logic [7:0] b) { assign a = T[1]; assign b = T[i]; }");
    assert!(out.contains("assign a = 20;"), "{out}");
    assert!(out.contains("localparam logic [4:0] T [3] = '{10, 20, 30};"), "{out}");
    assert!(out.contains("assign b = T[i];"), "{out}");
}

#[test]
fn a_part_of_an_expression_is_shifted_and_cast() {
    let out = sv("module M(input logic [7:0] a, input logic [7:0] b, output logic y) { assign y = (a & b)[3]; }");
    assert!(out.contains("1'(8'(a & b) >> 3)"), "{out}");
}

#[test]
fn loop_locals_are_named_per_iteration() {
    let out = sv("module M(input logic [3:0] a, output logic [3:0] y [2]) { for i in 0..2 { wire [3:0] t = a + i; assign y[i] = t; } }");
    assert!(out.contains("t_0"), "{out}");
    assert!(out.contains("t_1"));
}

#[test]
fn tie_off_drives_the_idle_values() {
    let out = sv("module M(input clk, input rst) { Stream<logic [7:0]> s; tie_off(s); }");
    assert!(out.contains("assign s_valid = 0;"), "{out}");
    assert!(out.contains("assign s_data = 0;"), "{out}");
}

#[test]
fn streams_are_driven_by_send_and_received_by_recv() {
    let src = "fn put(d) { send(in, d); }
module Pass(input clk, input rst, slave Stream<logic [7:0]> in, master Stream<logic [7:0]> out) {
  assign out.valid = in.valid;
  assign out.data = in.data;
  assign in.ready = out.ready;
}
test \"pass\" for Pass {
  put(5);
  recv(out, out.data == 5);
}";
    let tb = ovc::compile_full(src, "t.ov").unwrap().tests_sv.expect("tests");
    assert!(tb.contains("in_data = 5;"), "{tb}");
    assert!(tb.contains("in_valid = 1;"));
    assert!(tb.contains("out_ready = 1;"));
}

#[test]
fn a_manifest_gives_the_rows_of_a_test_loop() {
    let dir = std::env::temp_dir().join("ovc_features_manifest");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("m.toml"), "[[rows]]\nname = \"one\"\nvalue = 0x10\n\n[[rows]]\nname = \"two\"\nvalue = 2\n").unwrap();
    let file = dir.join("t.ov");
    let src = "config Manifest from \"m.toml\" { }
module D(input logic [7:0] a, output logic [7:0] y) { assign y = a; }
for r in Manifest.rows.filter(t => t.value > 5) {
  test r.name for D { a = r.value; step; expect(y == r.value); }
}";
    let path = file.to_string_lossy().replace('\\', "/");
    let tb = ovc::compile_full(src, &path).unwrap().tests_sv.expect("tests");
    assert_eq!(tb.matches("module ovc_test_").count(), 1, "{tb}");
    assert!(tb.contains("test \"one\""));
    assert!(tb.contains("a = 16;"));
}

#[test]
fn a_view_through_a_parameterized_struct_slices_the_word() {
    let out = sv("config Cfg { W = 24; }
struct SegWord<int W = 24> { logic [W/2-2:0] lo; logic mid; logic [W/2-2:0] hi; logic sgn; }
module M<config C : Cfg = Cfg>(input logic [23:0] a, output logic [10:0] lo, output logic [10:0] hi, output logic sgn) {
  val av = a.as(SegWord<C.W>);
  assign lo = av.lo;
  assign hi = av.hi;
  assign sgn = av.sgn;
}");
    assert!(out.contains("assign lo = av[23:13];"), "{out}");
    assert!(out.contains("assign hi = av[11:1];"), "{out}");
    assert!(out.contains("assign sgn = av[0:0];"), "{out}");
}

#[test]
fn a_tree_reduction_by_a_function_names_its_levels() {
    let inputs: String = (0..4).map(|k| format!("input logic [7:0] x{k}, ")).collect();
    let src = format!("fn smax(a, b) {{ return a > b ? a : b; }}
module T({inputs}output logic [7:0] y) {{
  val xs = [x0, x1, x2, x3];
  assign y = xs.tree_reduce(smax);
}}");
    let out = sv(&src);
    assert!(out.contains("assign ov_tree1 = x0 > x1 ? x0 : x1;"), "{out}");
    assert!(out.contains("assign y = ov_tree1 > ov_tree2 ? ov_tree1 : ov_tree2;"), "{out}");
}

#[test]
fn an_exported_struct_has_its_memory_layout_in_the_c_header() {
    let src = "export struct DmaDesc \"DMA descriptor\" { logic [31:0] src \"source address\"; logic [31:0] len; }
module M(input logic clk) {}";
    let file = ovc::parser::parse_file(src, "d.ov").unwrap();
    let compiled = ovc::compile_parsed(vec![file], &ovc::Options { asserts: true, top: None }).unwrap();
    let h = ovc::outputs::record_c_header(&compiled.records);
    assert!(h.contains("#define DMA_DESC_SIZE 8u"), "{h}");
    assert!(h.contains("#define DMA_DESC_LEN_OFFSET 4u"), "{h}");
    assert!(h.contains("  uint32_t src;  /* source address */"), "{h}");
}

#[test]
fn an_isa_has_its_opcodes_in_the_sv_package() {
    let src = "isa Mini \"tiny\" : opcode 4 { NOP = 1 \"no operation\" {} LD = 5 { rd : 5; imm : 16; } }
module M(input logic clk) {}";
    let file = ovc::parser::parse_file(src, "i.ov").unwrap();
    let compiled = ovc::compile_parsed(vec![file], &ovc::Options { asserts: true, top: None }).unwrap();
    let pkg = ovc::outputs::isa_sv_package(&compiled.isas);
    assert!(pkg.contains("  localparam int LD_OPCODE = 5;"), "{pkg}");
    assert!(pkg.contains("  localparam int LD_RD_S = 4;"), "{pkg}");
    assert!(pkg.contains("localparam int MAX_INSTR_LEN = 25;"), "{pkg}");
}

#[test]
fn a_priority_select_over_a_range_stays_a_loop() {
    // one `if` per iteration would make the text as long as the range; the loop says the same in one statement
    let out = sv("module Lead(input logic [31:0] mag, output logic [4:0] lead) {
  always_comb {
    lead = 0;
    for i in 0..32 { if (mag[i]) { lead = i; } }
  }
}");
    assert!(out.contains("for (int i = 0; i < 32; i++) begin"), "{out}");
    assert!(!out.contains("mag[31]"), "{out}");
}

#[test]
fn a_case_with_distinct_constant_labels_says_unique0() {
    // at most one label can match, so the tools may take the case as parallel; a repeated value keeps `case`
    let out = sv("module Dec(input logic [1:0] a, output logic [1:0] y) {
  always_comb {
    case (a) { 2'd0: y = 1; 2'd1: y = 2; default: y = 0; }
  }
}
module Dup(input logic [1:0] a, output logic [1:0] y) {
  always_comb {
    case (a) { 2'd0: y = 1; 2'd0: y = 2; default: y = 0; }
  }
}");
    assert!(out.contains("unique0 case (a)"), "{out}");
    assert_eq!(out.matches("unique0 case").count(), 1, "{out}");
    assert!(out.contains("    case (a)"), "{out}");
}

#[test]
fn an_output_reg_is_reset_like_a_local_reg() {
    // the implicit always_ff resets the registers it writes: an output reg port too, to 0
    let out = sv("module R(input logic clk, input logic rst, input logic en, output reg [3:0] cnt) {
  always_ff { if (en) { cnt <= cnt + 1; } }
}");
    assert!(out.contains("if (rst) begin\n      cnt <= 0;"), "{out}");
}

#[test]
fn enumerate_pairs_each_element_with_its_index() {
    let out = sv("module E(input logic [1:0] sel, output logic [7:0] y) {
  val tab = [8, 9, 10].enumerate();
  assign y = tab.select(sel, 0);
}");
    assert!(out.contains("assign y = sel == 0 ? 8 : sel == 1 ? 9 : sel == 2 ? 10 : 0;"), "{out}");
}

#[test]
fn a_loop_over_rows_in_always_comb_unrolls_each_row() {
    let out = sv("module R<int N = 3>(input logic tr, output logic [7:0] y [N]) {
  always_comb {
    for (k, lanes) in [[1, 0..N], [0, 0..N]] {
      if (tr == k) { for i in lanes { y[i] = i; } }
    }
  }
}");
    assert!(!out.contains("for ("), "{out}");
}

#[test]
fn an_integer_const_is_a_compile_time_value_not_a_declaration() {
    let out = sv("module C(input logic [1:0] s, output logic y) {
  const int A = 0, B = 1;
  assign y = s == B;
}");
    assert!(!out.contains("localparam"), "{out}");
    assert!(out.contains("assign y = s == 1;"), "{out}");
}

#[test]
fn an_instance_parameter_can_be_a_loop_value() {
    let out = sv("module Lane<int KIND>(input logic a, output logic y) {
  assign y = a ^ (KIND == 1);
}
module Top(input logic a, output logic [1:0] y) {
  for (s, kind) in [0, 1].enumerate() {
    Lane<kind> l(.a(a), .y(y[s]));
  }
}");
    assert!(out.contains("module Lane__0 ("), "{out}");
    assert!(out.contains("module Lane__1 ("), "{out}");
}
