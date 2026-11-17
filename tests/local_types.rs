//! A typedef inside a module is the module's own: its name resolves in that module, and two modules may give the same
//! name to different types.

mod common;

use common::sv_of;

#[test]
fn a_typedef_inside_a_module_gives_its_declarations_their_type() {
    let sv = sv_of(
        "module T(input logic clk, output logic [7:0] y);\n\
         \x20 typedef logic [7:0] byte_t;\n\
         \x20 byte_t r;\n\
         \x20 always_ff @(posedge clk) r <= 8'd1;\n\
         \x20 assign y = r;\n\
         endmodule\n",
    );
    assert!(sv.contains("logic [7:0] r;"), "{sv}");
}

#[test]
fn two_modules_can_give_the_same_name_to_different_types() {
    let sv = sv_of(
        "module A(output logic [3:0] y);\n\
         \x20 typedef logic [3:0] word_t;\n\
         \x20 word_t w;\n\
         \x20 assign w = 4'd1;\n\
         \x20 assign y = w;\n\
         endmodule\n\
         module B(output logic [15:0] y);\n\
         \x20 typedef logic [15:0] word_t;\n\
         \x20 word_t w;\n\
         \x20 assign w = 16'd1;\n\
         \x20 assign y = w;\n\
         endmodule\n",
    );
    assert!(sv.contains("logic [3:0] w;") && sv.contains("logic [15:0] w;"), "{sv}");
}

#[test]
fn an_enum_typedef_inside_a_module_gives_its_members_values() {
    let sv = sv_of(
        "module E(input logic clk, output logic [1:0] y);\n\
         \x20 typedef enum logic [1:0] { Idle, Go, Done } st_e;\n\
         \x20 st_e state;\n\
         \x20 always_ff @(posedge clk) state <= Go;\n\
         \x20 assign y = state;\n\
         endmodule\n",
    );
    assert!(sv.contains("st_e_Go = 1"), "{sv}");
    assert!(sv.contains("logic [1:0] state;"), "{sv}");
}

#[test]
fn a_struct_typedef_inside_a_module_is_hoisted_and_its_signal_uses_it() {
    let sv = common::sv_of(
        "module TS(input logic [7:0] a, output logic [3:0] y);\n\
         \x20 typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;\n\
         \x20 pair_t p;\n\
         \x20 assign p = '{hi: a[7:4], lo: a[3:0]};\n\
         \x20 assign y = p.lo;\n\
         endmodule\n",
    );
    assert!(sv.contains("typedef struct packed {"), "the struct is declared once, at the file: {sv}");
    assert!(sv.contains("pair_t_pkg::pair_t p;"), "{sv}");
}

#[test]
fn an_anonymous_struct_declaration_in_a_module_gets_a_hoisted_type() {
    let sv = common::sv_of(
        "module AS(input logic [11:0] d, output logic [11:0] q);\n\
         \x20 struct packed {\n\
         \x20   logic [11:0] addr;\n\
         \x20   logic valid;\n\
         \x20 } reg_q, reg_d;\n\
         \x20 assign reg_d.addr = d;\n\
         \x20 assign reg_d.valid = 1'b1;\n\
         \x20 assign reg_q = reg_d;\n\
         \x20 assign q = reg_q.addr;\n\
         endmodule\n",
    );
    assert!(sv.contains("reg_q;") && sv.contains("reg_d;"), "{sv}");
    assert!(sv.contains("typedef struct packed {"), "{sv}");
}
