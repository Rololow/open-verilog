//! Pipelines with state outside the stages (what a CPU needs), and the checks / test features added with the SoC.
//! Ported from the Python `tests/test_pipeline_state.py`.

const CPU_LIKE: &str = "
pipeline P(input clk, input rst, master Stream<logic [7:0]> o, input logic [4:0] rd_in, input logic [7:0] d,
           output logic [7:0] count) {
  Stream<logic [12:0]> q;                     // an internal producer feeds stage 0
  assign q.valid = 1;
  assign q.data = {rd_in, d};
  wire valid_in = q.valid;
  logic ready_out;
  assign q.ready = ready_out;
  logic [7:0] rf [32] = '{default: 0};        // written outside the stages: stays an array
  always_ff @(posedge clk) { if (wb.fire) { rf[wb.rd] <= wb.v; } }
  reg [7:0] n;
  always_ff { if (ex.fire) { n <= (n + 1).wrap(); } }
  assign count = n;
  stage dec { logic [4:0] rd = q.data[12:8]; logic [7:0] a = rf[rd]; }
  stage ex { logic [7:0] v = (a + q.data[7:0]).wrap(); }
  stage wb {
    assign o.valid = valid;
    assign o.data = v;
    stall_when(!o.ready);
  }
}
";

fn sv(src: &str) -> String {
    ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message)).sv
}

fn error(src: &str) -> String {
    match ovc::compile(src, "t.ov") {
        Ok(_) => panic!("the design must be refused:\n{src}"),
        Err(e) => e.message,
    }
}

#[test]
fn state_outside_the_stages() {
    let out = sv(CPU_LIKE);
    // wb.rd: carried copy
    assert!(out.contains("logic [7:0] rf [32] = '{default: 0};") && out.contains("rf[rd__s2] <= v__s2;"), "{out}");
    assert!(out.contains("assign ov_fire_ex = ov_valid_ex & ~ov_hold_ex;") && out.contains("if (ov_fire_ex) begin"), "{out}");
    assert!(out.contains("assign ov_valid_dec = valid_in;") && out.contains("assign ready_out = ~ov_hold_dec;"), "{out}");
    // a bundle port is read as it is
    assert!(out.contains("assign ov_stall_wb = ov_valid_wb && !o_ready;"), "{out}");
    assert!(!out.contains("o_ready__s"), "{out}");
}

#[test]
#[ignore = "design: Rust keeps `stage.x` forwarding (a later stage's value read combinationally); Python refused it"]
fn stage_reference_to_a_later_definition_is_refused() {
    let msg = error(&CPU_LIKE.replace("logic [7:0] a = rf[rd];", "logic [7:0] a = dec.v;"));
    assert!(msg.contains("'v' is not defined in stage 'dec' or before it"), "{msg}");
}

#[test]
fn initial_and_always_ff_on_one_variable() {
    let bad = "
module M(input clk, input logic [3:0] a, input logic [7:0] d, output logic [7:0] q) {
  logic [7:0] m [16];
  initial { for i in 0..16 { m[i] = 0; } }
  always_ff @(posedge clk) { m[a] <= d; }
  assign q = m[a];
}";
    let msg = error(bad);
    assert!(msg.contains("'m' is written by an always_ff and by an initial block"), "{msg}");
    // a plain always is legal SV
    ovc::compile_full(&bad.replace("always_ff @(posedge clk)", "always @(posedge clk)"), "t.ov")
        .unwrap_or_else(|e| panic!("{}", e.message));
}

