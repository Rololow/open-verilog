//! Protocol monitors: a bundle's `monitor` module watches every slave port of that bundle (AXI4: burst rules,
//! WLAST / RLAST against the lengths, responses without requests). Ported from the Python `tests/test_monitors.py`;
//! the simulation runs only when Questa is on the PATH.

mod common;

use ovc::Options;

/// A slave that answers whatever it is sent; the test drives it as a faulty master would.
const SLAVE: &str = "
module Sink(input clk, input rst, slave Axi<32, 32, 4> s, input logic rv, input logic bv) {
  clock clk;
  reset rst;

  assign s.r.valid = rv;                   // the test answers as a faulty slave would
  assign s.b.valid = bv;
  assign s.aw.ready = 1;
  assign s.w.ready = 1;
  assign s.ar.ready = 1;
  assign s.b.data = 0;
  assign s.r.data = 0;
}
";

fn options(asserts: bool) -> Options {
    Options { asserts, top: None, cover: false }
}

#[test]
fn a_monitor_on_each_slave_port() {
    let src = format!("{SLAVE}module Two(input clk, input rst, slave Vec<Axi<32, 32, 4>, 2> v) {{ clock clk; reset rst; }}");
    let sv = ovc::compile_with(&src, "t.ov", &options(true)).unwrap_or_else(|e| panic!("{}", e.message)).sv;
    // one monitor per port, named after it (the Python output named them `ov_mon_s`, `ov_mon_v_0`)
    assert!(sv.contains("AxiMonitor__32_32_4_8 s_mon0 (.clk(clk)") && sv.contains(".rst(rst))"), "{sv}");
    assert!(sv.contains(".b_aw_data(s_aw_data)") && sv.contains(".b_r_valid(s_r_valid)"), "{sv}");
    assert!(sv.contains("AxiMonitor__32_32_4_8 v_1_mon0 (") && sv.contains(".b_aw_valid(v_aw_valid[1])"), "{sv}");
    let off = ovc::compile_with(SLAVE, "t.ov", &options(false)).unwrap_or_else(|e| panic!("{}", e.message)).sv;
    assert!(!off.contains("AxiMonitor"), "--no-asserts: {off}");
}

const FAULTY_TEST: &str = "
test \"a faulty master\" for Sink {
  s.r.ready = 1;
  s.b.ready = 1;
  s.aw.data.len = 1;                       // 2 beats announced
  s.aw.data.burst = 1;
  s.aw.data.size = 2;
  s.aw.data.addr = 'h0ffc;                 // ... and they cross 4 KiB
  s.aw.valid = 1;
  step;
  s.aw.valid = 0;
  s.w.data.last = 1;                       // WLAST on the first beat
  s.w.valid = 1;
  step;
  s.w.valid = 0;
  rv = 1;                                  // a read response nobody asked for
  step;
  rv = 0;
  step(3);
}
";

#[test]
fn monitor_reports_a_faulty_master() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    let src = format!("{SLAVE}{FAULTY_TEST}");
    let compiled = ovc::compile_with(&src, "t.ov", &options(true)).unwrap_or_else(|e| panic!("{}", e.message));
    let tests = compiled.tests_sv.clone().expect("a test block gives a testbench");
    let dir = common::scratch("monitors");
    std::fs::write(dir.join("dut.sv"), &compiled.sv).expect("dut.sv");
    std::fs::write(dir.join("tests.sv"), tests).expect("tests.sv");
    let out = common::simulate(&dir, &["-sv", "+define+OVC_TESTS", "dut.sv", "tests.sv"], "ovc_tests");
    assert!(out.contains("crosses a 4 KiB boundary"), "{out}");
    assert!(out.contains("W burst of 1 beats (WLAST) but AWLEN announced 2"), "{out}");
    assert!(out.contains("R beat without an outstanding read"), "{out}");
    assert!(out.contains("FAIL  checks of the design failed") && out.contains("ovc tests: 0 passed, 1 failed"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
