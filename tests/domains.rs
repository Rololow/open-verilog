//! Clock domains: a crossing is an error unless it goes through an `async` port or the clocks are related; the reset
//! uses are warnings. The messages are those of the Python reference (`ovc/domains.py`).

/// The error message of a source that must not compile.
fn error(src: &str) -> String {
    match ovc::compile_full(src, "d.ov") {
        Ok(_) => panic!("expected an error for:\n{src}"),
        Err(e) => e.message,
    }
}

/// The warnings of a source that compiles.
fn warnings(src: &str) -> Vec<String> {
    ovc::compile_full(src, "d.ov").unwrap_or_else(|e| panic!("{}", e.render(src))).warnings
}

fn compiles(src: &str) {
    ovc::compile_full(src, "d.ov").unwrap_or_else(|e| panic!("{}", e.render(src)));
}

const CROSSING: &str = "module X(input clk_a, input clk_b, input logic a, output logic q) {
  logic r;
  always_ff @(posedge clk_a) { r <= a; }
  always_ff @(posedge clk_b) { q <= r; }
}
";

#[test]
fn a_register_that_reads_another_domain_is_a_crossing() {
    assert_eq!(
        error(CROSSING),
        "clock-domain crossing in module 'X': 'r' (domain 'clk_a') is read by register q (domain 'clk_b'). Pass it \
         through a synchronizer (an `async` input, e.g. Sync or an asynchronous FIFO) or, if the clocks are synchronous, \
         declare `same_domain clk_a, clk_b;`"
    );
}

#[test]
fn a_combinational_path_keeps_the_domain_of_its_source() {
    let src = "module X(input clk_a, input clk_b, input logic a, output logic q) {
  logic r;
  logic w;
  always_ff @(posedge clk_a) { r <= a; }
  assign w = r & a;
  always_ff @(posedge clk_b) { q <= w; }
}
";
    assert!(error(src).contains("'w' (domain 'clk_a') is read by register q (domain 'clk_b')"));
}

#[test]
fn a_synchronizer_takes_the_value_through_an_async_input() {
    compiles(
        "module Sync<int W = 1>(input clk, input async logic [W-1:0] d, output logic [W-1:0] q) {
  logic [W-1:0] meta;
  always_ff @(posedge clk) { meta <= d; q <= meta; }
}
module Top(input clk_a, input clk_b, input logic a, output logic q) {
  logic r;
  always_ff @(posedge clk_a) { r <= a; }
  Sync s(.clk(clk_b), .d(r), .q(q));
}
",
    );
}

#[test]
fn same_domain_clocks_may_cross() {
    compiles(
        "module Top(input clk_a, input clk_b, input logic a, output logic q) {
  same_domain clk_a, clk_b;
  logic r;
  always_ff @(posedge clk_a) { r <= a; }
  always_ff @(posedge clk_b) { q <= r; }
}
",
    );
}

#[test]
fn an_async_memory_may_be_read_in_another_domain() {
    compiles(
        "module Top(input wclk, input rclk, input logic [7:0] d, input logic [3:0] wa, input logic [3:0] ra, output logic [7:0] q) {
  async logic [7:0] mem [16];
  always_ff @(posedge wclk) { mem[wa] <= d; }
  always_ff @(posedge rclk) { q <= mem[ra]; }
}
",
    );
}

#[test]
fn an_instance_input_used_from_another_domain_is_a_crossing() {
    let src = "module Child(input clk, input logic d, output logic q) {
  always_ff @(posedge clk) { q <= d; }
}
module Top(input clk_a, input clk_b, input logic a, output logic q) {
  logic r;
  always_ff @(posedge clk_a) { r <= a; }
  Child c(.clk(clk_b), .d(r), .q(q));
}
";
    assert!(error(src).contains("'r' (domain 'clk_a') is read by input 'd' of instance 'c' (domain 'clk_b')"));
}

#[test]
fn an_input_used_in_two_domains_is_an_error() {
    let src = "module Child(input clk, input logic d, output logic q) {
  always_ff @(posedge clk) { q <= d; }
}
module Top(input clk_a, input clk_b, input logic x, output logic q1, output logic q2) {
  Child c1(.clk(clk_a), .d(x), .q(q1));
  Child c2(.clk(clk_b), .d(x), .q(q2));
}
";
    assert!(error(src).starts_with("clock-domain crossing in module 'Top': input 'x' is used in domain 'clk_a'"));
}

#[test]
fn a_reset_asynchronous_in_one_place_and_synchronous_in_another_warns() {
    let src = "module Top(input clk, input rst, input logic a, output logic q) {
  logic r;
  always_ff @(posedge clk or negedge rst) { if (!rst) { r <= 0; } else { r <= a; } }
  always_ff @(posedge clk) { if (rst) { q <= 0; } else { q <= r; } }
}
";
    let w = warnings(src);
    assert!(
        w.iter().any(|x| x.contains("reset 'rst' is asynchronous in Top but synchronous in Top")),
        "{w:?}"
    );
}

#[test]
fn a_reset_tested_active_high_and_active_low_warns() {
    let src = "module Top(input clk, input rst, input logic a, output logic q, output logic p) {
  logic r;
  always_ff @(posedge clk) { if (rst) { r <= 0; } else { r <= a; } }
  always_ff @(posedge clk) { if (!rst) { p <= 0; } else { p <= r; } }
  assign q = r;
}
";
    let w = warnings(src);
    assert!(w.iter().any(|x| x.contains("reset 'rst' is tested both active high and active low in Top")), "{w:?}");
}

#[test]
fn a_producer_whose_valid_waits_for_ready_is_an_error() {
    let src = "bundle Stream<type T> {
  output logic valid;
  input  logic ready;
  output T data;
}
module Src(input clk, master Stream<logic [7:0]> out) {
  assign out.valid = out.ready;
  assign out.data = 0;
}
";
    assert!(error(src).starts_with("module 'Src': out.valid depends combinationally on out.ready."));
}

#[test]
fn the_examples_and_the_bundle_example_are_not_crossings() {
    compiles(include_str!("../examples/cdc.ov"));
    compiles(include_str!("../examples/bundle.ov"));
}

#[test]
fn a_nested_producer_whose_valid_waits_for_ready_is_an_error() {
    let src = "bundle Stream<type T> {
  output logic valid;
  input  logic ready;
  output T data;
}
bundle Pair { output Stream<logic [7:0]> s; }
module Src(input clk, master Pair p) {
  assign p.s.valid = p.s.ready;
  assign p.s.data = 0;
}
";
    assert!(error(src).starts_with("module 'Src': p.s.valid depends combinationally on p.s.ready."));
}
