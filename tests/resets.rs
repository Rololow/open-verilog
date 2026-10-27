//! Reset checks: a reset used both asynchronously and synchronously, or with both polarities, across the hierarchy.
//! Ported from the Python `tests/test_resets.py`.

use std::fs;

use ovc::lint::{self, Options};

const CORE: &str = "
module Core(input clk, input i_rst, input logic [7:0] d, output logic [7:0] q) {
  clock clk;
  reset i_rst low;

  always_ff @(posedge clk) { if (!i_rst) { q <= 0; } else { q <= d; } }
}
";

/// The reset warnings of a design, without their positions.
fn warnings(src: &str) -> Vec<String> {
    let compiled = ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message));
    compiled
        .warnings
        .iter()
        .filter_map(|w| w.split_once("warning: ").map(|(_, msg)| msg.to_string()))
        .filter(|msg| msg.starts_with("reset '"))
        .collect()
}

#[test]
fn async_reset_used_synchronously_in_a_child() {
    let src = format!(
        "{CORE}
module Top(input clk, input bus_reset, input logic [7:0] d, output logic [7:0] q, output logic [7:0] r) {{
  clock clk;
  reset bus_reset;

  always_ff @(posedge clk or posedge bus_reset) {{ if (bus_reset) {{ r <= 0; }} else {{ r <= d; }} }}
  Core c(.clk, .i_rst(~bus_reset), .d, .q);
}}"
    );
    assert_eq!(
        warnings(&src),
        ["reset 'bus_reset' is asynchronous in Top but synchronous in c (Core): its release is not timed the same way \
          everywhere; release it through a reset synchronizer and use one style"]
    );
}

#[test]
fn polarity_follows_the_inversion_at_the_connection() {
    let ok = format!(
        "{CORE}
module Top(input clk, input rst, input logic [7:0] d, output logic [7:0] q, output logic [7:0] r) {{
  clock clk;
  reset rst;

  always_ff @(posedge clk) {{ if (rst) {{ r <= 0; }} else {{ r <= d; }} }}
  Core c(.clk, .i_rst(~rst), .d, .q);
}}"
    );
    assert!(warnings(&ok).is_empty());
    let bad = ok.replace(".i_rst(~rst)", ".i_rst(rst)");
    assert_eq!(warnings(&bad), ["reset 'rst' is active high in Top but active low in c (Core)"]);
}

#[test]
fn both_polarities_in_one_module() {
    let src = "
module M(input clk, input rst, input logic [7:0] d, output logic [7:0] a, output logic [7:0] b) {
  clock clk;
  reset rst;

  always_ff @(posedge clk) { if (rst) { a <= 0; } else { a <= d; } }
  always_ff @(posedge clk) { if (rst == 0) { b <= 0; } else { b <= d; } }
}";
    assert_eq!(warnings(src), ["reset 'rst' is tested both active high and active low in M"]);
}

#[test]
fn reported_once_where_the_uses_meet() {
    let src = format!(
        "{CORE}
module Mid(input clk, input rst, input logic [7:0] d, output logic [7:0] q, output logic [7:0] r) {{
  clock clk;
  reset rst;

  always_ff @(posedge clk or negedge rst) {{ if (!rst) {{ r <= 0; }} else {{ r <= d; }} }}
  Core c(.clk, .i_rst(rst), .d, .q);
}}
module Top(input clk, input rst, input logic [7:0] d, output logic [7:0] q, output logic [7:0] r) {{
  Mid m(.clk, .rst, .d, .q, .r);
}}"
    );
    assert_eq!(
        warnings(&src),
        ["reset 'rst' is asynchronous in Mid but synchronous in c (Core): its release is not timed the same way \
          everywhere; release it through a reset synchronizer and use one style"]
    );
}

#[test]
fn deep_paths_are_shown() {
    let src = format!(
        "{CORE}
module Mid(input clk, input rst, input logic [7:0] d, output logic [7:0] q) {{
  Core c(.clk, .i_rst(rst), .d, .q);
}}
module Top(input clk, input rst, input logic [7:0] d, output logic [7:0] q, output logic [7:0] r) {{
  clock clk;
  reset rst;

  always_ff @(posedge clk or negedge rst) {{ if (!rst) {{ r <= 0; }} else {{ r <= d; }} }}
  Mid m(.clk, .rst, .d, .q);
}}"
    );
    let w = warnings(&src);
    assert!(w.first().is_some_and(|m| m.contains("synchronous in m.c (Core)")), "{w:?}");
}

#[test]
fn sv_lint_reports_the_reset_category() {
    let dir = std::env::temp_dir().join("ovc_resets_lint");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("top.sv");
    fs::write(
        &file,
        "module core(input logic clk, input logic i_rst, input logic [7:0] d, output logic [7:0] q);
  clock clk;
  reset i_rst low;
  always_ff @(posedge clk) if (!i_rst) q <= 0; else q <= d;
endmodule
module top(input logic clk, input logic rst, input logic [7:0] d, output logic [7:0] q, output logic [7:0] r);
  clock clk;
  reset rst;
  always_ff @(posedge clk or posedge rst) if (rst) r <= 0; else r <= d;
  core c(.clk(clk), .i_rst(~rst), .d(d), .q(q));
endmodule
",
    )
    .unwrap();
    let opts = Options { files: vec![file.to_string_lossy().replace('\\', "/")], ..Options::default() };
    let report = lint::run(&opts);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let reset: Vec<_> = report.findings.iter().filter(|f| f.category == "reset").collect();
    assert!(
        reset.iter().any(|f| f.message.contains("asynchronous in top but synchronous in c (core)")),
        "{:?}",
        report.findings.iter().map(|f| (f.category, &f.message)).collect::<Vec<_>>()
    );
}

#[test]
fn a_write_enable_is_not_a_reset() {
    // a RAM whose enable is a signal of the top: `if (we) { mem[a] <= d; }` writes data, it resets nothing
    let src = "
module Ram(input clk, input we, input logic [1:0] a, input logic [7:0] d, output logic [7:0] q) {
  clock clk;

  logic [7:0] mem [4];
  always_ff @(posedge clk) { if (we) { mem[a] <= d; } }
  assign q = mem[a];
}
module Top(input clk, input word, input logic [1:0] a, input logic [7:0] d, output logic [7:0] q1, output logic [7:0] q2) {
  Ram ra(.clk, .we(word), .a, .d, .q(q1));
  Ram rb(.clk, .we(!word), .a, .d, .q(q2));
}";
    assert!(warnings(src).is_empty(), "{:?}", warnings(src));
}

#[test]
fn every_element_of_a_reg_array_is_reset() {
    // a reg array written in always_ff is reset element by element (a v3 counter array stayed X before this)
    let src = "
module M(input clk, input rst, input logic en, input logic [1:0] i, output logic [15:0] q) {
  clock clk;
  reset rst;

  reg [15:0] cnt [4];
  always_ff { if (en) { cnt[i] <= cnt[i] + 1; } }
  assign q = cnt[i];
}";
    let sv = ovc::compile(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message));
    for k in 0..4 {
        assert!(sv.contains(&format!("cnt[{k}] <= 0;")), "{sv}");
    }
}
