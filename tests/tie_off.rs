//! Idle bundles: a bundle declares the value of each field when nothing drives it; an unconnected bundle port takes
//! those values, and tie_off(b) makes a bundle signal an idle master. Ported from the Python `tests/test_tie_off.py`.

const SRC: &str = "
bundle Req { output logic valid = 0; input logic ready = 1; output logic [7:0] addr = 'h40; }
bundle Bus { output Req req; input Stream<logic [7:0]> resp; }
module Mem(input clk, input rst, slave Bus b, output logic seen) {
  assign b.req.ready = 1;
  assign b.resp.valid = 0;
  assign b.resp.data = 0;
  always_ff @(posedge clk) { seen <= b.req.valid && b.resp.ready && b.req.addr == 'h40; }
}
module Top(input clk, input rst, output logic a, output logic c) {
  Vec<Bus, 2> v;
  tie_off(v[1]);
  Mem used(.clk, .rst, .b(v[1]), .seen(a));
  Mem unused(.clk, .rst, .seen(c));
}
";

#[test]
fn unconnected_bundle_port_is_idle_and_tie_off_drives_the_master_side() {
    let c = ovc::compile_full(SRC, "t.ov").unwrap_or_else(|e| panic!("{}", e.message));
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let sv = &c.sv;
    // the idle values of the unconnected port
    assert!(sv.contains("input logic b_req_valid = 0,") && sv.contains("input logic b_resp_ready = 1,"), "{sv}");
    assert!(sv.contains("assign v_req_valid[1] = 0;") && sv.contains("assign v_resp_ready[1] = 1;"), "{sv}");
    assert!(sv.contains("assign v_req_addr[1] = 'h40;"), "{sv}");
    // the slave side is left to the slave
    assert!(!sv.contains("v_req_ready[1] ="), "{sv}");
}

#[test]
fn tie_off_takes_a_bundle_signal() {
    let err = ovc::compile("module M(input clk, slave Stream<logic> s) { tie_off(s); }", "t.ov").expect_err("a port");
    assert!(err.message.contains("tie_off() takes a bundle signal"), "{}", err.message);
    let err = ovc::compile("module M(input clk) { Vec<Stream<logic>, 2> v; tie_off(v); }", "t.ov").expect_err("a vector");
    assert!(err.message.contains("an element of the Vec"), "{}", err.message);
}
