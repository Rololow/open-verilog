//! Coverage (--cover): each arm of a match / case on a hardware value taken, the enum values only a default arm handles;
//! printed by the simulation (OVC_COVER), reported by `ovc cover`. Ported from the Python `tests/test_cover.py`.

use ovc::Options;

const SRC: &str = "
enum St : logic [1:0] { IDLE = 0, RUN = 1, DONE = 2, ERR = 3 }
module M(input clk, input rst, input logic go, input logic [1:0] op, output logic [7:0] y) {
  clock clk;
  reset rst;

  reg [1:0] st;
  always_ff { case (st) { St.IDLE: { if (go) { st <= St.RUN; } } St.RUN: { st <= St.DONE; } default: { st <= St.IDLE; } } }
  assign y = match (op) { 0: 1; 1, 2: 5; default: 9 };
}";

fn with_cover(src: &str, cover: bool) -> String {
    let options = Options { asserts: true, top: None, cover };
    ovc::compile_with(src, "t.ov", &options).unwrap_or_else(|e| panic!("{}", e.message)).sv
}

#[test]
fn items_of_case_and_match() {
    let sv = with_cover(SRC, true);
    assert!(sv.contains("if (!(st == St_IDLE) && st == St_RUN) ov_cov_1 = 1'b1;"), "{sv}");
    assert!(sv.contains("ovc-cover %0d t.ov:8 case value St.DONE (default arm)"), "{sv}");
    assert!(sv.contains("if (!(op == 0) && (op == 1 || op == 2)) ov_cov_6 = 1'b1;"), "{sv}");
    assert!(sv.contains("`ifdef OVC_COVER") && sv.contains("final begin"), "{sv}");
    // only with --cover
    assert!(!with_cover(SRC, false).contains("ov_cov"));
}

#[test]
fn a_default_arm_no_value_reaches_is_not_an_item() {
    // every value of the enum has its own arm, so the default arm reaches nothing
    let full = SRC.replace("St.RUN: { st <= St.DONE; } default:", "St.RUN: { st <= St.DONE; } St.DONE, St.ERR: { } default:");
    assert!(!with_cover(&full, true).contains("case arm default"));
}

#[test]
fn report_is_the_union_of_instances_and_tests() {
    let log = [
        "ovc-cover 0 a.ov:5 case arm X",
        "ovc-cover 1 a.ov:5 case arm X",
        "ovc-cover 0 a.ov:7 match arm 1",
        "# ovc-cover 1 b.ov:2 case arm default",
    ];
    let items = ovc::cover::collect(log);
    let (text, _) = ovc::cover::report(&items);
    assert!(text.contains("2 of 3 items covered (66.7%)"), "{text}");
    assert!(text.contains("7: match arm 1") && !text.contains("a.ov:5"), "{text}");
}
