//! Configs read from a manifest (`config C from "file.toml"`), and the delay lines that designs built on them use.

use std::fs;
use std::path::PathBuf;

/// A source file next to its manifest `m.toml`, in a folder of its own.
fn project(name: &str, toml: &str) -> String {
    let dir: PathBuf = std::env::temp_dir().join(format!("ovc_manifest_{name}"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("m.toml"), toml).unwrap();
    dir.join("m.ov").to_string_lossy().into_owned()
}

fn compile(src: &str, file: &str) -> String {
    ovc::compile(src, file).unwrap_or_else(|e| panic!("{}", e.render(src)))
}

#[test]
fn a_manifest_value_bounds_a_generate_loop() {
    let file = project("bound", "[array]\nk = 4\n");
    let src = "config Cf from \"m.toml\" { K = array.k; }
module M<config C = Cf>(input logic [7:0] a [4], output logic [7:0] y [4]) {
  val K = C.K;
  for lane in 0..K { assign y[lane] = a[lane]; }
}
";
    let sv = compile(src, &file);
    // the loop is a generate loop over the four lanes the manifest gives
    assert!(sv.contains("for (genvar lane = 0; lane < 4; lane++)"), "{sv}");
    assert!(sv.contains("assign y[lane] = a[lane];"), "{sv}");
}

#[test]
fn a_section_of_the_manifest_is_read_through_the_config_parameter() {
    let file = project("section", "[tables]\nres = [0, -3, -5, -6]\n");
    let src = "config Cf from \"m.toml\" { K = 1; }
module M<config C = Cf>(input logic [1:0] i, output logic [2:0] y) {
  val NEG = C.tables.res.map(v => -v);
  assign y = NEG[i];
}
";
    let sv = compile(src, &file);
    assert!(sv.contains("'{0, 3, 5, 6}"), "{sv}");
    assert!(sv.contains("assign y = NEG[i];"), "{sv}");
}

#[test]
fn a_delay_of_a_delay_knows_its_width() {
    let src = "module M(input logic clk, input logic [7:0] a, output logic [7:0] y) {
  clock clk;

  wire [7:0] m = a;
  val m1 = m.delay(1);
  val m2 = m1.delay(1);
  assign y = m2;
}
";
    let sv = compile(src, "m.ov");
    assert!(sv.contains("logic [7:0] m_d1_d1;"), "{sv}");
    assert!(sv.contains("m_d1_d1 <= m_d1;"), "{sv}");
    assert!(sv.contains("assign y = m_d1_d1;"), "{sv}");
}

#[test]
fn a_delay_of_an_element_of_a_two_dimensional_array() {
    let src = "module M(input logic clk, input logic [7:0] p [3] [2], output logic [7:0] y [2]) {
  clock clk;

  for j in 0..2 { assign y[j] = p[2][j].delay(j + 1); }
}
";
    let sv = compile(src, "m.ov");
    assert!(sv.contains("logic [7:0] p_2_0_d1;"), "{sv}");
    assert!(sv.contains("p_2_0_d1 <= p[2][0];"), "{sv}");
    assert!(sv.contains("p_2_1_d2 <= p_2_1_d1;"), "{sv}");
}
