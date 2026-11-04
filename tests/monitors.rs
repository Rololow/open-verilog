//! Protocol monitors: a bundle's `monitor` module watches every slave port of that bundle (AXI4: burst rules,
//! WLAST / RLAST against the lengths, responses without requests). Ported from the Python `tests/test_monitors.py`.

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
    let src = format!(
        "{SLAVE}module Two(input clk, input rst, slave Vec<Axi<32, 32, 4>, 2> v) {{ clock clk; reset rst;
  for i in 0..2 {{ assign v[i].aw.ready = 1; assign v[i].w.ready = 1; assign v[i].ar.ready = 1;
    assign v[i].b.valid = 0; assign v[i].b.data = 0; assign v[i].r.valid = 0; assign v[i].r.data = 0; }}
}}"
    );
    let sv = ovc::compile_with(&src, "t.ov", &options(true)).unwrap_or_else(|e| panic!("{}", e.message)).sv;
    // one monitor per port, named after it (the Python output named them `ov_mon_s`, `ov_mon_v_0`)
    assert!(sv.contains("AxiMonitor__32_32_4_8 s_mon0 (.clk(clk)") && sv.contains(".rst(rst))"), "{sv}");
    assert!(sv.contains(".b_aw_data(s_aw_data)") && sv.contains(".b_r_valid(s_r_valid)"), "{sv}");
    assert!(sv.contains("AxiMonitor__32_32_4_8 v_1_mon0 (") && sv.contains(".b_aw_valid(v_aw_valid[1])"), "{sv}");
    let off = ovc::compile_with(SLAVE, "t.ov", &options(false)).unwrap_or_else(|e| panic!("{}", e.message)).sv;
    assert!(!off.contains("AxiMonitor"), "--no-asserts: {off}");
}