#[test]
fn signed_keeps_the_width_of_its_argument() {
    let out = sv("
module M(input logic [31:0] a, input logic [31:0] b, output logic [31:0] hi) {
  val p = $signed(a.sext(33)) * $signed(b.sext(33));
  assign hi = p[63:32];
}");
    assert!(out.contains("logic [65:0] p;") && out.contains("assign hi = p[63:32];"), "{out}");
}

#[test]
fn parameterized_tests() {
    let src = "
module Inc<int N = 1>(input logic [7:0] a, output logic [7:0] y) { assign y = (a + N).wrap(); }
for n in [1, 2, 5] {
  test \"adds\" for Inc<n> { a = 10; expect(y == 10 + n); }
}
";
    let msg = error(src);
    assert!(msg.contains("two tests are named \"adds\""), "{msg}");
    let cfg = "
val CASES = [[\"one\", 1], [\"two\", 2]];
module Inc<int N = 1>(input logic [7:0] a, output logic [7:0] y) { assign y = (a + N).wrap(); }
for (name, n) in CASES {
  test name for Inc<n> timeout 10 * n { a = 10; expect(y == 10 + n); }
}
";
    let tb = ovc::compile_full(cfg, "t.ov").unwrap_or_else(|e| panic!("{}", e.message)).tests_sv.expect("a testbench");
    assert!(tb.contains("PASS  one  (t.ov:5)") && tb.contains("PASS  two  (t.ov:5)"), "{tb}");
    assert!(tb.contains("Inc dut (.*);") && tb.contains("Inc__2 dut (.*);") && tb.contains("if (((|(y == 12)) !== 1'b1)) begin"), "{tb}");
    // timeout 10 * n cycles, n bound per test
    assert!(tb.contains("#100;") && tb.contains("#200;"), "{tb}");
}

const YOUNGER: &str = "
pipeline Q(input clk, input rst, input logic valid_in, input logic [7:0] d, input logic go, input logic hold_a,
           output logic ready_out, output logic [7:0] y) {
  stage a { logic [7:0] x = d; stall_when(pipe.hold_a); }
  stage b { logic [7:0] u = x; }
  stage c { assign y = u; flush_younger_when(pipe.go); }
}
";

#[test]
fn flush_younger_reaches_earlier_stages_through_bubbles() {
    let out = sv(YOUNGER);
    assert!(out.contains("assign ov_younger_c = ov_valid_c && go;"), "{out}");
    assert!(out.contains("assign ov_kill_c = ov_younger_c;") && out.contains("assign ov_kill_b = ov_kill_c;"), "{out}");
    // a killed input is taken, then dropped
    assert!(out.contains("assign ready_out = ~(ov_hold_a & ~ov_kill_b);"), "{out}");
    assert!(out.contains("assign ready_out = ~(ov_hold_a & ~ov_kill_b);"), "{out}");
}

#[test]
fn exponential_fold_is_reported() {
    let msg = error(
        "
module M(input logic [3:0] p [32], output logic [4:0] best) {
  assign best = (0..32).fold(5'd0, (b, i) => p[i] > p[b] ? i : b);
}",
    );
    assert!(msg.contains("more than 200000 terms"), "{msg}");
}

#[test]
fn val_keeps_the_struct_view() {
    let out = sv("
struct Pte { logic [21:0] ppn; logic [7:0] flags; logic v; logic [0:0] pad; }
module M(input logic [7:0] lo, input logic [23:0] hi, output logic [21:0] ppn, output logic v) {
  val p = {hi, lo}.as(Pte);
  assign ppn = p.ppn;
  assign v = p.v;
}");
    assert!(out.contains("assign ppn = p[31:10];") && out.contains("assign v = p[1:1];"), "{out}");
}

#[test]
fn width_of_a_ternary_chain_of_literals() {
    // val sel = fold(...) gave 1 bit: an inner `c ? 0 : 0` made the literals 1..3 count as nothing
    let out = sv("
module M(input logic [3:0] m, output logic [1:0] w) {
  val sel = (0..4).fold(0, (s, i) => m[i] ? i : s);
  assign w = sel;
}");
    assert!(out.contains("logic [1:0] sel;"), "{out}");
}

#[test]
fn ports_hide_global_names() {
    // a port `u` and a global `fn u` (microcode helpers): the port wins, in the module and in its tests
    let src = "
fn u(op, d) { return (op << 4) | d; }
val TABLE = [u(1, 2), u(3, 4)];
module M(input logic i, output logic [7:0] u) { assign u = i ? TABLE[1] : TABLE[0]; }
test \"the port\" for M { i = 1; expect(u == 'h34); }
";
    let c = ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message));
    assert!(c.sv.contains("assign u = i ? 52 : 18;"), "{}", c.sv);
    assert!(c.tests_sv.as_deref().unwrap_or("").contains("if (((|(u == 'h34)) !== 1'b1))"), "{:?}", c.tests_sv);
}

#[test]
fn statement_fn_is_expanded_where_it_is_called() {
    // a fn of statements (no return) called in a procedural block: its assignments, with the arguments bound
    let out = sv("
module M(input clk, input logic go, input logic [7:0] a, output logic [7:0] x, output logic [7:0] y) {
  fn put(v, k) { x <= v; y <= (v + k).wrap(); }
  always_ff @(posedge clk) { if (go) { put(a, 1); } else { put(0, 2); } }
}");
    for line in ["x <= a;", "y <= a + 1;", "x <= 0;", "y <= 2;"] {
        assert!(out.contains(line), "{line} missing:\n{out}");
    }
}

#[test]
fn value_fn_with_statements_is_explained() {
    let msg = error(
        "
module M(input logic [7:0] a, output logic [7:0] y) {
  fn f(v) { y = v; return v; }
  assign y = f(a);
}",
    );
    assert!(msg.contains("a fn of statements has no `return` and is called as a statement"), "{msg}");
}

#[test]
fn narrow_signed_operand_of_an_unsigned_sum() {
    // acc + $signed(a) * $signed(b): SV makes it unsigned and zero-extends the product; .ov keeps it signed
    let out = sv("
module M(input logic [31:0] acc, input logic [7:0] a, input logic [7:0] b, output logic [31:0] y) {
  assign y = (acc + $signed(a) * $signed(b)).wrap();
}");
    assert!(out.contains("acc + 32'($signed(a) * $signed(b))"), "{out}");
}

#[test]
#[ignore = "not implemented: lint has no [sign] category yet"]
fn narrow_signed_operand_in_lint() {
    let dir = std::env::temp_dir().join("ovc_pipeline_state_sign");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.sv");
    std::fs::write(
        &file,
        "
module m(input logic [31:0] acc, input logic [7:0] a, input logic [7:0] b, output logic [31:0] y);
  assign y = acc + $signed(a) * $signed(b);
endmodule
",
    )
    .unwrap();
    let report = ovc::lint::run(&ovc::lint::Options { files: vec![file.to_string_lossy().replace('\\', "/")], ..Default::default() });
    assert!(
        report.findings.iter().any(|f| f.message.contains("extended from 8 to 32 bits with zeros, not with its sign")),
        "{:?}",
        report.findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn constant_right_shift_narrows_the_value() {
    let c = ovc::compile_full(
        "module M(input logic [31:0] m, input logic s, output logic [31:0] t) { assign t = (m >> 1) + s; }",
        "t.ov",
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
}

#[test]
#[ignore = "not implemented: lint has no [sign] category yet (signed and unsigned operands in SV)"]
fn mulhsu_of_darkriscv_in_lint() {
    // the upstream darkriscv's MULHSU: a signed * unsigned product in a declaration, rs1 zero-extended
    let dir = std::env::temp_dir().join("ovc_pipeline_state_mulhsu");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.sv");
    std::fs::write(
        &file,
        "
module m(input logic signed [31:0] s1, input logic [31:0] u2, output logic [31:0] hi);
  wire signed [63:0] prod_su = s1 * u2;
  assign hi = prod_su[63:32];
endmodule
",
    )
    .unwrap();
    let report = ovc::lint::run(&ovc::lint::Options { files: vec![file.to_string_lossy().replace('\\', "/")], ..Default::default() });
    assert!(
        report.findings.iter().any(|f| f.message.contains("'s1' is signed but this expression is unsigned in SV")),
        "{:?}",
        report.findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn input_read_in_a_later_stage_must_say_which_value() {
    // an input read in a later stage: the value that came with the instruction (`b.h`) or its value now (`pipe.h`);
    // left implicit, it was silently carried (the debug halt request arrived 2 cycles late)
    let src = "
pipeline P(input clk, input rst, input logic [7:0] d, input logic h, output logic [7:0] q) {
  stage a { logic [7:0] x = d; }
  stage b { assign q = HERE ? x : 0; }   // a stage holds assignments and declarations only
}";
    let msg = error(&src.replace("HERE", "h"));
    assert!(msg.contains("write 'b.h' for the value that came with this instruction, or 'pipe.h'"), "{msg}");
    let with_b = sv(&src.replace("HERE", "b.h"));
    assert!(with_b.contains("assign q = h__s1 ? x__s1 : 0;"), "{with_b}");
    let live = sv(&src.replace("HERE", "pipe.h"));
    assert!(live.contains("assign q = h ? x__s1 : 0;") && !live.contains("h__s1") && !live.contains("$stable(h)"), "{live}");
}
