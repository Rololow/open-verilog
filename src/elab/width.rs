//! Lossless width inference, as the reference does it (ovc/width.py): the result of `+` and `*` is as wide as
//! the largest value it can hold, so `a * b` on 8-bit operands is 16 bits and `p + c` on a 16-bit `p` is 17 bits.
//! Each expression is a width, whether it is a flexible unsized literal, and its largest value.

use super::env::{Env, StructInfo};
use super::fold::int_of;
use crate::ast::{DType, Expr};

#[derive(Clone, Copy, Debug)]
pub struct Width {
    pub width: u32,
    /// An unsized literal: it takes the width of its context and does not force one.
    pub flex: bool,
    pub max: u128,
}

fn mask(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

/// Bits needed for `v` (at least one).
fn bits(v: u128) -> u32 {
    (128 - v.leading_zeros()).max(1)
}

fn full(w: u32) -> Width {
    Width { width: w, flex: false, max: mask(w) }
}

/// The largest value of parts packed side by side (the last part at the bottom), each at its own largest value.
/// Beyond 127 bits the packed value is not tracked: its declared width is the bound.
fn packed_max(ws: &[Width]) -> u128 {
    let total: u32 = ws.iter().map(|w| w.width).sum();
    if total > 127 {
        return mask(total);
    }
    let mut max = 0u128;
    let mut shift = 0u32;
    for w in ws.iter().rev() {
        max |= w.max << shift;
        shift += w.width;
    }
    max
}

/// The width of a type: the product of its packed dimensions, or the width of a base type.
/// The width of a field of a packed struct signal (`p.r`), when the base is such a signal.
fn packed_field_width(base: &Expr, field: &str, env: &Env) -> Option<u32> {
    let (_, msb, lsb) = packed_struct_of(base, env)?.fields.into_iter().find(|(f, _, _)| f.as_str() == field)?;
    u32::try_from(msb - lsb + 1).ok()
}

/// The struct of a packed struct value: a signal of a packed struct type, or a field of one that is a packed struct.
pub fn packed_struct_of(e: &Expr, env: &Env) -> Option<StructInfo> {
    match e {
        // a signal of a packed struct type, or an array of them (its elements are the struct)
        Expr::Ident(signal, _) => {
            let (dtype, _) = env.signal(signal)?;
            env.typed_struct_named(&dtype.name)
        }
        Expr::Index(base, _) => packed_struct_of(base, env),
        Expr::Member(base, field, _) => {
            let outer = packed_struct_of(base, env)?;
            let key = outer.field_types.get(field)?;
            env.struct_info(key).filter(|s| s.typed)
        }
        _ => None,
    }
}

/// The bits of a signal of this type: a packed struct type's width, else the width of its declared dimensions.
pub fn signal_bits(dt: &DType, env: &Env) -> Option<u32> {
    env.typedef_width(&dt.name).or_else(|| dtype_width(dt))
}

pub fn dtype_width(dt: &DType) -> Option<u32> {
    let base = match dt.name.as_str() {
        "int" | "integer" => 32,
        "byte" => 8,
        "shortint" => 16,
        "longint" => 64,
        _ => 1,
    };
    if dt.packed.is_empty() {
        return Some(base);
    }
    let mut w: u32 = 1;
    for (msb, lsb) in &dt.packed {
        let (m, l) = (int_of(msb)?, int_of(lsb)?);
        w = w.checked_mul(u32::try_from((m - l).abs() + 1).ok()?)?;
    }
    Some(w)
}

/// A sized or unsized decimal, hex, octal or binary literal (not x, z or ?).
fn literal(text: &str) -> Option<Width> {
    let text = text.replace('_', "");
    let Some((size, rest)) = text.split_once('\'') else {
        let v: u128 = text.parse().ok()?;
        return Some(Width { width: bits(v), flex: true, max: v });
    };
    let rest = rest.trim_start_matches(['s', 'S']);
    let (radix, digits) = rest.split_at(1);
    // an unsized based literal (`'h3f`) is as wide as its value, like a decimal one
    let no_size = size.is_empty();
    let size: u32 = if no_size { 0 } else { size.parse().ok()? };
    let radix = match radix {
        "d" | "D" => 10,
        "h" | "H" => 16,
        "o" | "O" => 8,
        "b" | "B" => 2,
        _ => return None,
    };
    let v = u128::from_str_radix(digits.trim(), radix).ok()?;
    if no_size {
        return Some(Width { width: bits(v), flex: true, max: v });
    }
    Some(Width { width: size, flex: false, max: v.min(mask(size)) })
}

/// The width of an expression, or `None` when it cannot be determined.
pub fn width_of(e: &Expr, env: &Env) -> Option<Width> {
    match e {
        Expr::Num(text) => literal(text),
        // an enum member: the width of its enum
        Expr::Ident(name, _) if env.signal(name).is_none() && env.enum_member_type(name).is_some() => {
            Some(full(dtype_width(&env.enum_member_type(name)?)?))
        }
        // a field of a packed struct signal: the width of the field
        Expr::Member(base, field, _) if packed_field_width(base, field, env).is_some() => {
            Some(full(packed_field_width(base, field, env)?))
        }
        Expr::Ident(name, _) | Expr::Member(_, name, _) => {
            let (dtype, dims) = env.signal(name)?;
            if !dims.is_empty() {
                return None;
            }
            Some(full(signal_bits(&dtype, env)?))
        }
        Expr::Unary(op, arg) => match op.as_str() {
            "!" | "&" | "|" | "^" => Some(full(1)),
            _ => width_of(arg, env),
        },
        Expr::Binary(op, l, r, _) => binary(op, l, r, env),
        Expr::Ternary(_, a, b) => {
            let (a, b) = (width_of(a, env)?, width_of(b, env)?);
            if a.flex && b.flex {
                Some(Width { width: a.width.max(b.width), flex: true, max: a.max.max(b.max) })
            } else {
                Some(full(a.width.max(b.width)))
            }
        }
        Expr::Index(base, _) => {
            // an element of an array, or a bit of a packed vector; `a[i][j]` indexes the dimensions in turn, then the bits
            let (mut depth, mut root) = (1, &**base);
            while let Expr::Index(inner, _) = root {
                depth += 1;
                root = inner;
            }
            let Expr::Ident(name, _) = root else { return None };
            let (dtype, dims) = env.signal(name)?;
            if dims.is_empty() || depth > dims.len() {
                Some(full(1))
            } else {
                Some(full(signal_bits(&dtype, env)?))
            }
        }
        Expr::Slice(_, msb, lsb, op) => match op.as_str() {
            ":" => Some(full((int_of(msb)? - int_of(lsb)?).unsigned_abs() as u32 + 1)),
            _ => Some(full(u32::try_from(int_of(lsb)?).ok()?)),
        },
        Expr::Concat(parts) => {
            let ws: Vec<Width> = parts.iter().map(|p| width_of(p, env)).collect::<Option<_>>()?;
            if ws.iter().any(|w| w.flex) {
                return None;
            }
            let total: u32 = ws.iter().map(|w| w.width).sum();
            Some(Width { width: total, flex: false, max: packed_max(&ws) })
        }
        Expr::Repl(count, parts) => {
            let n = u32::try_from(int_of(count)?).ok()?;
            let ws: Vec<Width> = parts.iter().map(|p| width_of(p, env)).collect::<Option<_>>()?;
            if ws.iter().any(|w| w.flex) {
                return None;
            }
            let unit: u32 = ws.iter().map(|w| w.width).sum();
            let total = n * unit;
            if total > 127 {
                return Some(full(total));
            }
            // the unit repeated: {k{1'b0}} has a largest value of zero, so a zero-extended value keeps its own maximum
            let unit_max = packed_max(&ws);
            let mut max = 0u128;
            for _ in 0..n {
                max = (max << unit) | unit_max;
            }
            Some(Width { width: total, flex: false, max })
        }
        Expr::Cast(width, _) => Some(full(u32::try_from(int_of(width)?).ok()?)),
        // `$signed(x)` and `$unsigned(x)` are x's bits in another interpretation; `$clog2(x)` is an integer
        Expr::Call(func, args) => match (&**func, args.as_slice()) {
            (Expr::Ident(name, _), [arg]) if name == "$signed" || name == "$unsigned" => width_of(arg, env),
            (Expr::Ident(name, _), _) if name == "$clog2" => Some(full(32)),
            _ => None,
        },
        _ => None,
    }
}

fn binary(op: &str, l: &Expr, r: &Expr, env: &Env) -> Option<Width> {
    if matches!(op, "==" | "!=" | "===" | "!==" | "<" | "<=" | ">" | ">=" | "&&" | "||") {
        return Some(full(1));
    }
    // a shift's amount has no width of its own: only the value does
    if matches!(op, ">>" | ">>>" | "<<" | "<<<") && int_of(r).is_none() {
        let a = width_of(l, env)?;
        return (!a.flex && op.starts_with('>')).then(|| full(a.width));
    }
    let (a, b) = (width_of(l, env)?, width_of(r, env)?);
    let shift = || int_of(r).and_then(|n| u32::try_from(n).ok());
    match op {
        "+" => {
            let max = a.max.checked_add(b.max)?;
            Some(Width { width: bits(max), flex: false, max })
        }
        "*" => {
            let max = a.max.checked_mul(b.max)?;
            Some(Width { width: bits(max), flex: false, max })
        }
        "-" => Some(combine(&[a, b], 1, true)),
        "&" => Some(combine(&[a, b], 0, false)),
        "|" | "^" => Some(combine(&[a, b], 0, true)),
        "<<" | "<<<" => {
            let n = shift()?;
            if a.flex {
                return Some(full(a.width + n));
            }
            let max = a.max.checked_shl(n)?;
            Some(Width { width: bits(max), flex: false, max })
        }
        ">>" | ">>>" | "/" => {
            let Some(n) = shift() else {
                // a shift by a signal keeps the width of the value
                return (op != "/" && !a.flex).then(|| full(a.width));
            };
            if op == ">>" && n > 0 && !a.flex {
                let max = a.max >> n;
                Some(Width { width: bits(max), flex: false, max })
            } else {
                Some(full(a.width))
            }
        }
        "%" => Some(full(a.width)),
        _ => None,
    }
}

/// Operands of a bitwise or arithmetic operator: the widest one, plus `extra` bits. With `literals` false, an
/// unsized literal does not widen the result (`a & 'hff` keeps the width of `a`).
fn combine(ws: &[Width], extra: u32, literals: bool) -> Width {
    let fixed = ws.iter().filter(|w| !w.flex).map(|w| w.width).max();
    let base = match fixed {
        Some(f) if !literals => f,
        _ => ws.iter().map(|w| w.width).max().unwrap_or(1),
    };
    full(base + extra)
}
