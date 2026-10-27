//! Register maps: one source for the RTL block, the C header and the documentation. Ported from the Python
//! `tests/test_regmap.py`; the simulations run only when Questa is on the PATH.

mod common;

use std::collections::HashMap;
use std::fs;

const UART: &str = "
regmap Uart \"UART\" {
  CTRL @ 'h00 \"Control\" {
    en   : rw [0]      = 0   \"Enable\";
    baud : rw [15:4]   = 434 \"Baud-rate divider\";
    go   : w1p [16]          \"Start a transfer (write 1)\";
  }
  STATUS @ 'h04 \"Status\" { busy : ro [0] \"Transfer in progress\"; level : ro [11:8] \"FIFO level\"; }
  IRQ @ 'h08 { done : w1c [0] \"Transfer finished (write 1 to clear)\"; err : w1c [1]; }
  KEY @ 'h0c { key : wo [31:0] \"Write-only key\"; }
}
";

fn compiled(src: &str) -> ovc::Compiled {
    let c = ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message));
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    c
}

/// The `#define NAME VALUE` lines of a C header, the values as numbers (`0x..u` or decimal `..u`).
fn defines(header: &str) -> HashMap<String, u64> {
    header
        .lines()
        .filter_map(|line| {
            let mut words = line.strip_prefix("#define ")?.split_whitespace();
            let (name, value) = (words.next()?, words.next()?.trim_end_matches('u'));
            let n = match value.strip_prefix("0x") {
                Some(hex) => u64::from_str_radix(hex, 16).ok()?,
                None => value.parse().ok()?,
            };
            Some((name.to_string(), n))
        })
        .collect()
}

#[test]
fn rtl_header_and_doc_from_one_source() {
    let c = compiled(UART);
    assert!(c.sv.contains("module Uart") && c.sv.contains("output logic [11:0] ctrl_baud") && c.sv.contains("input logic irq_done_set"));
    let h = ovc::regmap::c_header(&c.regmaps);
    assert!(h.contains("#define UART_CTRL_RESET  0x00001b20u"), "{h}");
    assert!(h.contains("#define UART_CTRL_BAUD_MASK  0x0000fff0u"), "{h}");
    let doc = ovc::regmap::markdown(&c.regmaps);
    assert!(doc.contains("| 15:4 | baud | rw | 434 | Baud-rate divider |"), "{doc}");
}

#[test]
fn errors_are_reported_at_the_mistake() {
    let cases = [
        ("A @ 'h0 { x : rw [3:0]; y : rw [4:2]; }", "fields 'A.x' and 'A.y' overlap on bit 2"),
        ("A @ 'h0 { x : rw [0]; } B @ 'h0 { y : rw [0]; }", "registers 'A' and 'B' are both at address 'h0"),
        ("A @ 'h2 { x : rw [0]; }", "is not aligned on 4 bytes"),
        ("A @ 'h0 { x : rw [1:0] = 4; }", "reset value 4 of 'A.x' does not fit in 2 bit(s)"),
        ("A @ 'h0 { x : rx [0]; }", "unknown access 'rx'"),
        ("A @ 'h0 { x : rw [32]; }", "does not fit in a 32-bit register"),
        ("A @ 'h0 { x : ro [0] = 1; }", "cannot have a reset value"),
        ("A @ 'h0 { x : rw [0]; x : rw [1]; }", "field 'x' declared twice"),
    ];
    for (body, text) in cases {
        let err = ovc::compile(&format!("regmap R {{ {body} }}"), "t.ov").expect_err(body);
        assert!(err.message.contains(text), "'{}' does not contain '{text}'", err.message);
    }
}

