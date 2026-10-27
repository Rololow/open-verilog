//! `.as(Struct<args>)`: a view of a packed word through a parametric struct (the argument is a type). Ported from the
//! Python `tests/test_as_struct.py`.

use ovc::Options;

const SRC: &str = "
config Cfg { W = 24; }
struct SegWord<int W = 24> { logic [W/2-2:0] lo; logic mid; logic [W/2-2:0] hi; logic sgn; }
module M<config C : Cfg = Cfg>(input logic [23:0] a, output logic [10:0] lo, output logic [10:0] hi, output logic sgn) {
  val av = a.as(SegWord<C.W>);
  assign lo = av.lo;
  assign hi = av.hi;
  assign sgn = av.sgn;
}
";

fn sv(src: &str) -> Result<String, ovc::error::Error> {
    ovc::compile(src, "t.ov")
}

#[test]
fn parametric_struct_view_slices_the_word() {
    let sv = sv(SRC).unwrap_or_else(|e| panic!("{}", e.message));
    assert!(sv.contains("assign lo = av[23:13];"), "{sv}");
    assert!(sv.contains("assign hi = av[11:1];"), "{sv}");
    assert!(sv.contains("assign sgn = av[0:0];"), "{sv}");
}

#[test]
fn parametric_view_checks_the_width() {
    let bad = SRC.replace("input logic [23:0] a", "input logic [15:0] a");
    let err = sv(&bad).expect_err("a 16-bit word is not a 24-bit struct");
    assert!(err.message.contains("16 bits but the struct is 24 bits"), "{}", err.message);
}

#[test]
fn plain_struct_view_still_works() {
    let src = "
struct Word24 { logic [10:0] lo; logic mid; logic [10:0] hi; logic sgn; }
module P(input logic [23:0] a, output logic [10:0] lo) { val w = a.as(Word24); assign lo = w.lo; }
";
    let sv = sv(src).unwrap_or_else(|e| panic!("{}", e.message));
    // the view is a signal of the word, and its fields are slices of it
    assert!(sv.contains("assign lo = w[23:13];"), "{sv}");
}

#[allow(dead_code)]
fn options() -> Options {
    Options { asserts: true, top: None, cover: false }
}
