//! A view of bits as a struct (`x.as(S)`): its fields are read as slices of the bits, wherever the view is made.

fn compile(src: &str) -> String {
    ovc::compile(src, "v.ov").unwrap_or_else(|e| panic!("{}", e.render(src)))
}

const SEG: &str = "struct SegWord<int W = 24> {
  logic           sgn;
  logic [W/2-2:0] hi;
  logic           mid;
  logic [W/2-2:0] lo;
}
";

#[test]
fn a_view_in_a_generate_loop_reads_its_fields_as_slices() {
    let src = format!(
        "{SEG}module M(input logic [23:0] w [2], output logic [10:0] a [2]) {{
  for lane in 0..2 {{
    val v = w[lane].as(SegWord<24>);
    assign a[lane] = v.hi;
  }}
}}
"
    );
    let sv = compile(&src);
    // a plain vector has no members: `v_0.hi` would not compile
    assert!(sv.contains("assign a[0] = v_0[22:12];"), "{sv}");
    assert!(sv.contains("assign a[1] = v_1[22:12];"), "{sv}");
    assert!(!sv.contains("v_0.hi"), "{sv}");
}

#[test]
fn a_view_outside_a_loop_reads_its_fields_as_slices() {
    let src = format!(
        "{SEG}module M(input logic [23:0] w, output logic [10:0] lo) {{
  val v = w.as(SegWord<24>);
  assign lo = v.lo;
}}
"
    );
    let sv = compile(&src);
    assert!(sv.contains("assign lo = v[10:0];"), "{sv}");
}

#[test]
fn registers_of_a_generate_block_and_register_arrays_are_reset() {
    let src = "module M<int W = 8>(input clk, input rst, input logic [7:0] d, output logic [7:0] y) {
  clock clk;
  reset rst;

  reg [7:0] hold [2];
  if (W >= 8) {
    reg [7:0] acc;
    assign y = acc;
    always_ff { acc <= d; hold[0] <= d; hold[1] <= acc; }
  }
}
";
    let sv = compile(src);
    assert!(sv.contains("acc <= 0;"), "{sv}");
    assert!(sv.contains("hold[0] <= 0;") && sv.contains("hold[1] <= 0;"), "{sv}");
}
