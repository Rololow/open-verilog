//! Exported structs: one memory layout for the hardware (a view of loaded words) and the software (C, Python).
//! Ported from the Python `tests/test_records.py`. The generated Python module is checked as text, not run; the
//! SoC headers are not compared here.

const DESC: &str = "
export struct Desc \"a descriptor\" {
  logic [31:0] src \"source\";
  logic [15:0] len;
  logic signed [7:0] bias;
  logic [7:0] flags;
}
module M(input logic [63:0] w, output logic [31:0] src, output logic [15:0] len, output logic [7:0] bias) {
  val d = w.as(Desc);
  assign src = d.src;
  assign len = d.len;
  assign bias = d.bias;
}
";

fn compiled(src: &str) -> ovc::Compiled {
    ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message))
}

#[test]
fn first_field_at_the_lowest_bits() {
    // the view is a typed signal `d` of the word, and its fields are slices of it (the Python output wrote `w[31:0]`)
    let c = compiled(DESC);
    assert!(c.sv.contains("assign d = w;"), "{}", c.sv);
    assert!(c.sv.contains("assign src = d[31:0];"), "{}", c.sv);
    assert!(c.sv.contains("assign len = d[47:32];"), "{}", c.sv);
    assert!(c.sv.contains("assign bias = d[55:48];"), "{}", c.sv);
    let [r] = c.records.as_slice() else { panic!("one exported struct") };
    assert_eq!(r.size, 8);
    assert_eq!(r.fields.iter().map(|f| f.offset).collect::<Vec<_>>(), vec![0, 4, 6, 7]);
}

#[test]
fn c_header_and_python_module_have_the_layout() {
    let c = compiled(DESC);
    let h = ovc::outputs::record_c_header(&c.records);
    assert!(h.contains("  int8_t bias;"), "{h}");
    assert!(h.contains("#define DESC_LEN_OFFSET 4u"), "{h}");
    assert!(h.contains("sizeof(Desc) == 8"), "{h}");
    let py = ovc::outputs::record_python_module(&c.records);
    assert!(py.contains("class Desc(Record):"), "{py}");
    assert!(py.contains("OFFSETS = {"), "{py}");
}

/// Layouts the C struct cannot hold are errors, at the struct.
#[test]
fn layouts_c_cannot_hold_are_errors() {
    let cases = [
        ("logic [31:0] a; logic [2:0] b; logic [28:0] c;", "is 3 bits at bit 32"),
        ("logic [7:0] a; logic [31:0] b; logic [23:0] c;", "is 32 bits at bit 8"),
        ("logic [31:0] a; logic [15:0] b;", "make it a whole number of 32-bit words"),
    ];
    for (fields, text) in cases {
        let src = format!(
            "export struct S {{ {fields} }}\nmodule M(input logic [63:0] w, output logic o) {{ val s = w.as(S); assign o = 0; }}"
        );
        let err = ovc::compile(&src, "t.ov").expect_err("the layout must be refused");
        assert!(err.message.contains(text), "'{}' does not contain '{text}'", err.message);
    }
}