#[test]
fn software_view_matches_hardware() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    // the testbench only uses the values of the generated C header: offsets, masks, resets
    let c = compiled(UART);
    let v = defines(&ovc::regmap::c_header(&c.regmaps));
    let x = |name: &str| v[name];
    let dir = common::scratch("regmap-uart");
    fs::write(dir.join("dut.sv"), &c.sv).unwrap();
    let tb = format!(
        "module tb;
  logic clk = 0, rst = 1, wr = 0, rd = 0, status_busy = 1, irq_done_set = 0, irq_err_set = 0;
  logic [3:0] addr = 0, status_level = 4'd9;
  logic [31:0] wdata = 0, rdata, key;
  logic ctrl_en, ctrl_go, irq_done, irq_err;
  logic [11:0] ctrl_baud;
  Uart dut (.*, .key_key(key));
  always #5 clk = ~clk;
  task automatic write(int a, int d); @(negedge clk) begin addr = a; wdata = d; wr = 1; end
    @(negedge clk) wr = 0; endtask
  task automatic read(int a, output logic [31:0] d); @(negedge clk) begin addr = a; rd = 1; end
    @(negedge clk) begin rd = 0; d = rdata; end endtask
  logic [31:0] d;
  initial begin
    @(negedge clk) rst = 0;
    read({ctrl}, d);
    if (d !== {reset}) $fatal(1, \"FAIL reset %h\", d);
    write({ctrl}, (123 << {baud_shift}) | {en_mask});
    if (ctrl_baud !== 123 || ctrl_en !== 1) $fatal(1, \"FAIL ctrl\");
    read({status}, d);
    if ((d & {level_mask}) >> {level_shift} !== 9 || !(d & {busy_mask}))
      $fatal(1, \"FAIL status %h\", d);
    @(negedge clk) irq_done_set = 1; @(negedge clk) irq_done_set = 0;
    read({irq}, d);
    if (d !== {done_mask}) $fatal(1, \"FAIL irq set %h\", d);
    write({irq}, {done_mask});
    read({irq}, d);
    if (d !== 0) $fatal(1, \"FAIL irq clear %h\", d);
    fork write({ctrl}, {go_mask}); @(posedge ctrl_go); join
    write({key_off}, 32'hcafe);
    read({key_off}, d);
    if (key !== 32'hcafe || d !== 0) $fatal(1, \"FAIL key %h %h\", key, d);
    $display(\"PASS\"); $finish;
  end
endmodule
",
        ctrl = x("UART_CTRL_OFFSET"),
        reset = x("UART_CTRL_RESET"),
        baud_shift = x("UART_CTRL_BAUD_SHIFT"),
        en_mask = x("UART_CTRL_EN_MASK"),
        status = x("UART_STATUS_OFFSET"),
        level_mask = x("UART_STATUS_LEVEL_MASK"),
        level_shift = x("UART_STATUS_LEVEL_SHIFT"),
        busy_mask = x("UART_STATUS_BUSY_MASK"),
        irq = x("UART_IRQ_OFFSET"),
        done_mask = x("UART_IRQ_DONE_MASK"),
        go_mask = x("UART_CTRL_GO_MASK"),
        key_off = x("UART_KEY_OFFSET"),
    );
    fs::write(dir.join("tb.sv"), tb).unwrap();
    let out = common::simulate(&dir, &["-sv", "dut.sv", "tb.sv"], "tb");
    assert!(out.contains("PASS"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}

const CSR: &str = "
regmap Csr \"accelerator CSR\" {
  MAGIC @ 'h0c \"identification\" { magic : const [31:0] = 'h4C4E5341 \"LNSA\"; }
  CTRL @ 'h40 { scratch : rw [31:0] \"free for the host\"; }
  WEN_COUNT @ 'h68 { n : counter [31:0] \"memory writes\"; }
  MEM_ADDR @ 'h60 strobe \"write: address and store\" { addr : wo [31:0]; }
}
";

#[test]
fn const_counter_strobe_and_byte_enables_generate() {
    let c = compiled(CSR);
    assert!(c.sv.contains("input logic [3:0] wstrb = 4'hf") && c.sv.contains("rdata[31:0] <= 32'h4c4e5341;"), "{}", c.sv);
    assert!(c.sv.contains("input logic wen_count_n_inc") && c.sv.contains("output logic mem_addr_written"), "{}", c.sv);
    // a constant has no port
    let ports = c.sv.split(");").next().unwrap_or("");
    assert!(!ports.contains("magic"), "{ports}");
    let h = ovc::regmap::c_header(&c.regmaps);
    assert!(h.contains("#define CSR_MAGIC_RESET  0x4c4e5341u"), "{h}");
    assert!(h.contains("a write / read starts an action"), "{h}");
}

#[test]
fn csr_behaviour() {
    if !common::on_path("vsim") {
        eprintln!("skipped: Questa (vsim) is not on the PATH");
        return;
    }
    let c = compiled(CSR);
    let dir = common::scratch("regmap-csr");
    fs::write(dir.join("dut.sv"), &c.sv).unwrap();
    fs::write(dir.join("tb.sv"), CSR_TB).unwrap();
    let out = common::simulate(&dir, &["-sv", "dut.sv", "tb.sv"], "tb");
    assert!(out.contains("PASS"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}

const CSR_TB: &str = "module tb;
  logic clk = 0, rst = 1, wr = 0, rd = 0, inc = 0; logic [6:0] addr = 0; logic [31:0] wdata = 0, rdata;
  logic [3:0] wstrb = 4'hf; logic [31:0] scratch, n, a; logic written, readp; int pulses = 0;
  Csr dut (.clk, .rst, .wr, .rd, .addr, .wdata, .wstrb, .rdata, .ctrl_scratch(scratch), .wen_count_n(n),
           .wen_count_n_inc(inc), .mem_addr_written(written), .mem_addr_read(readp), .mem_addr_addr(a));
  always #5 clk = ~clk;
  always @(posedge clk) pulses += written;
  task automatic access(input logic w, input int ad, input logic [31:0] d, input logic [3:0] s);
    @(negedge clk) begin wr = w; rd = !w; addr = ad; wdata = d; wstrb = s; end
    @(negedge clk) begin wr = 0; rd = 0; wstrb = 4'hf; end
  endtask
  initial begin
    @(negedge clk) rst = 0;
    access(1, 'h40, 32'h11223344, 4'hf);
    access(1, 'h40, 32'hAABBCCDD, 4'b0101);            // bytes 0 and 2 only
    if (scratch !== 32'h11BB33DD) $fatal(1, \"FAIL byte enables %h\", scratch);
    access(0, 'h0c, 0, 4'hf);
    if (rdata !== 32'h4C4E5341) $fatal(1, \"FAIL const %h\", rdata);
    @(negedge clk) inc = 1; repeat (3) @(negedge clk); inc = 0;
    if (n !== 3) $fatal(1, \"FAIL counter %0d\", n);
    access(1, 'h60, 32'd77, 4'hf);
    @(negedge clk);
    if (a !== 77 || pulses !== 1) $fatal(1, \"FAIL strobe %0d %0d\", a, pulses);
    $display(\"PASS\"); $finish;
  end
endmodule
";

#[test]
fn build_emits_the_header_of_a_project() {
    // `ovc build --emit-c --emit-doc`: the register maps of every file of a multi-file project
    let dir = common::scratch("regmap-project");
    fs::write(dir.join("regs.ov"), UART).unwrap();
    fs::write(
        dir.join("top.ov"),
        "module Top(input clk, input rst, input wr, input rd, input logic [3:0] addr,\n  input logic [31:0] wdata, output logic [31:0] rdata) {\n  Uart u(.clk, .rst, .wr, .rd, .addr, .wdata, .rdata);\n}\n",
    )
    .unwrap();
    let (h, doc) = (dir.join("uart.h"), dir.join("uart.md"));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_ovc"))
        .arg("build")
        .arg(&dir)
        .arg("--root")
        .arg(&dir)
        .arg("-o")
        .arg(dir.join("out"))
        .arg("--emit-c")
        .arg(&h)
        .arg("--emit-doc")
        .arg(&doc)
        .status()
        .expect("ovc runs");
    assert!(status.success());
    assert!(fs::read_to_string(&h).unwrap().contains("#define UART_CTRL_RESET  0x00001b20u"));
    assert!(fs::read_to_string(&doc).unwrap().contains("| 15:4 | baud | rw | 434 | Baud-rate divider |"));
    let _ = fs::remove_dir_all(&dir);
}
