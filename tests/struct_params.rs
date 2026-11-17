//! A parameter of a struct type: each field is a constant of the parameter (`Cfg.W`), given by an assignment pattern.

mod common;

use common::{error_of, sv_of};

const PKG: &str = "package P;\n\
    \x20 typedef struct packed { int unsigned W; logic [7:0] K; } cfg_t;\n\
    \x20 localparam cfg_t cfg_default = '{W: 8, K: 8'd5};\n\
    endpackage\n";

#[test]
fn the_fields_of_a_struct_parameter_are_constants_of_the_module() {
    let sv = sv_of(&format!(
        "{PKG}module SP #(parameter P::cfg_t Cfg = P::cfg_default) (input logic [Cfg.W-1:0] a, output logic [Cfg.W-1:0] y, output logic [7:0] k);\n\
         \x20 assign y = a;\n\
         \x20 assign k = Cfg.K;\n\
         endmodule\n"
    ));
    assert!(sv.contains("input logic [7:0] a"), "the width is the field's value: {sv}");
    assert!(sv.contains("assign k = 8'd5;"), "the field's value is used: {sv}");
}

#[test]
fn an_instance_can_give_a_struct_parameter_a_pattern_of_its_own() {
    let sv = sv_of(&format!(
        "{PKG}module SP #(parameter P::cfg_t Cfg = P::cfg_default) (input logic [Cfg.W-1:0] a, output logic [Cfg.W-1:0] y);\n\
         \x20 assign y = a;\n\
         endmodule\n\
         module Top(input logic [3:0] a, output logic [3:0] y);\n\
         \x20 SP #(.Cfg('{{W: 4, K: 8'd1}})) u(.a(a), .y(y));\n\
         endmodule\n"
    ));
    assert!(sv.contains("input logic [3:0] a"), "the instance's field is the width of its ports: {sv}");
}

#[test]
fn a_pattern_that_does_not_give_every_field_is_refused() {
    let err = error_of(&format!(
        "{PKG}module SP #(parameter P::cfg_t Cfg = '{{W: 4}}) (input logic [Cfg.W-1:0] a, output logic y);\n\
         \x20 assign y = a[0];\n\
         endmodule\n"
    ));
    assert!(err.contains("does not give field 'K'"), "{err}");
}

#[test]
fn a_struct_constant_that_is_a_cast_and_not_a_pattern_is_refused_with_its_form() {
    let err = error_of(
        "package Q;\n\
         \x20 typedef struct packed { int unsigned XLEN; logic [7:0] K; } cfg_t;\n\
         \x20 localparam cfg_t cfg_empty = cfg_t'(0);\n\
         endpackage\n\
         module CD #(parameter Q::cfg_t Cfg = Q::cfg_empty) (input logic [Cfg.XLEN-1:0] a, output logic y);\n\
         \x20 assign y = a[0];\n\
         endmodule\n",
    );
    assert!(err.contains("has a struct type: its value must be an assignment pattern"), "{err}");
}

#[test]
fn a_struct_returning_function_with_field_assignments_gives_a_module_constant_its_fields() {
    let sv = sv_of(
        "package BC;\n\
         \x20 typedef struct packed { int unsigned XLEN; logic [7:0] K; } user_t;\n\
         \x20 typedef struct packed { int unsigned XLEN; int unsigned PLEN; logic [7:0] K; } cfg_t;\n\
         \x20 function automatic cfg_t build(user_t U);\n\
         \x20   bit IS64 = (U.XLEN == 64) ? 1'b1 : 1'b0;\n\
         \x20   cfg_t cfg;\n\
         \x20   cfg.XLEN = U.XLEN;\n\
         \x20   cfg.PLEN = IS64 ? 56 : 34;\n\
         \x20   cfg.K = U.K;\n\
         \x20   return cfg;\n\
         \x20 endfunction\n\
         endpackage\n\
         module BM #(parameter BC::user_t U = '{XLEN: 64, K: 8'd3}) (output logic [63:0] y);\n\
         \x20 localparam BC::cfg_t C = BC::build(U);\n\
         \x20 assign y = C.PLEN;\n\
         endmodule\n",
    );
    assert!(sv.contains("56") && !sv.contains("C.PLEN"), "the field of the constant is its value: {sv}");
}

#[test]
fn a_bit_cast_takes_the_width_of_its_type() {
    let sv = sv_of("module BC2(input logic [3:0] a, output logic y);\n  assign y = bit'(a > 4'd3);\nendmodule\n");
    assert!(sv.contains("1'("), "the cast is to one bit: {sv}");
}
