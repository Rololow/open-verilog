//! The levels of the checks in the compiler: a category at `deny` makes its findings errors, at `warn` (the default) and
//! `allow` the compilation is unchanged.

mod common;

use std::fs;

use ovc::diag::{Category, Level, Levels};
use ovc::Options;

/// A module that drops the upper four bits of its input: a width finding of the checks.
const NARROW: &str = "module Narrow(input logic [7:0] a, output logic [3:0] y) {\n  assign y = a;\n}\n";

fn with(cat: Category, level: Level) -> Options {
    let mut levels = Levels::default();
    levels.set(cat, level);
    Options { levels, ..Options::default() }
}

#[test]
fn width_is_an_error_by_default() {
    let Err(err) = ovc::compile_with(NARROW, "t.ov", &Options::default()) else { panic!("the design must be refused") };
    assert_eq!((err.line, err.col), (2, 3), "{}", err.message);
}

#[test]
fn deny_makes_the_finding_an_error_at_its_position() {
    let Err(err) = ovc::compile_with(NARROW, "t.ov", &with(Category::Width, Level::Deny)) else { panic!("the design must be refused") };
    assert_eq!((err.line, err.col), (2, 3), "{}", err.message);
    assert!(err.message.contains("bits"), "{}", err.message);
}

#[test]
fn allow_and_warn_do_not_refuse() {
    assert!(ovc::compile_with(NARROW, "t.ov", &with(Category::Width, Level::Allow)).is_ok());
    assert!(ovc::compile_with(NARROW, "t.ov", &with(Category::Width, Level::Warn)).is_ok());
}

#[test]
fn deny_of_another_category_does_not_refuse() {
    // `width` is an error by default: it is set to warn here, so that only the `latch` level is under test
    let mut levels = Levels::default();
    levels.set(Category::Width, Level::Warn);
    levels.set(Category::Latch, Level::Deny);
    assert!(ovc::compile_with(NARROW, "t.ov", &Options { levels, ..Options::default() }).is_ok());
}

#[test]
fn ignore_comment_silences_a_denied_finding() {
    let dir = common::scratch("levels-ignore");
    let path = dir.join("narrow.ov");
    let src = NARROW.replace("assign y = a;", "assign y = a; // ovc: ignore width");
    fs::write(&path, &src).unwrap();
    let file = path.to_string_lossy().into_owned();
    assert!(ovc::compile_with(&src, &file, &with(Category::Width, Level::Deny)).is_ok());
    let _ = fs::remove_dir_all(&dir);
}

/// A `.wrap()` written in a generate loop is honoured: its copies are elaborated with the loop index in their names.
fn carry_in_a_loop(rhs: &str) -> String {
    format!(
        "module Loop(input logic [2:0] a, output logic [2:0] y [2]) {{\n  for i in 0..2 {{\n    wire [2:0] w = {rhs};\n    assign y[i] = w;\n  }}\n}}\n"
    )
}

#[test]
fn a_wrap_in_a_generate_loop_is_an_intended_truncation() {
    let src = carry_in_a_loop("(a + i).wrap()");
    assert!(ovc::compile_with(&src, "t.ov", &with(Category::Carry, Level::Deny)).is_ok(), "{src}");
}

#[test]
fn a_wrap_outside_a_loop_is_an_intended_truncation() {
    let src = "module Flat(input logic [2:0] a, output logic [2:0] y) {\n  wire [2:0] w = (a + 1).wrap();\n  assign y = w;\n}\n";
    assert!(ovc::compile_with(src, "t.ov", &with(Category::Carry, Level::Deny)).is_ok());
}

#[test]
fn a_loop_without_wrap_is_still_a_carry_finding() {
    let src = carry_in_a_loop("a + i");
    let Err(err) = ovc::compile_with(&src, "t.ov", &with(Category::Carry, Level::Deny)) else { panic!("the design must be refused") };
    assert!(err.message.contains("bits"), "{}", err.message);
}

/// A signed operand mixed with an unsigned one, in a sum truncated to the target with `.wrap()`.
fn mixed_sum(u_width: u32, rhs: &str) -> String {
    format!(
        "module Mixed(input logic signed [7:0] s, input logic [{}:0] u, output logic [{}:0] y) {{\n  assign y = {rhs};\n}}\n",
        u_width - 1,
        7
    )
}

#[test]
fn a_wrapped_sum_of_operands_as_wide_as_the_target_is_not_a_sign_finding() {
    // the bits of the target do not depend on the sign of an operand that is not extended
    let src = mixed_sum(8, "(s + u).wrap()");
    assert!(ovc::compile_with(&src, "t.ov", &with(Category::Sign, Level::Deny)).is_ok(), "{src}");
}

#[test]
fn a_mixed_sum_without_wrap_is_a_sign_finding() {
    let src = mixed_sum(8, "s + u");
    assert!(ovc::compile_with(&src, "t.ov", &with(Category::Sign, Level::Deny)).is_err(), "{src}");
}

#[test]
fn a_wrapped_sum_with_a_narrower_unsigned_operand_is_not_a_sign_finding() {
    // `u` is zero-extended to 8 bits, as an unsigned value is: the bits of the target are the intended ones
    let src = mixed_sum(4, "(s + u).wrap()");
    assert!(ovc::compile_with(&src, "t.ov", &with(Category::Sign, Level::Deny)).is_ok(), "{src}");
}

#[test]
fn a_wrapped_sum_with_a_signed_operand_narrower_than_the_target_is_a_sign_finding() {
    // `s` is 4 bits and extended to 8 with zeros in SV: its negative values are wrong
    let src = "module Narrow(input logic signed [3:0] s, input logic [7:0] u, output logic [7:0] y) {\n  assign y = (s + u).wrap();\n}\n";
    assert!(ovc::compile_with(src, "t.ov", &with(Category::Sign, Level::Deny)).is_err(), "{src}");
}
