//! Diagnostics: each mistake gives its message at its position, with the offending line and a caret (ported from the
//! Python `tests/test_errors.py`).

/// (name, source, the text the message must contain, the line and column of the error if it has a position)
const CASES: &[(&str, &str, &str, Option<(usize, usize)>)] = &[
    (
        "inconnu_suggestion",
        "module M(input clk, input rst, output logic [7:0] q) {
  clock clk;
  reset rst;
\n  reg [7:0] cnt = 0;\n  always_ff {\n    cnt <= cnt2 + 1;\n  }\n  assign q = cnt;\n}\n",
        "unknown identifier 'cnt2' (did you mean 'cnt'?)",
        Some((7, 12)),
    ),
    (
        "point_virgule_manquant",
        "module M(input clk, output logic [7:0] q) {
  clock clk;
\n  always_ff @(posedge clk) {\n    q <= q + 1\n  }\n}\n",
        "expected ';' after '1'",
        Some((5, 15)),
    ),
    (
        "membre_enum",
        "enum State { IDLE, BUSY }\nmodule M(output logic y) {\n  assign y = State.BUS;\n}\n",
        "did you mean 'BUSY'",
        Some((3, 20)),
    ),
    (
        "goto_hors_etat",
        "enum S { A, B }\nmodule M(input clk, input rst) {\n  fsm S s = A {\n    A: { goto C; }\n  }\n}\n",
        "'C' is not a state of 'S'",
        // at the state named after `goto`, not at the keyword (the Python test pointed at the keyword)
        Some((4, 15)),
    ),
    (
        "trop_args_generiques",
        "module A<int W = 8>(input logic [W-1:0] d) { }\nmodule T(input logic [3:0] d) {\n  A<4, 5> a(.d(d));\n}\n",
        "too many generic arguments",
        Some((3, 3)),
    ),
    (
        "override_sans_cible",
        "module A(input clk) { }\nmodule B extends A { override fn f() -> logic { return 1; } }\n",
        "does not override anything",
        Some((2, 31)),
    ),
    (
        "methode_inconnue",
        "module A(input clk) { fn ready() -> logic { return 1; } }\nmodule T(input clk, output logic y) {\n  A a(.clk);\n  assign y = a.redy();\n}\n",
        "did you mean 'ready'",
        Some((4, 16)),
    ),
    (
        "lecture_stage_futur",
        "pipeline P(input clk, input rst, input logic [7:0] a, output logic [7:0] y) {
  clock clk;
  reset rst;
\n  stage s0 { logic [7:0] x = a + later; }\n  stage s1 { logic [7:0] later = a; assign y = later; }\n}\n",
        "forwarding",
        None,
    ),
    (
        "bits_unknown_method",
        "module M(input logic [7:0] a, output logic [7:0] y) {\n  assign y = a.reverze();\n}\n",
        "did you mean 'reverse'",
        Some((2, 16)),
    ),
    (
        "bits_unsized_cat",
        "module M(input logic [7:0] a, output logic [8:0] y) {\n  assign y = a.cat(1);\n}\n",
        "cat() needs sized operands",
        Some((2, 16)),
    ),
    (
        "pipeline_array_index",
        "pipeline P(input clk, input rst, input logic [7:0] d, output logic [7:0] y) {
  clock clk;
  reset rst;
\n  logic [7:0] v [2];\n  stage a { assign v[0] = d; }\n  stage b { assign y = v[2]; }\n}\n",
        "index 2 out of range (size 2)",
        None,
    ),
    (
        "struct_unknown_field",
        "struct S { logic [3:0] a; logic b; }\nmodule M(input S s, output logic y) {\n  assign y = s.c;\n}\n",
        "struct 'S' has no field 'c'",
        Some((3, 16)),
    ),
    (
        "import_unknown_package",
        "module M import Nope::*; (input clk) { }\n",
        "package 'Nope' not found",
        // at the package name, not at the keyword (the Python test pointed at the keyword)
        Some((1, 17)),
    ),
    (
        "bundle_unknown_field",
        "bundle S { output logic valid; input logic ready; }\nmodule M(master S o) {\n  assign o.vald = 1;\n  assign o.valid = 1;\n}\n",
        "bundle 'S' has no field 'vald' (did you mean 'valid'?)",
        Some((3, 12)),
    ),
    (
        "bundle_roles",
        "bundle S { output logic valid; input logic ready; }\nmodule A(master S o) { assign o.valid = 1; }\nmodule B(slave S i) { A a(.o(i)); }\n",
        "is a master bundle but is connected to a slave bundle port",
        Some((3, 23)),
    ),
];

/// The problems of one case, empty when it is right.
fn problems(name: &str, src: &str, msg: &str, pos: Option<(usize, usize)>) -> Vec<String> {
    let err = match ovc::compile(src, "t.ov") {
        Ok(_) => return vec![format!("{name}: the source compiles")],
        Err(e) => e,
    };
    let mut out = Vec::new();
    if !err.message.contains(msg) {
        out.push(format!("{name}: message is '{}', expected it to contain '{msg}'", err.message));
    }
    if let Some((line, col)) = pos {
        if (err.line, err.col) != (line, col) {
            out.push(format!("{name}: position is {}:{}, expected {line}:{col}", err.line, err.col));
        }
        let rendered = err.render(src);
        let text = src.lines().nth(line - 1).unwrap_or("").trim();
        if !rendered.contains(text) || !rendered.contains('^') {
            out.push(format!("{name}: the offending line and a caret are not both shown"));
        }
    }
    out
}

#[test]
fn every_mistake_gives_its_message_at_its_position() {
    let failures: Vec<String> = CASES.iter().flat_map(|(name, src, msg, pos)| problems(name, src, msg, *pos)).collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
