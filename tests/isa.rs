//! isa: one description of the instruction formats -> decoding views, SV package, Python codec, documentation.
//! Ported from the Python `tests/test_isa.py`. The codec runs in Python (when it is installed).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn example() -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "examples", "isa.ov"].iter().collect();
    fs::read_to_string(path).expect("examples/isa.ov")
}

fn compiled(src: &str) -> ovc::Compiled {
    let c = ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.message));
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    c
}

/// Runs a Python script in `dir`, where the codec module `isa_gen.py` has been written. Returns its standard output, or
/// None when Python is not installed.
fn python(dir: &Path, script: &str) -> Option<String> {
    let out = Command::new("python").arg("-c").arg(script).current_dir(dir).output().ok()?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The codec module of the instruction sets, in `dir`.
fn write_codec(dir: &Path, c: &ovc::Compiled) {
    fs::write(dir.join("isa_gen.py"), ovc::outputs::isa_python_module(&c.isas)).unwrap();
}

#[test]
fn layout_and_views() {
    let c = compiled(&example());
    let e = &c.isas[0];
    let length = |name: &str| e.formats.iter().find(|f| f.name == name).map(|f| f.length).unwrap();
    assert_eq!(length("LD"), 18);
    assert_eq!(length("BB"), 24);
    assert_eq!(e.lengths.iter().copied().max(), Some(24));
    assert_eq!(e.lengths[5], 18);
    assert_eq!(e.lengths[12], 18);
    assert_eq!(e.lengths[0], 4);
    assert!(c.sv.contains("assign addr = instr[17:8];") && c.sv.contains("assign rd2 = instr[19:16];"), "{}", c.sv);
    assert!(c.sv.contains("assign rdv = instr[8 + bank * 4+:4];"), "{}", c.sv);
}

#[test]
fn sv_package_and_doc() {
    let c = compiled(&example());
    let pkg = ovc::outputs::isa_sv_package(&c.isas);
    assert!(pkg.contains("localparam int LD_MEM_ADDR_S = 8;") && pkg.contains("localparam int BB_RD_ADDR_W = 4;"), "{pkg}");
    assert!(pkg.contains("OPCODE_LEN_LUT [0:15] = '{4, 4, 24, 4, 4, 18"), "{pkg}");
    assert!(ovc::outputs::isa_markdown(&c.isas).contains("| 17:8 | mem_addr | data memory address |"));
}

#[test]
fn python_codec_round_trip() {
    let dir = common::scratch("isa-codec");
    let c = compiled(&example());
    write_codec(&dir, &c);
    let script = "
import isa_gen as g
E = g.Einsum
w = E.encode(E.BB, pe_mode=3, rd_addr=[1, 2, 3, 4])
fmt, f = E.decode(w)
assert fmt is E.BB and f == {'pe_mode': 3, 'rd_addr': [1, 2, 3, 4]} and w & 0xF == 2
try:
    E.encode(E.LD, mem_addr=5000)
    raise SystemExit('no error')
except ValueError as e:
    assert 'does not fit in 10 bits' in str(e), str(e)
try:
    E.encode(E.LD, adr=1)
    raise SystemExit('no error')
except ValueError as e:
    assert 'has no field' in str(e), str(e)
print('OK')
";
    match python(&dir, script) {
        Some(out) => assert!(out.contains("OK"), "{out}"),
        None => eprintln!("skipped: Python is not installed"),
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn errors_are_reported() {
    let cases = [
        ("A = 1 {} B = 1 {}", "'B' and 'A' have the same opcode 1"),
        ("A = 16 {}", "opcode 16 of 'A' does not fit in 4 bits"),
        ("A = 1 { x : 30; }", "'A' is 34 bits long, more than the maximum 32"),
        ("A = 1 { x : 2; x : 3; }", "field 'x' declared twice"),
        ("A = 1 like B;", "'A' is like 'B', which is not declared"),
        ("A = 1 { x : 0; }", "has no bits"),
    ];
    for (body, text) in cases {
        let err = ovc::compile(&format!("isa I : opcode 4 max 32 {{ {body} }}"), "t.ov").expect_err(body);
        assert!(err.message.contains(text), "'{}' does not contain '{text}'", err.message);
    }
}

#[test]
fn unknown_field_in_a_view() {
    let src = example().replace("assign addr = ld.mem_addr;", "assign addr = ld.mem_adr;");
    let err = ovc::compile(&src, "t.ov").expect_err("an unknown field");
    assert!(err.message.contains("format 'LD' has no field 'mem_adr'"), "{}", err.message);
}

