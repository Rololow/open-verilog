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
fn a_loop_whose_bound_is_an_input_is_refused_as_not_analyzed() {
    let text = "module H(input logic [7:0] a, input logic [3:0] n, output logic [7:0] y);
                  function [7:0] s;
                    input [7:0] p;
                    input [3:0] m;
                    integer i;
                    begin
                      s = 0;
                      for (i = 0; i < m; i = i + 1) s = s + p[i];
                    end
                  endfunction
                  assign y = s(a, n);
                endmodule
";
    let err = common::error_of(text);
    assert!(err.contains("'s' cannot be unrolled at this call"), "{err}");
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

#[test]
fn a_case_in_a_function_is_an_if_chain_on_its_labels() {
    let sv = sv_of(
        "module CC(input logic [1:0] sel, output logic y);\n\
         \x20 function automatic logic f(input logic [1:0] op);\n\
         \x20   unique case (op)\n\
         \x20     2'd1: return 1'b1;\n\
         \x20     2'd2: return 1'b1;\n\
         \x20     default: return 1'b0;\n\
         \x20   endcase\n\
         \x20 endfunction\n\
         \x20 assign y = f(sel);\n\
         endmodule\n",
    );
    assert!(sv.contains("sel == 2'd1 ? 1'b1 : sel == 2'd2 ? 1'b1 : 1'b0"), "{sv}");
}

#[test]
fn an_assertion_in_a_function_changes_no_value() {
    let sv = sv_of(
        "module AA(input logic [7:0] v, output logic y);\n\
         \x20 function automatic logic f(input logic [7:0] x);\n\
         \x20   assert (x < 8'd200) else $fatal(1, \"too big\");\n\
         \x20   return x[0];\n\
         \x20 endfunction\n\
         \x20 assign y = f(v);\n\
         endmodule\n",
    );
    assert!(sv.contains("assign y = v[0];"), "{sv}");
}

#[test]
fn a_case_range_label_is_a_bound_check_on_the_selector() {
    let sv = sv_of(
        "module CR(input logic [3:0] sel, output logic y);\n\
         \x20 function automatic logic f(input logic [3:0] op);\n\
         \x20   case (op) inside\n\
         \x20     [4'd2 : 4'd5]: return 1'b1;\n\
         \x20     default: return 1'b0;\n\
         \x20   endcase\n\
         \x20 endfunction\n\
         \x20 assign y = f(sel);\n\
         endmodule\n",
    );
    assert!(sv.contains("sel >= 4'd2 && sel <= 4'd5 ? 1'b1 : 1'b0"), "{sv}");
}

#[test]
fn a_void_function_that_only_checks_is_read_and_changes_nothing() {
    let sv = sv_of(
        "module VC(input logic [7:0] v, output logic y);\n\
         \x20 function automatic void check(input logic [7:0] x);\n\
         \x20   if (x > 8'd250) begin\n\
         \x20     $fatal(1, \"too big\");\n\
         \x20   end\n\
         \x20 endfunction\n\
         \x20 assign y = v[0];\n\
         endmodule\n",
    );
    assert!(sv.contains("assign y = v[0];"), "{sv}");
}

#[test]
fn a_loop_with_constant_bounds_is_unrolled_into_the_bits_it_assigns() {
    let sv = sv_of(
        "module LR(input logic [7:0] p, output logic [7:0] y);\n\
         \x20 function automatic logic [7:0] rev(input logic [7:0] x);\n\
         \x20   logic [7:0] r;\n\
         \x20   for (int i = 0; i < 8; i++) begin\n\
         \x20     r[i] = x[7 - i];\n\
         \x20   end\n\
         \x20   return r;\n\
         \x20 endfunction\n\
         \x20 assign y = rev(p);\n\
         endmodule\n",
    );
    assert!(sv.contains("assign y = 8'({1'(p[0]), 1'(p[1]), 1'(p[2]), 1'(p[3]), 1'(p[4]), 1'(p[5]), 1'(p[6]), 1'(p[7])});"), "{sv}");
}

#[test]
fn a_part_assignment_keeps_the_other_bits_of_the_local() {
    let sv = sv_of(
        "module PA(input logic [7:0] p, output logic [7:0] z);\n\
         \x20 function automatic logic [7:0] half(input logic [7:0] x);\n\
         \x20   logic [7:0] r;\n\
         \x20   r = x;\n\
         \x20   r[7:4] = 4'hA;\n\
         \x20   return r;\n\
         \x20 endfunction\n\
         \x20 assign z = half(p);\n\
         endmodule\n",
    );
    assert!(sv.contains("assign z = 8'({4'(4'hA), 4'(p[3:0])});"), "{sv}");
}

#[test]
fn a_local_with_a_bit_never_assigned_is_refused_not_read_as_zero() {
    let text = "module PU(input logic [7:0] p, output logic [7:0] y);\n\
                \x20 function automatic logic [7:0] part(input logic [7:0] x);\n\
                \x20   logic [7:0] r;\n\
                \x20   r[3:0] = x[3:0];\n\
                \x20   return r;\n\
                \x20 endfunction\n\
                \x20 assign y = part(p);\n\
                endmodule\n";
    let err = common::error_of(text);
    assert!(err.contains("some bits of 'r' are never assigned in the function"), "{err}");
}

#[test]
fn a_loop_bound_read_from_a_struct_argument_is_unrolled_at_the_call() {
    let sv = sv_of(
        "package DP;
           typedef struct packed { logic [3:0] n; } cfg_t;
           function automatic logic [7:0] flip(input cfg_t C, input logic [7:0] x);
             logic [7:0] r;
             r = x;
             for (int k = 0; k < C.n; k++) begin
               r[k] = ~x[k];
             end
             return r;
           endfunction
         endpackage
         module DM(input logic [7:0] a, output logic [7:0] y);
           localparam cfg_t K = '{n: 4'd3};
           assign y = DP::flip(K, a);
         endmodule
",
    );
    assert!(sv.contains("assign y = 8'({5'(a[7:3]), 1'(~a[2]), 1'(~a[1]), 1'(~a[0])});"), "{sv}");
}

