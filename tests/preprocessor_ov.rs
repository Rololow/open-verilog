//! The preprocessor directives in a .ov file: `define`, `ifdef`/`else`/`endif` and the macro uses, as in a .sv file.

#[test]
fn a_define_expands_and_an_ifdef_chooses_the_branch() {
    let src = "`define WIDTH 8\n`define FAST\n\nmodule M(input logic [`WIDTH-1:0] a, output logic [`WIDTH-1:0] y) {\n  `ifdef FAST\n  assign y = a;\n  `else\n  assign y = ~a;\n  `endif\n}\n";
    let sv = ovc::compile(src, "p.ov").unwrap_or_else(|e| panic!("{}", e.render(src)));
    assert!(sv.contains("assign y = a;"), "{sv}");
    assert!(!sv.contains("~a"), "{sv}");
    assert!(sv.contains("[8-1:0]") || sv.contains("[7:0]"), "{sv}");
}

#[test]
fn an_error_after_the_directives_is_on_its_own_line() {
    let src = "`define WIDTH 8\n\nmodule M(input logic [`WIDTH-1:0] a, output logic [`WIDTH-1:0] y) {\n  assignn y = a;\n}\n";
    let Err(err) = ovc::compile(src, "p.ov") else { panic!("the design must be refused") };
    assert_eq!((err.line, err.col), (4, 3), "{}", err.message);
}
