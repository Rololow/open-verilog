//! Instruction sets: the layout of each format (the opcode in the low bits, then each field above the previous one),
//! the length of each opcode, and views (`val v = instr.as(Isa.FORMAT)`) that name the fields of a signal.

use std::collections::HashMap;

use super::env::Env;
use super::fold::int_of;
use crate::ast::{Expr, IsaDef};
use crate::error::{Error, Result};

/// A field of a format: its first bit, its width and, for an array, its number of elements.
#[derive(Debug, Clone)]
pub struct FieldLayout {
    pub name: String,
    pub start: i64,
    pub width: i64,
    pub count: Option<i64>,
    pub desc: String,
}

#[derive(Debug, Clone)]
pub struct FormatInfo {
    pub name: String,
    /// The opcode of the format, its description, and the bits of the whole instruction.
    pub code: i64,
    pub desc: String,
    pub length: i64,
    pub fields: Vec<FieldLayout>,
}

/// An instruction set, laid out: its opcode width, its formats, and the length of each opcode.
#[derive(Debug, Clone)]
pub struct IsaInfo {
    pub name: String,
    pub desc: String,
    pub opcode_bits: i64,
    /// The formats in declaration order.
    pub formats: Vec<FormatInfo>,
    pub lengths: Vec<i64>,
    /// The code of each named format (`Isa.NAME.opcode`).
    pub codes: HashMap<String, i64>,
}

/// Lays out an instruction set. Opcodes without a format have the length of the opcode.
pub fn build(def: &IsaDef, env: &Env, file: &str) -> Result<IsaInfo> {
    let fail = |msg: String| Error::new(msg, file, def.pos.0, def.pos.1);
    let value = |e: &Expr, what: &str| int_of(&super::fold::fold_expr(e, env)).ok_or_else(|| fail(format!("{what} of isa '{}' is not a constant", def.name)));
    let opcode_bits = value(&def.opcode_bits, "the opcode width")?;
    if !(1..=16).contains(&opcode_bits) {
        return Err(fail(format!("the opcode of isa '{}' takes 1 to 16 bits", def.name)));
    }
    let max_len = def.max.as_ref().map(|m| value(m, "the maximum length")).transpose()?;
    let mut formats: Vec<FormatInfo> = Vec::new();
    let mut lengths = vec![opcode_bits; 1usize << opcode_bits];
    let mut codes: HashMap<i64, String> = HashMap::new();
    let mut names: HashMap<String, i64> = HashMap::new();
    for f in &def.formats {
        let code = value(&f.code, &format!("the code of format '{}'", f.name))?;
        if !(0..(1i64 << opcode_bits)).contains(&code) {
            return Err(Error::new(format!("the opcode {code} of '{}' does not fit in {opcode_bits} bits", f.name), file, f.pos.0, f.pos.1));
        }
        if let Some(other) = codes.insert(code, f.name.clone()) {
            return Err(Error::new(format!("'{}' and '{other}' have the same opcode {code}", f.name), file, f.pos.0, f.pos.1));
        }
        // `like` copies the fields of another format; otherwise the fields follow the opcode
        let mut layout = Vec::new();
        match &f.like {
            Some(base) => {
                let base_fields = formats.iter().find(|x| &x.name == base).ok_or_else(|| Error::new(format!("'{}' is like '{base}', which is not declared", f.name), file, f.pos.0, f.pos.1))?;
                layout = base_fields.fields.clone();
            }
            None => {
                let mut next = opcode_bits;
                for field in &f.fields {
                    if layout.iter().any(|l: &FieldLayout| l.name == field.name) {
                        return Err(Error::new(format!("field '{}' declared twice in '{}'", field.name, f.name), file, f.pos.0, f.pos.1));
                    }
                    let width = value(&field.width, &format!("the width of '{}'", field.name))?;
                    if width == 0 {
                        return Err(Error::new(format!("field '{}' of '{}' has no bits", field.name, f.name), file, f.pos.0, f.pos.1));
                    }
                    let count = match &field.count {
                        Some(c) => Some(value(c, &format!("the count of '{}'", field.name))?),
                        None => None,
                    };
                    layout.push(FieldLayout { name: field.name.clone(), start: next, width, count, desc: field.desc.clone() });
                    next += width * count.unwrap_or(1);
                }
            }
        }
        let end = layout.iter().map(|l| l.start + l.width * l.count.unwrap_or(1)).max().unwrap_or(0);
        lengths[code as usize] = end.max(opcode_bits);
        let length = end.max(opcode_bits);
        if let Some(max) = max_len {
            if length > max {
                return Err(Error::new(format!("'{}' is {length} bits long, more than the maximum {max}", f.name), file, f.pos.0, f.pos.1));
            }
        }
        formats.push(FormatInfo { name: f.name.clone(), code, desc: f.desc.clone(), length, fields: layout });
        names.insert(f.name.clone(), code);
    }
    Ok(IsaInfo { name: def.name.clone(), desc: def.desc.clone(), opcode_bits, formats, lengths, codes: names })
}

/// The field of a view: the bits of the signal, for a scalar field or an element of an array.
pub fn field_bits(info: &IsaInfo, format: &str, field: &str) -> Option<FieldLayout> {
    info.formats.iter().find(|x| x.name == format)?.fields.iter().find(|f| f.name == field).cloned()
}
