//! Compiled output must match the reference SystemVerilog byte for byte.

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn golden(name: &str) {
    let src = fs::read_to_string(root().join("examples").join(format!("{name}.ov"))).unwrap();
    let expected = fs::read_to_string(root().join("expected").join(format!("{name}.sv"))).unwrap();
    let file = format!("examples/{name}.ov");
    let got = ovc::compile(&src, &file).unwrap_or_else(|e| panic!("{}", e.render(&src)));
    assert_eq!(got, expected, "{name}");
}

#[test]
fn counter() {
    golden("counter");
}

#[test]
fn pipe() {
    golden("pipe");
}

#[test]
fn mixed() {
    golden("mixed");
}

#[test]
fn nested() {
    golden("nested");
}

#[test]
fn widths() {
    golden("widths");
}

#[test]
fn loops() {
    golden("loops");
}

#[test]
fn fsm() {
    golden("fsm");
}

#[test]
fn cdc() {
    golden("cdc");
}

#[test]
fn adder() {
    golden("adder");
}

#[test]
fn bits() {
    golden("bits");
}

#[test]
fn systolic() {
    golden("systolic");
}

#[test]
fn hier() {
    golden("hier");
}

#[test]
fn types() {
    golden("types");
}

#[test]
fn bundle() {
    golden("bundle");
}

#[test]
fn config() {
    golden("config");
}

#[test]
fn methods() {
    golden("methods");
}

#[test]
fn classes() {
    golden("classes");
}

#[test]
fn isa() {
    golden("isa");
}

#[test]
fn probes() {
    golden("probes");
}

#[test]
fn testbench() {
    golden("testbench");
}

#[test]
fn structs() {
    golden("structs");
}
