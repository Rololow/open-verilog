//! `ovc audit`: the report of the places where a design asks the compiler to trust the author.

use ovc::ast::File;
use ovc::diag::{Category, Level, Levels};

fn units(files: &[(&str, &str)]) -> Vec<(String, String, File)> {
    files
        .iter()
        .map(|(name, src)| (name.to_string(), src.to_string(), ovc::parser::parse_file(src, name).unwrap_or_else(|e| panic!("{}", e.render(src)))))
        .collect()
}

#[test]
fn an_async_port_and_an_async_declaration_are_listed_with_their_module() {
    let u = units(&[(
        "sync.ov",
        "module Sync(input clk, input async logic d, output logic q) {\n  clock clk;\n  logic meta;\n  always_ff @(posedge clk) { meta <= d; q <= meta; }\n}\nmodule Mem(input clk, output logic q) {\n  async logic [7:0] mem [4];\n  always_ff @(posedge clk) { q <= mem[0][0]; }\n}\n",
    )]);
    let report = ovc::audit::report(&u, &Levels::default());
    assert!(report.contains("sync.ov:1  Sync: input d"), "{report}");
    assert!(report.contains("sync.ov:7  Mem: mem"), "{report}");
}

#[test]
fn same_domain_ignores_and_wraps_are_listed_and_comments_are_not_counted() {
    let u = units(&[(
        "top.ov",
        "module Top(input clk_a, input clk_b, input logic a, output logic [3:0] q) {\n  same_domain clk_a, clk_b;\n  logic r;\n  always_ff @(posedge clk_a) { r <= a; }\n  // not a truncation: .wrap() in a comment\n  always_ff @(posedge clk_b) { q <= (r + 1).wrap(); } // ovc: ignore width\n}\n",
    )]);
    let report = ovc::audit::report(&u, &Levels::default());
    assert!(report.contains("top.ov:2  Top: clk_a, clk_b"), "{report}");
    assert!(report.contains("top.ov:6  ovc: ignore width"), "{report}");
    assert!(report.contains("top.ov: 1"), "{report}");
    assert!(report.contains("lint levels below their defaults:\n  none"), "{report}");
}

#[test]
fn a_level_below_its_default_is_listed() {
    let u = units(&[("a.ov", "module A(input logic a, output logic q) { assign q = a; }\n")]);
    let mut levels = Levels::default();
    levels.set(Category::Width, Level::Warn);
    levels.set(Category::Carry, Level::Allow);
    let report = ovc::audit::report(&u, &levels);
    assert!(report.contains("width = warn (default: deny)"), "{report}");
    assert!(report.contains("carry = allow (default: deny)"), "{report}");
    assert!(!report.contains("driver ="), "{report}");
}

#[test]
fn a_clean_design_has_no_entries() {
    let u = units(&[("c.ov", "module C(input logic a, output logic q) { assign q = a; }\n")]);
    let report = ovc::audit::report(&u, &Levels::default());
    assert_eq!(report.matches("none").count(), 6, "{report}");
}
