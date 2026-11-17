//! The binary does not abort on a deep source: a nesting that a test thread could not hold is compiled.

use std::fs;
use std::process::Command;

#[test]
fn a_deeply_nested_expression_is_compiled_not_a_stack_overflow() {
    let depth = 3000;
    let expr = format!("{}a{}", "(".repeat(depth), ")".repeat(depth));
    let src = format!("module M(input logic [7:0] a, output logic [7:0] y) {{\n  assign y = {expr};\n}}\n");
    let dir = std::env::temp_dir().join(format!("ovc-deep-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("deep.ov");
    fs::write(&file, &src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ovc")).arg(&file).output().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("overflowed its stack"), "{err}");
    assert!(out.status.success(), "{err}");
}
