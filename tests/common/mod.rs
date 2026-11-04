//! Helpers shared by the integration tests. Each test crate uses some of them.
#![allow(dead_code)]

use std::path::PathBuf;

/// A scratch folder for one test, empty.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ovc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

/// The SystemVerilog of a design that compiles with no warning. A refused design panics with its rendered error.
pub fn sv_of(src: &str) -> String {
    let out = ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.render(src)));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    out.sv
}

/// The message of a design that must be refused.
pub fn error_of(src: &str) -> String {
    match ovc::compile(src, "t.ov") {
        Ok(_) => panic!("the design must be refused:\n{src}"),
        Err(e) => e.message,
    }
}
