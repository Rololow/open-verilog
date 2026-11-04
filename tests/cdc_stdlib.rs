//! The clock-domain crossings of the standard library compile without a definition in the design.

use ovc::Options;

fn compiles(src: &str) {
    let r = ovc::compile_with(src, "t.ov", &Options::default());
    assert!(r.is_ok(), "{}", r.err().map(|e| e.render(src)).unwrap_or_default());
}

#[test]
fn a_pulse_crosses_with_the_stdlib_pulse_sync() {
    compiles(
        "module Top(input clk_a, input rst_a, input p, input clk_b, input rst_b, output logic q) {
  PulseSync ps(.clk_src(clk_a), .rst_src(rst_a), .pulse(p), .clk_dst(clk_b), .rst_dst(rst_b), .out(q));
}
",
    );
}

#[test]
fn a_level_crosses_with_the_stdlib_sync() {
    compiles(
        "module Top(input clk_a, input clk_b, input logic a, output logic [3:0] q) {
  logic [3:0] r;
  always_ff @(posedge clk_a) { r <= r + 1; }
  Sync<4> s(.clk(clk_b), .d(r), .q(q));
}
",
    );
}

#[test]
fn a_fifo_crosses_with_the_stdlib_async_fifo() {
    compiles(
        "module Top(input wclk, input wrst, input rclk, input rrst, input push, input pop, input logic [7:0] d,
            output logic [7:0] q, output logic full, output logic empty) {
  AsyncFifo<8, 4> f(.wclk, .wrst, .push, .wdata(d), .full, .rclk, .rrst, .pop, .rdata(q), .empty);
}
",
    );
}