#[test]
fn two_part_assignments_after_a_conditional_keep_the_bits_of_each_branch() {
    let sv = sv_of(
        "module WG(input logic s, input logic g1, input logic g2, input logic [63:0] v, input logic [28:0] ppn, output logic [40:0] y);\n\
         \x20 function automatic logic [40:0] f(input logic s, input logic g1, input logic g2, input logic [63:0] v, input logic [28:0] p);\n\
         \x20   logic [40:0] r;\n\
         \x20   if (s) begin\n\
         \x20     r = {p[28:0], v[11:0]};\n\
         \x20     if (g1) r[29:12] = v[29:12];\n\
         \x20     if (g2) r[20:12] = v[20:12];\n\
         \x20   end else begin\n\
         \x20     r = v[40:0];\n\
         \x20   end\n\
         \x20   return r;\n\
         \x20 endfunction\n\
         \x20 assign y = f(s, g1, g2, v, ppn);\n\
         endmodule\n",
    );
    assert!(sv.contains("9'(v[20:12])"), "the second part is taken from its branch: {sv}");
    assert!(sv.contains("18'(v[29:12])"), "the first part is kept for the other branch: {sv}");
}

#[test]
fn a_typed_localparam_is_a_constant_of_its_value_not_a_signal() {
    let sv = sv_of(
        "module TL(input logic [7:0] a, output logic [7:0] y);\n\
         \x20 localparam logic [3:0] N = 4'd3;\n\
         \x20 assign y = a & N;\n\
         endmodule\n",
    );
    assert!(sv.contains("assign y = a & 4'd3;") || sv.contains("assign y = a & 3;"), "{sv}");
    assert!(!sv.contains("logic [3:0] N"), "no signal is declared for the constant: {sv}");
}

#[test]
fn an_output_argument_of_a_function_is_refused_with_its_reason() {
    let err = common::error_of(
        "module OA(input logic [7:0] a, output logic [7:0] y);\n\
         \x20 function automatic logic [7:0] f(input logic [7:0] x, output logic [7:0] z);\n\
         \x20   return x;\n\
         \x20 endfunction\n\
         \x20 assign y = f(a, y);\n\
         endmodule\n",
    );
    assert!(err.contains("an output argument of a function is not supported: a function has one result"), "{err}");
}
