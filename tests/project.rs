//! Multi-file projects: `ovc build` keeps the folder structure and resolves names across files. Ported from the Python
//! `tests/test_project.py`; the output is compiled with Questa (`vlog`) when it is on the PATH.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

/// Writes the project files under `root`.
fn make_project(root: &Path) {
    let files = [
        (
            "src/rtl/top.ov",
            "module Top(input clk, input logic [7:0] d, output logic [7:0] q, output logic [8:0] s) {
  Reg<Cfg.w> r(.clk, .d(d), .q(q));
  Legacy l(.a(d), .b(q), .s(s));
}",
        ),
        (
            "src/rtl/sub/reg.ov",
            "module Reg<int W = 4>(input clk, input logic [W-1:0] d, output logic [W-1:0] q) {
  clock clk;

  always_ff @(posedge clk) { q <= d; }
}",
        ),
        ("src/rtl/cfg.ov", "config Cfg { w = 8; }\n"),
        (
            "src/ip/legacy.sv",
            "module Legacy(input logic [7:0] a, input logic [7:0] b, output logic [8:0] s);
  assign s = a + b;
endmodule
",
        ),
    ];
    for (name, text) in files {
        let p = root.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}

/// Runs `ovc` with `args` in `dir`: (succeeded, standard error).
fn ovc(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ovc")).args(args).current_dir(dir).output().expect("ovc runs");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn build_mirrors_the_tree() {
    let dir = common::scratch("project-mirror");
    make_project(&dir);
    let (ok, err) = ovc(&dir, &["build", "src"]);
    assert!(ok, "{err}");
    let out = dir.join("out");
    let top = fs::read_to_string(out.join("src/rtl/top.sv")).unwrap();
    assert_eq!(top.matches("module Top").count(), 1, "{top}");
    // the generic variant lives with its definition
    let reg = fs::read_to_string(out.join("src/rtl/sub/reg.sv")).unwrap();
    assert!(reg.contains("module Reg__8"), "{reg}");
    // a config alone is no hardware: it has no output (the Python build wrote a placeholder)
    assert!(!out.join("src/rtl/cfg.sv").exists());
    // a file that is already SystemVerilog is copied as it is
    assert_eq!(
        fs::read_to_string(out.join("src/ip/legacy.sv")).unwrap(),
        fs::read_to_string(dir.join("src/ip/legacy.sv")).unwrap()
    );
    let order: Vec<String> = fs::read_to_string(out.join("files.f"))
        .unwrap()
        .split_whitespace()
        .map(|p| p.trim_start_matches("out/").to_string())
        .collect();
    assert_eq!(order, ["src/ip/legacy.sv", "src/rtl/sub/reg.sv", "src/rtl/top.sv"]);
    // no warning: Legacy may drive `s`
    assert!(!err.contains("warning"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in_place_and_filelist() {
    let dir = common::scratch("project-filelist");
    make_project(&dir);
    // (the Python list also had `+incdir+src`: `build` does not preprocess the sources, so the option is refused)
    fs::write(dir.join("rtl.f"), "// project files\nsrc/rtl/top.ov\nsrc/rtl/cfg.ov\nsrc/rtl/sub\n").unwrap();
    let (ok, err) = ovc(&dir, &["build", "-f", "rtl.f", "--in-place"]);
    assert!(ok, "{err}");
    assert!(dir.join("src/rtl/top.sv").exists() && dir.join("src/rtl/sub/reg.sv").exists());
    let listed = fs::read_to_string(dir.join("ovc_files.f")).unwrap();
    assert_eq!(listed.split_whitespace().collect::<Vec<_>>(), ["src/rtl/sub/reg.sv", "src/rtl/top.sv"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn errors_name_the_right_file() {
    let dir = common::scratch("project-errors");
    make_project(&dir);
    fs::write(
        dir.join("src/rtl/sub/reg.ov"),
        "module Reg<int W = 4>(input clk, input logic [W-1:0] d, output logic [W-1:0] q) { clock clk;
  always_ff @(posedge clk) { q <= dd; }
}
",
    )
    .unwrap();
    let (ok, err) = ovc(&dir, &["build", "src"]);
    assert!(!ok);
    assert!(err.contains("src/rtl/sub/reg.ov:2:"), "{err}");
    assert!(err.contains("unknown identifier 'dd'") && err.contains("q <= dd"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_module_across_files() {
    let dir = common::scratch("project-duplicate");
    make_project(&dir);
    fs::write(dir.join("src/rtl/again.ov"), "module Reg(input clk) { }\n").unwrap();
    let (ok, err) = ovc(&dir, &["build", "src"]);
    assert!(!ok);
    assert!(err.contains("module 'Reg' is defined twice (first in"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn output_compiles_with_its_file_list() {
    if !common::on_path("vlog") {
        eprintln!("skipped: Questa (vlog) is not on the PATH");
        return;
    }
    let dir = common::scratch("project-vlog");
    make_project(&dir);
    let (ok, err) = ovc(&dir, &["build", "src"]);
    assert!(ok, "{err}");
    // the file list names the files from the folder the build ran in
    let out = Command::new("vlib").arg("work").current_dir(&dir).output().unwrap();
    assert!(out.status.success());
    let r = Command::new("vlog").args(["-sv", "-f", "out/files.f"]).current_dir(&dir).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    let _ = fs::remove_dir_all(&dir);
}

/// Compiles one source with the library, for the checks that live in the compiler.
fn compile_one(src: &str) -> ovc::Compiled {
    ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message))
}

#[test]
fn concat_lvalue_statement() {
    let c = compile_one(
        "
module M(input clk, input logic [7:0] d, output logic [3:0] hi, output logic [3:0] lo) {
  clock clk;

  always_ff @(posedge clk) { {hi, lo} <= d; }
}",
    );
    assert!(c.sv.contains("{hi, lo} <= d;"), "{}", c.sv);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
}

#[test]
fn declarations_in_any_order() {
    let c = compile_one(
        "
module M(input logic [7:0] a, output logic [7:0] y) {
  assign y = t;
  wire [7:0] t = u;
  wire [7:0] u = a;
}",
    );
    let body = c.sv.split_once(");").map(|(_, b)| b).unwrap_or("");
    let pos = |s: &str| body.find(s).unwrap_or(usize::MAX);
    assert!(pos("wire [7:0] t;") < pos("assign y = t;"), "{}", c.sv);
    assert!(pos("wire [7:0] u;") < pos("assign t = u;"), "{}", c.sv);
}

#[test]
fn constant_logic_simplifies() {
    let c = compile_one(
        "
config K { on = false; }
module M<config C = K>(input logic a, input logic b, output logic y, output logic z) {
  assign y = C.on && a || b;
  assign z = !C.on || a;
}",
    );
    assert!(c.sv.contains("assign y = b;") && c.sv.contains("assign z = 1;"), "{}", c.sv);
}
