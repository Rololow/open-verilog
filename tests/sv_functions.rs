//! SystemVerilog functions inside a module: the headers and bodies of the common forms are read, and the call is
//! inlined as a conditional expression. A body outside that form stays not analyzed.

mod common;

use common::sv_of;

#[test]
fn a_function_with_an_if_else_body_becomes_a_conditional_at_its_calls() {
    let sv = sv_of(
        "module F(input logic [7:0] a, input logic [7:0] b, output logic [7:0] y);\n\
         \x20 function [7:0] mx;\n\
         \x20   input [7:0] p, q;\n\
         \x20   begin\n\
         \x20     if (p > q) mx = p; else mx = q;\n\
         \x20   end\n\
         \x20 endfunction\n\
         \x20 assign y = mx(a, b);\n\
         endmodule\n",
    );
    assert!(sv.contains("assign y = 8'(a > b ? a : b);"), "the call is inlined as a conditional: {sv}");
}

#[test]
fn a_function_with_its_inputs_in_the_header_and_dimensions_only_as_return_type_is_read() {
    let sv = sv_of(
        "module G(input logic [15:0] a, input logic [15:0] b, output logic [15:0] y);\n\
         \x20 function [15:0] lo(input [15:0] p, input [15:0] q);\n\
         \x20   lo = (p < q) ? p : q;\n\
         \x20 endfunction\n\
         \x20 assign y = lo(a, b);\n\
         endmodule\n",
    );
    assert!(sv.contains("assign y"), "{sv}");
}

#[test]
fn a_function_with_a_loop_in_its_body_is_refused_as_not_analyzed() {
    let text = "module H(input logic [7:0] a, output logic [7:0] y);\n\
                \x20 function [7:0] s;\n\
                \x20   input [7:0] p;\n\
                \x20   integer i;\n\
                \x20   begin\n\
                \x20     s = 0;\n\
                \x20     for (i = 0; i < 8; i = i + 1) s = s + p[i];\n\
                \x20   end\n\
                \x20 endfunction\n\
                \x20 assign y = s(a);\n\
                endmodule\n";
    let err = common::error_of(text);
    assert!(err.contains("a function statement is an assignment, an if or a declaration, not 'for'"), "{err}");
}

#[test]
fn a_function_of_a_package_with_assignments_is_read_and_called_by_its_package_name() {
    let sv = sv_of(
        "package P;\n\
         \x20 function automatic [7:0] mx(input [7:0] p, input [7:0] q);\n\
         \x20   if (p > q) mx = p; else mx = q;\n\
         \x20 endfunction\n\
         endpackage\n\
         module Q(input logic [7:0] a, input logic [7:0] b, output logic [7:0] y);\n\
         \x20 assign y = P::mx(a, b);\n\
         endmodule\n",
    );
    assert!(sv.contains("assign y = 8'(a > b ? a : b);"), "{sv}");
}

#[test]
fn an_integer_parameter_is_a_constant_value_of_its_width() {
    let sv = common::sv_of("module R #(parameter integer W = 8) (input logic [W-1:0] a, output logic [W-1:0] y);\n  assign y = a;\nendmodule\n");
    assert!(sv.contains("input logic [7:0] a"), "the parameter is folded to its value: {sv}");
}
