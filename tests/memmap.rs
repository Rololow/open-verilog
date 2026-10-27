//! Memory maps: the devices of a bus described once (decoder regions, indices, interrupt lines), and the device tree,
//! C defines and documentation generated from the root module's map. Ported from the Python `tests/test_memmap.py`.

use ovc::Options;

const SRC: &str = "
config Cfg { extra = false; }
module Top<config C : Cfg = Cfg>(input logic a_irq, input logic b_irq, output logic [31:0] lines,
                                 output logic [31:0] n, output logic [31:0] base_b) {
  memmap io \"two devices\" {
    A @ 'h1000 size 'h100 irq 3 = a_irq compatible \"x,a\" dt \"reg-shift = <2>\" \"first\";
    B @ 'h2000 size 'h1000 irq 7 = b_irq if (C.extra);
    C2 @ 'h4000 size 'h10;
  }
  assign lines = io.irqs;
  assign n = io.count + io.C2;
  assign base_b = io.A.base;
}
";

fn compiled(src: &str, top: &str) -> ovc::Compiled {
    let options = Options { asserts: true, top: Some(top.to_string()), cover: false };
    ovc::compile_with(src, "t.ov", &options).unwrap_or_else(|e| panic!("{}", e.message))
}

#[test]
fn entries_indices_and_interrupt_lines() {
    let c = compiled(SRC, "Top");
    assert!(c.sv.contains("assign n = 3;"), "two devices (B absent), C2 is index 1:\n{}", c.sv);
    assert!(c.sv.contains("assign base_b = 4096;"), "{}", c.sv);
    assert!(c.sv.contains("a_irq, 1'b0, 1'b0, 1'b0};"), "{}", c.sv);
    assert!(!c.sv.split("assign lines").nth(1).unwrap_or("").split(';').next().unwrap_or("").contains("b_irq"), "{}", c.sv);
    let [map] = c.memmaps.as_slice() else { panic!("one memory map") };
    assert_eq!(map.0, "io");
    let names: Vec<&str> = map.2.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["A", "C2"]);
}

#[test]
fn outputs() {
    let c = compiled(&SRC.replace("extra = false", "extra = true"), "Top");
    let h = ovc::outputs::memmap_c_header(&c.memmaps);
    assert!(h.contains("#define IO_B_BASE 0x00002000u") && h.contains("#define IO_B_IRQ 7u"), "{h}");
    assert!(!h.contains("IO_C2_IRQ"), "{h}");
    let d = ovc::outputs::memmap_dts(&c.memmaps);
    assert!(d.contains("a@1000 {") && d.contains("compatible = \"x,a\";") && d.contains("interrupts = <3>;"), "{d}");
    assert!(d.contains("reg-shift = <2>;"), "{d}");
    // no compatible: not a device tree node
    assert!(!d.contains("b@"), "{d}");
    let md = ovc::outputs::memmap_markdown(&c.memmaps);
    assert!(md.contains("| A | 0x00001000 | 0x100 | 3 | x,a | first |"), "{md}");
}

#[test]
fn errors_are_reported() {
    let cases = [
        ("A @ 'h1000 size 'h100; B @ 'h10f0 size 'h10;", "devices 'A' ('h1000..'h10ff) and 'B' ('h10f0..'h10ff) overlap"),
        ("A @ 'h1000 size 'h100 irq 3; B @ 'h2000 size 'h100 irq 3;", "both use interrupt 3"),
        ("A @ 'h1000 size 'h100; regions @ 'h2000 size 'h10;", "'regions' is reserved"),
    ];
    for (entries, text) in cases {
        let src = format!("module M(output logic o) {{ memmap io {{ {entries} }} assign o = 0; }}");
        let err = ovc::compile(&src, "t.ov").expect_err(entries);
        assert!(err.message.contains(text), "'{}' does not contain '{text}'", err.message);
    }
}

#[test]
fn a_map_cannot_hide_a_signal() {
    let err = ovc::compile("module M(output logic o) { Stream<logic> io; memmap io { A @ 0 size 4; } assign o = 0; }", "t.ov")
        .expect_err("the name is taken");
    assert!(err.message.contains("memmap 'io' reuses the name of a signal"), "{}", err.message);
}
