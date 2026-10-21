//! Testbenches from `test` blocks: one module per test, a top module that reports each test.

fn example() -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read_to_string(root.join("examples").join("testbench.ov")).unwrap()
}

#[test]
fn one_module_per_test_and_a_report() {
    let src = example();
    let tb = ovc::compile_full(&src, "testbench.ov").unwrap().tests_sv.expect("the example has tests");
    assert_eq!(tb.matches("module ovc_test_").count(), 3);
    assert!(tb.contains("module ovc_tests;"));
    assert!(tb.contains("Counter__4 dut (.*);"));
    assert!(tb.contains("repeat (253) @(negedge clk);"));
    assert!(tb.contains("if (((|(q == 0)) !== 1'b1)) begin"));
    assert!(tb.contains("PASS  adds with carry  (testbench.ov:28)"));
}

#[test]
fn a_file_without_tests_has_no_testbench() {
    let src = "module M(input logic a, output logic y) { assign y = a; }";
    assert!(ovc::compile_full(src, "m.ov").unwrap().tests_sv.is_none());
}

#[test]
fn a_test_can_only_drive_inputs() {
    let src = example() + "\ntest \"bad\" for Add { s = 1; }\n";
    let e = ovc::compile_full(&src, "t.ov").unwrap_err();
    assert!(e.message.contains("'s' is not an input of 'Add'"), "{}", e.message);
}
