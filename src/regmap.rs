//! Register maps: one description, the RTL block (generated as .ov source, compiled like any other module, so every
//! check applies to it), and the checks of the description itself.
//!
//! ```text
//! regmap Uart "UART" {
//!   CTRL @ 'h00 "Control" {
//!     en   : rw [0]    = 0   "Enable";
//!     baud : rw [15:4] = 434 "Baud-rate divider";
//!   }
//!   IRQ @ 'h08 strobe { done : w1c [0] "Transfer finished (write 1 to clear)"; }
//! }
//! ```

use std::collections::{HashMap, HashSet};

use crate::ast::Pos;
use crate::error::{Error, Result};

/// Access types of a field.
pub const ACCESS: &[&str] = &["rw", "ro", "wo", "w1c", "w1p", "const", "counter"];
/// Width of a register: the bus is 32 bits.
const DATA_BITS: i64 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub access: String,
    pub msb: i64,
    pub lsb: i64,
    pub reset: i64,
    pub desc: String,
    pub pos: Pos,
}

impl Field {
    pub fn width(&self) -> i64 {
        self.msb - self.lsb + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reg {
    pub name: String,
    pub addr: i64,
    pub desc: String,
    pub pos: Pos,
    pub fields: Vec<Field>,
    /// `strobe`: a write or a read of the register starts an action (`<reg>_written` / `<reg>_read` pulses).
    pub strobe: bool,
}

impl Reg {
    pub fn reset_value(&self) -> i64 {
        self.fields.iter().map(|f| f.reset << f.lsb).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegMap {
    pub name: String,
    pub desc: String,
    pub pos: Pos,
    pub regs: Vec<Reg>,
}

impl RegMap {
    /// Address bits of the bus: enough for the last register, at least two.
    pub fn addr_bits(&self) -> i64 {
        let top = self.regs.iter().map(|r| r.addr).max().unwrap_or(0);
        (64 - (top as u64).leading_zeros() as i64).max(2)
    }
}

/// The checks of a description: unique names and addresses, aligned registers, fields that fit and do not overlap,
/// reset values that fit.
pub fn check(rm: &RegMap, file: &str) -> Result<()> {
    let err = |msg: String, pos: Pos| Error::new(msg, file, pos.0, pos.1);
    let mut seen_regs: HashSet<&str> = HashSet::new();
    let mut seen_addr: HashMap<i64, &str> = HashMap::new();
    for r in &rm.regs {
        if !seen_regs.insert(&r.name) {
            return Err(err(format!("register '{}' declared twice in regmap '{}'", r.name, rm.name), r.pos));
        }
        if r.addr % (DATA_BITS / 8) != 0 {
            return Err(err(format!("register '{}' at 'h{:x} is not aligned on {} bytes", r.name, r.addr, DATA_BITS / 8), r.pos));
        }
        if let Some(prev) = seen_addr.insert(r.addr, &r.name) {
            return Err(err(format!("registers '{prev}' and '{}' are both at address 'h{:x}", r.name, r.addr), r.pos));
        }
        let mut bits: HashMap<i64, &str> = HashMap::new();
        let mut names: HashSet<&str> = HashSet::new();
        for f in &r.fields {
            if !names.insert(&f.name) {
                return Err(err(format!("field '{}' declared twice in register '{}'", f.name, r.name), f.pos));
            }
            if !ACCESS.contains(&f.access.as_str()) {
                let mut known = ACCESS.to_vec();
                known.sort_unstable();
                return Err(err(
                    format!("unknown access '{}' for field '{}.{}' (expected one of {})", f.access, r.name, f.name, known.join(", ")),
                    f.pos,
                ));
            }
            if f.msb < f.lsb || f.lsb < 0 || f.msb >= DATA_BITS {
                return Err(err(
                    format!("field '{}.{}' [{}:{}] does not fit in a {DATA_BITS}-bit register", r.name, f.name, f.msb, f.lsb),
                    f.pos,
                ));
            }
            for b in f.lsb..=f.msb {
                if let Some(other) = bits.insert(b, &f.name) {
                    return Err(err(format!("fields '{}.{other}' and '{}.{}' overlap on bit {b}", r.name, r.name, f.name), f.pos));
                }
            }
            if f.reset < 0 || f.reset >= 1 << f.width() {
                return Err(err(format!("reset value {} of '{}.{}' does not fit in {} bit(s)", f.reset, r.name, f.name, f.width()), f.pos));
            }
            if f.reset != 0 && (f.access == "ro" || f.access == "w1p") {
                return Err(err(
                    format!("field '{}.{}' ({}) has no register: it cannot have a reset value", r.name, f.name, f.access),
                    f.pos,
                ));
            }
        }
    }
    Ok(())
}

/// The port of a field: `<reg>_<field>`, in lower case.
fn port(r: &Reg, f: &Field) -> String {
    format!("{}_{}", r.name, f.name).to_lowercase()
}

/// A declaration prefix of `width` bits.
fn vec_of(width: i64) -> String {
    if width > 1 { format!("logic [{}:0] ", width - 1) } else { "logic ".into() }
}

/// The RTL block of a register map, as .ov source.
pub fn module_source(rm: &RegMap) -> String {
    let aw = rm.addr_bits();
    let mut ports: Vec<String> = [
        "input clk",
        "input rst",
        "input wr",
        "input rd",
        &format!("input logic [{}:0] addr", aw - 1),
        "input logic [31:0] wdata",
        "input logic [3:0] wstrb = 4'hf",
        "output logic [31:0] rdata",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let (mut resets, mut pulses, mut writes, mut reads, mut sets) = (Vec::<String>::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for r in &rm.regs {
        let hit = format!("wr && addr == {aw}'h{:x}", r.addr);
        let rd_hit = format!("rd && addr == {aw}'h{:x}", r.addr);
        let (mut wr_stmts, mut rd_stmts) = (Vec::<String>::new(), Vec::<String>::new());
        if r.strobe {
            let name = r.name.to_lowercase();
            ports.push(format!("output logic {name}_written"));
            ports.push(format!("output logic {name}_read"));
            resets.push(format!("{name}_written <= 0;"));
            resets.push(format!("{name}_read <= 0;"));
            pulses.push(format!("{name}_written <= {hit};"));
            pulses.push(format!("{name}_read <= {rd_hit};"));
        }
        for f in &r.fields {
            let p = port(r, f);
            let sl = if f.width() > 1 { format!("[{}:{}]", f.msb, f.lsb) } else { format!("[{}]", f.lsb) };
            let new = format!("({p} & ~wmask{sl}) | (wdata{sl} & wmask{sl})"); // the enabled bytes only
            let decl = vec_of(f.width());
            match f.access.as_str() {
                "ro" => ports.push(format!("input {decl}{p}")),
                "const" => {}
                _ => ports.push(format!("output {decl}{p}")),
            }
            if f.access == "rw" || f.access == "wo" {
                resets.push(format!("{p} <= {};", f.reset));
                wr_stmts.push(format!("{p} <= {new};"));
            }
            if f.access == "w1p" {
                resets.push(format!("{p} <= 0;"));
                pulses.push(format!("{p} <= {hit} ? wdata{sl} & wmask{sl} : 0;"));
            }
            if f.access == "w1c" {
                ports.push(format!("input {decl}{p}_set"));
                resets.push(format!("{p} <= {};", f.reset));
                sets.push(format!("{p} <= {p}_set | ({hit} ? {p} & ~(wdata{sl} & wmask{sl}) : {p});"));
            }
            if f.access == "counter" {
                ports.push(format!("input {p}_inc"));
                resets.push(format!("{p} <= {};", f.reset));
                sets.push(format!("{p} <= {hit} ? {new} : {p}_inc ? ({p} + 1).wrap() : {p};"));
            }
            if matches!(f.access.as_str(), "rw" | "ro" | "w1c" | "counter") {
                rd_stmts.push(format!("rdata{sl} <= {p};"));
            }
            if f.access == "const" {
                rd_stmts.push(format!("rdata{sl} <= {}'h{:x};", f.width(), f.reset));
            }
        }
        if !wr_stmts.is_empty() {
            writes.push(format!("if ({hit}) {{ {} }}", wr_stmts.join(" ")));
        }
        if !rd_stmts.is_empty() {
            reads.push(format!("{aw}'h{:x}: {{ {} }}", r.addr, rd_stmts.join(" ")));
        }
    }
    resets.push("rdata <= 0;".into());
    writes.extend(pulses);
    writes.extend(sets);
    let case = if reads.is_empty() { String::new() } else { format!("case (addr) {{ {} }}", reads.join(" ")) };
    format!(
        "module {}({}) {{\n  wire [31:0] wmask = {{wstrb[3].fill(8), wstrb[2].fill(8), wstrb[1].fill(8), wstrb[0].fill(8)}};\n  always_ff @(posedge clk) {{\n    if (rst) {{\n      {}\n    }} else {{\n      {}\n      if (rd) {{ rdata <= 0; {} }}\n    }}\n  }}\n}}\n",
        rm.name,
        ports.join(", "),
        resets.join("\n      "),
        writes.join("\n      "),
        case
    )
}

/// The C header of the register maps: offsets, resets, and each field's shift and mask.
pub fn c_header(rms: &[RegMap]) -> String {
    let mut out = vec!["/* Generated by ovc from the regmap definitions: do not edit. */".to_string(), "#pragma once".to_string(), String::new()];
    for rm in rms {
        let n = rm.name.to_uppercase();
        let desc = if rm.desc.is_empty() { String::new() } else { format!(": {}", rm.desc) };
        out.push(format!("/* {}{desc} */", rm.name));
        for r in &rm.regs {
            let rn = format!("{n}_{}", r.name.to_uppercase());
            let mut note = if r.desc.is_empty() { String::new() } else { format!("{}; ", r.desc) };
            if r.strobe {
                note.push_str("a write / read starts an action");
            }
            let note = note.trim_matches(|c| c == ';' || c == ' ').to_string();
            let comment = if note.is_empty() { String::new() } else { format!("  /* {note} */") };
            out.push(format!("#define {rn}_OFFSET 0x{:02x}u{comment}", r.addr));
            out.push(format!("#define {rn}_RESET  0x{:08x}u", r.reset_value()));
            for f in &r.fields {
                let fname = format!("{rn}_{}", f.name.to_uppercase());
                let mask = ((1i64 << f.width()) - 1) << f.lsb;
                let desc = if f.desc.is_empty() { String::new() } else { format!(", {}", f.desc) };
                out.push(format!("#define {fname}_SHIFT {}u", f.lsb));
                out.push(format!("#define {fname}_MASK  0x{mask:08x}u  /* {}{desc} */", f.access));
            }
        }
        out.push(String::new());
    }
    out.join("\n")
}

/// The documentation of the register maps: one table per map, one per register.
pub fn markdown(rms: &[RegMap]) -> String {
    let mut out: Vec<String> = Vec::new();
    for rm in rms {
        let desc = if rm.desc.is_empty() { String::new() } else { format!(" — {}", rm.desc) };
        out.extend([format!("# {}{desc}", rm.name), String::new(), "| offset | register | reset | description |".into(), "|---|---|---|---|".into()]);
        out.extend(rm.regs.iter().map(|r| format!("| 0x{:02x} | {} | 0x{:08x} | {} |", r.addr, r.name, r.reset_value(), r.desc)));
        for r in &rm.regs {
            let desc = if r.desc.is_empty() { String::new() } else { format!(" — {}", r.desc) };
            out.extend([String::new(), format!("## {} (0x{:02x}){desc}", r.name, r.addr), String::new(), "| bits | field | access | reset | description |".into(), "|---|---|---|---|---|".into()]);
            let mut fields: Vec<&Field> = r.fields.iter().collect();
            fields.sort_by_key(|f| -f.msb);
            for f in fields {
                let bits = if f.width() > 1 { format!("{}:{}", f.msb, f.lsb) } else { f.lsb.to_string() };
                out.push(format!("| {bits} | {} | {} | {} | {} |", f.name, f.access, f.reset, f.desc));
            }
        }
        out.push(String::new());
    }
    out.join("\n")
}
