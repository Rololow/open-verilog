//! `ovc fmt` on the examples: the layout changes, the design does not. Each example is formatted, then compiled before and
//! after: the SystemVerilog (or the error) must be the same, and the formatting must be idempotent.

use std::fs;
use std::path::Path;

#[test]
fn formatting_the_examples_keeps_the_design_and_is_idempotent() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("the examples folder") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "ov") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = fs::read_to_string(&path).unwrap();
        let formatted = ovc::fmt::checked_format(&src).unwrap_or_else(|| panic!("{name}: the tokens changed"));
        assert_eq!(ovc::fmt::format(&formatted), formatted, "{name}: not idempotent");

        let before = ovc::compile_full(&src, &name).map(|o| o.sv).map_err(|e| e.message);
        let after = ovc::compile_full(&formatted, &name).map(|o| o.sv).map_err(|e| e.message);
        assert_eq!(before, after, "{name}: the design changed");
        checked += 1;
    }
    assert!(checked >= 10, "the examples were not found ({checked})");
}
