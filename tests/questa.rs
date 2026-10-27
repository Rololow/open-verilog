//! The examples simulated in Questa: each one compiles, its testbench passes, and the simulation has no error. Ported from
//! the Python `tests/test_questa.py`. The testbenches are in `tests/tb` of the repository.

mod common;

use std::fs;
use std::path::PathBuf;

use ovc::Options;

/// One simulation: (case, example, top, testbench, generics). A generic `DUT` names the module through a define.
struct Case {
    id: &'static str,
    example: &'static str,
    top: Option<&'static str>,
    tb: &'static str,
    generics: Vec<(&'static str, String)>,
}

fn case(id: &'static str, example: &'static str, top: Option<&'static str>, tb: &'static str) -> Case {
    Case { id, example, top, tb, generics: Vec::new() }
}

fn cases() -> Vec<Case> {
    let mut out = vec![
        case("counter", "counter", None, "counter_tb"),
        case("shift", "loops", Some("Shift<4, 8>"), "shift_tb"),
        case("classes", "classes", Some("Top"), "top_tb"),
        case("mac", "pipeline", Some("Mac"), "mac_tb"),
        case("counter2", "qol", Some("Counter2"), "counter2_tb"),
        case("flag", "qol", Some("Flag"), "flag_tb"),
        case("mac3", "qol", Some("Mac3"), "mac3_tb"),
        case("handshake", "fsm", Some("Handshake"), "handshake_tb"),
        case("alu", "fsm", Some("Alu"), "alu_tb"),
        case("bundle", "bundle", Some("Top"), "bundle_tb"),
        case("bits", "bits", Some("Bits"), "bits_tb"),
        case("config", "config", Some("Top"), "config_tb"),
        case("dual", "methods", Some("Dual"), "dual_tb"),
        case("luma", "methods", Some("Luma"), "luma_tb"),
        case("widths", "widths", Some("Widths"), "widths_tb"),
        case("pack", "types", Some("Pack"), "pack_tb"),
        case("swap", "types", Some("Swap"), "swap_tb"),
    ];
    for n in [0, 1, 3, 8] {
        out.push(Case { generics: vec![("N", n.to_string())], ..case(leak(format!("chain{n}")), "chain", Some(leak(format!("AddChain<{n}>"))), "chain_tb") });
    }
    for n in [1, 5] {
        out.push(Case {
            generics: vec![("N", n.to_string()), ("DUT", "VecSum".to_string())],
            ..case(leak(format!("vecsum{n}")), "bits", Some(leak(format!("VecSum<{n}, 8>"))), "adder_tb")
        });
    }
    out.push(Case { generics: vec![("DUT", "Mac2".to_string())], ..case("mac2", "qol", Some("Mac2"), "mac_tb") });
    out.push(case("slow", "pipeline", Some("Slow"), "slow_tb"));
    out.push(case("flushy", "pipeline", Some("Flushy"), "flushy_tb"));
    for n in [1, 2, 4, 7, 16] {
        out.push(Case { generics: vec![("N", n.to_string())], ..case(leak(format!("adder{n}")), "adder", Some(leak(format!("Adder<{n}, 8>"))), "adder_tb") });
    }
    for s in [0, 1, 4] {
        out.push(Case {
            generics: vec![("STAGES", s.to_string())],
            ..case(leak(format!("pipe{s}")), "pipe", Some(leak(format!("Pipe<logic [15:0], {s}>"))), "pipe_tb")
        });
    }
    out
}

/// The names of the generated cases live for the whole run.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn example(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "examples", &format!("{name}.ov")].iter().collect();
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{name}.ov: {e}"))
}

fn testbench(tb: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "tests", "tb", &format!("{tb}.sv")].iter().collect()
}

/// The SystemVerilog of an example, compiled with its top module.
fn compile_example(name: &str, top: Option<&str>) -> Result<String, String> {
    let src = example(name);
    let file = format!("{name}.ov");
    let options = Options { asserts: true, top: top.map(String::from), cover: false };
    ovc::compile_with(&src, &file, &options).map(|c| c.sv).map_err(|e| e.message)
}

/// Runs one case: the failure as text, or nothing when it passes.
fn run_case(c: &Case) -> Result<(), String> {
    let dut = compile_example(c.example, c.top).map_err(|e| format!("{}: compile: {e}", c.id))?;
    let dir = common::scratch(&format!("questa-{}", c.id));
    fs::write(dir.join("dut.sv"), dut).unwrap();
    let defines: Vec<String> = c.generics.iter().filter(|(k, _)| *k == "DUT").map(|(_, v)| format!("+define+DUT={v}")).collect();
    let tb_path = testbench(c.tb).to_string_lossy().into_owned();
    let mut vlog: Vec<&str> = vec!["-sv"];
    vlog.extend(defines.iter().map(String::as_str));
    vlog.extend(["dut.sv", tb_path.as_str()]);
    let generics: Vec<String> = c.generics.iter().filter(|(k, _)| *k != "DUT").map(|(k, v)| format!("-g{k}={v}")).collect();
    let generic_refs: Vec<&str> = generics.iter().map(String::as_str).collect();
    let out = common::simulate_with(&dir, &vlog, &generic_refs, c.tb);
    let _ = fs::remove_dir_all(&dir);
    if !out.contains("PASS") {
        return Err(format!("{}: the testbench did not pass:\n{out}", c.id));
    }
    if out.contains("** Error") {
        return Err(format!("{}: the simulation reported an error:\n{out}", c.id));
    }
    Ok(())
}

#[test]
fn simulation() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    let failures: Vec<String> = cases().iter().filter_map(|c| run_case(c).err()).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn mixed_compiles() {
    if !common::on_path("vlog") {
        eprintln!("skipped: Questa (vlog) is not on the PATH");
        return;
    }
    let sv = compile_example("mixed", None).unwrap_or_else(|e| panic!("{e}"));
    let dir = common::scratch("questa-mixed");
    fs::write(dir.join("dut.sv"), sv).unwrap();
    common::questa(&dir, "vlib", &["work"]);
    common::questa(&dir, "vlog", &["-sv", "dut.sv"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn module_param_and_extends_compile() {
    if !common::on_path("vlog") {
        eprintln!("skipped: Questa (vlog) is not on the PATH");
        return;
    }
    let sv = compile_example("classes", Some("Top<KillableStage>")).unwrap_or_else(|e| panic!("{e}"));
    let dir = common::scratch("questa-classes");
    fs::write(dir.join("dut.sv"), sv).unwrap();
    common::questa(&dir, "vlib", &["work"]);
    common::questa(&dir, "vlog", &["-sv", "dut.sv"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pipeline_assertion_catches_lost_input() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    let sv = compile_example("pipeline", Some("Slow")).unwrap_or_else(|e| panic!("{e}"));
    let dir = common::scratch("questa-lost");
    fs::write(dir.join("dut.sv"), sv).unwrap();
    let tb = testbench("slow_bad_tb").to_string_lossy().into_owned();
    let out = common::simulate(&dir, &["-sv", "dut.sv", tb.as_str()], "slow_bad_tb");
    let _ = fs::remove_dir_all(&dir);
    assert!(out.contains("that input is lost"), "{out}");
}
