//! Register maps: one source for the RTL block, the C header and the documentation. Ported from the Python
//! `tests/test_regmap.py`.

mod common;

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
