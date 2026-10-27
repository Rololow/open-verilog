//! Constant folding of expressions, data types and statements under an environment.
//! Anything that depends on a signal is left as it is.

use super::env::{Env, FnDef};
use super::width::width_of;
use crate::ast::{Arg, Decl, DType, Declarator, Expr, ForHeader, Item, ItemKind, Pos, Sens, Stmt};

/// The value of a literal: decimal, or unsized based (`'h03`). A sized literal (`8'hff`) keeps its width, so it is
/// not folded.
pub fn int_of(e: &Expr) -> Option<i64> {
    match e {
        Expr::Num(text) if !text.contains('\'') || text.starts_with('\'') => crate::lexer::number_value(text),
        _ => None,
    }
}

/// The truth value of an operand of `&&` or `||`: 0 or 1, not the operand itself (`2 && x` is `x != 0`).
fn truth(e: Expr, env: &Env) -> Expr {
    if let Some(v) = int_of(&e) {
        return num((v != 0) as i64);
    }
    let logical = matches!(&e, Expr::Binary(op, ..) if matches!(op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||"))
        || matches!(&e, Expr::Unary(op, _) if op == "!");
    // a one-bit value is already 0 or 1
    if logical || width_of(&e, env).is_some_and(|w| w.width == 1) {
        e
    } else {
        Expr::Binary("!=".into(), boxed(e), boxed(num(0)), (0, 0))
    }
}

/// The most terms a fold may build (see `take_fold_overflow`).
const FOLD_TERMS: usize = 200_000;

thread_local! {
    /// Set when a fold grew past `FOLD_TERMS`: the module being elaborated is refused.
    static FOLD_OVERFLOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a fold of the module just elaborated grew past its term limit (and clears the mark).
pub(crate) fn take_fold_overflow() -> bool {
    FOLD_OVERFLOW.with(|f| f.replace(false))
}

/// The number of terms of an expression, as a tree (a shared sub-expression counts at each use).
fn term_count(e: &Expr) -> usize {
    let n = std::cell::Cell::new(0usize);
    let _ = crate::pipeline::rewrite::map_expr(e, &mut |_| {
        n.set(n.get() + 1);
        Ok(None)
    });
    n.get()
}

fn num(v: i64) -> Expr {
    Expr::Num(v.to_string())
}

fn boxed(e: Expr) -> Box<Expr> {
    Box::new(e)
}

/// The width and the value of a sized literal (`8'hA5`). An unsized literal has no width, so it is not one.
fn sized_literal(e: &Expr) -> Option<(i64, i64)> {
    let Expr::Num(text) = e else { return None };
    let (width, rest) = text.split_once('\'')?;
    let width: i64 = width.parse().ok()?;
    let value = crate::lexer::number_value(&format!("'{rest}"))?;
    (0..(1i64 << width.min(62))).contains(&value).then_some((width, value))
}

/// An expression whose bits can only be selected by shifting it: SV does not select from an expression.
pub(crate) fn is_expression(e: &Expr) -> bool {
    sized_literal(e).is_some()
        || (matches!(e, Expr::Binary(..) | Expr::Ternary(..) | Expr::Unary(..) | Expr::Concat(_) | Expr::Call(..) | Expr::Cast(..))
            && !has_list(e))
}

/// `base[start +: count]` of an expression: the expression is shifted down at its own width (so a carry is kept), then
/// cut to `count` bits.
fn part_of_expression(base: &Expr, start: Expr, count: i64, env: &Env) -> Expr {
    let w = super::width::width_of(base, env).map_or(32, |w| i64::from(w.width));
    let wide = Expr::Cast(boxed(num(w)), boxed(base.clone()));
    let shifted = Expr::Binary(">>".into(), boxed(wide), boxed(start), (0, 0));
    Expr::Cast(boxed(num(count)), boxed(shifted))
}

pub fn fold_expr(e: &Expr, env: &Env) -> Expr {
    match e {
        Expr::Num(_) | Expr::OpRef(_) | Expr::Type(_) => e.clone(),
        Expr::Ident(name, pos) => match env.int(name) {
            Some(v) => num(v),
            None if env.signal(name).is_none() && env.global_expr(name).is_some() => env.global_expr(name).unwrap_or_else(|| e.clone()),
            None if env.signal(name).is_none() => match env.list(name) {
                Some(items) => Expr::List(items),
                // a member of an enum, used bare (after `import Pkg::*`), unless a signal has that name
                None => match env.enum_of_member(name) {
                    Some(enum_name) => {
                        env.note_enum(&enum_name);
                        Expr::Ident(format!("{enum_name}_{name}"), *pos)
                    }
                    None => e.clone(),
                },
            },
            None => e.clone(),
        },
        Expr::Unary(op, a) => {
            let a = fold_expr(a, env);
            match int_of(&a).and_then(|v| unary(op, v)) {
                Some(r) => num(r),
                None => Expr::Unary(op.clone(), boxed(a)),
            }
        }
        Expr::Binary(op, l, r, pos) => {
            let (l, r) = (fold_expr(l, env), fold_expr(r, env));
            // a constant left side decides `&&` and `||` (the right side is then not evaluated: it may not exist)
            match (op.as_str(), int_of(&l)) {
                ("&&", Some(0)) => return num(0),
                ("||", Some(v)) if v != 0 => return num(1),
                ("&&", Some(_)) | ("||", Some(0)) => return truth(r, env),
                _ => {}
            }
            // a constant right side decides them too: `x && 1` is `x != 0`, `x || 0` is `x != 0`, `x && 0` is 0, `x || 1` is 1
            match (op.as_str(), int_of(&r)) {
                ("&&", Some(v)) if v != 0 => return truth(l, env),
                ("||", Some(0)) => return truth(l, env),
                ("&&", Some(0)) => return num(0),
                ("||", Some(v)) if v != 0 => return num(1),
                _ => {}
            }
            // two strings compared: a constant
            if let (Expr::Num(a), Expr::Num(b)) = (&l, &r) {
                if a.starts_with('"') && b.starts_with('"') {
                    match op.as_str() {
                        "==" => return num(i64::from(a == b)),
                        "!=" => return num(i64::from(a != b)),
                        _ => {}
                    }
                }
            }
            // a constant table joined to another: `A ++ B`
            if let (Expr::List(a), "++", Expr::List(b)) = (&l, op.as_str(), &r) {
                return Expr::List(a.iter().chain(b).cloned().collect());
            }
            match int_of(&l).zip(int_of(&r)).and_then(|(a, b)| binary(op, a, b)) {
                Some(v) => num(v),
                None => Expr::Binary(op.clone(), boxed(l), boxed(r), *pos),
            }
        }
        Expr::Ternary(c, a, b) => {
            let c = fold_expr(c, env);
            match int_of(&c) {
                Some(v) => fold_expr(if v != 0 { a } else { b }, env),
                None => Expr::Ternary(boxed(c), boxed(fold_expr(a, env)), boxed(fold_expr(b, env))),
            }
        }
        Expr::Index(base, idx) => {
            // a constant list read at a signal position is a constant table of the module (`RCP_LUT[ma]`): an array
            // of constants, not a chain of conditionals. Its elements must all be literals.
            if let Expr::Ident(name, pos) = &**base {
                if env.signal(name).is_none() && int_of(&fold_expr(idx, env)).is_none() {
                    if let Some(items) = env.list(name) {
                        let values: Option<Vec<i64>> = items.iter().map(|x| match x {
                            Expr::Num(t) => crate::lexer::number_value(t),
                            _ => None,
                        }).collect();
                        if let Some(values) = values.filter(|v| !v.is_empty() && v.iter().all(|x| *x >= 0)) {
                            let width = values.iter().map(|v| 64 - (*v as u64).leading_zeros()).max().unwrap_or(1).max(1);
                            if env.declare_table(name, width, items.clone()) {
                                return Expr::Index(Box::new(Expr::Ident(name.clone(), *pos)), Box::new(fold_expr(idx, env)));
                            }
                        }
                    }
                }
            }
            // a field of the elements of a vector of bundles: `v.f[i]` is the field of element `i` (a mux over the
            // elements when `i` is a signal)
            if let Expr::Member(inner, field, pos) = &**base {
                if super::bundle::rooted_in_vector(inner, env) {
                    if let Some(prefixes) = super::bundle::bundle_prefixes(inner, env).filter(|p| !p.is_empty()) {
                        let element = |p: &String| Expr::Ident(format!("{p}_{field}"), *pos);
                        let count = prefixes.len();
                        return match int_of(&fold_expr(idx, env)) {
                            Some(k) => element(&prefixes[usize::try_from(k).unwrap_or(0).min(count - 1)]),
                            None => {
                                let mut acc = element(&prefixes[count - 1]);
                                for (k, p) in prefixes.iter().enumerate().take(count - 1).rev() {
                                    let hit = Expr::Binary("==".into(), boxed(fold_expr(idx, env)), boxed(num(k as i64)), (0, 0));
                                    acc = Expr::Ternary(boxed(hit), boxed(element(p)), boxed(acc));
                                }
                                acc
                            }
                        };
                    }
                }
            }
            // an element of an array field of a view: its bits, at a constant or a variable position
            if let Expr::Member(vb, field, _) = &**base {
                if let Expr::Ident(v, _) = &**vb {
                    if let Some((signal, isa, format)) = env.view(v) {
                        if let Some(info) = env.isa_info(&isa) {
                            if let Some(f) = super::isa::field_bits(info, &format, field).filter(|f| f.count.is_some()) {
                                let i = fold_expr(idx, env);
                                let w = f.width;
                                return match int_of(&i) {
                                    Some(k) => {
                                        let lo = f.start + k * w;
                                        Expr::Slice(Box::new(Expr::Ident(signal, (0, 0))), Box::new(num(lo + w - 1)), Box::new(num(lo)), ":".into())
                                    }
                                    None => {
                                        let start = Expr::Binary("+".into(), Box::new(num(f.start)), Box::new(Expr::Binary("*".into(), Box::new(i), Box::new(num(w)), (0, 0))), (0, 0));
                                        Expr::Slice(Box::new(Expr::Ident(signal, (0, 0))), Box::new(start), Box::new(num(w)), "+:".into())
                                    }
                                };
                            }
                        }
                    }
                }
            }
            let (base, idx) = (fold_expr(base, env), fold_expr(idx, env));
            // an element of a constant list, at a constant position
            if let (Expr::List(items), Some(k)) = (&base, int_of(&idx)) {
                if let Some(item) = usize::try_from(k).ok().and_then(|k| items.get(k)) {
                    return item.clone();
                }
            }
            // a part of a choice between tables: the choice of the parts (`(c ? A : B)[k]` is `c ? A[k] : B[k]`)
            if let (Expr::Ternary(c, a, b), Some(_)) = (&base, int_of(&idx)) {
                if matches!((&**a, &**b), (Expr::List(_), Expr::List(_)) | (Expr::Ternary(..), _)) || matches!(fold_expr(a, env), Expr::List(_)) {
                    let pick = |x: &Expr| Expr::Index(boxed(x.clone()), boxed(idx.clone()));
                    return fold_expr(&Expr::Ternary(c.clone(), boxed(pick(a)), boxed(pick(b))), env);
                }
            }
            // a constant table read at a variable position: a multiplexer over its elements (the last one for the
            // positions past the end)
            if let (Expr::List(items), None) = (&base, int_of(&idx)) {
                if let Some((last, rest)) = items.split_last() {
                    let mut acc = last.clone();
                    for (k, item) in rest.iter().enumerate().rev() {
                        let hit = Expr::Binary("==".into(), boxed(idx.clone()), boxed(num(k as i64)), (0, 0));
                        acc = Expr::Ternary(boxed(hit), boxed(item.clone()), boxed(acc));
                    }
                    return fold_expr(&acc, env);
                }
            }
            // a bit of a sized literal: a literal of one bit
            if let (Some(i), Some((_, v))) = (int_of(&idx), sized_literal(&base)) {
                return Expr::Num(format!("1'h{}", (v >> i) & 1));
            }
            if let Some(i) = int_of(&idx) {
                match &base {
                    // a bit of a part: the bit of the inner vector
                    Expr::Slice(inner, _, ilsb, iop) if iop.as_str() == ":" => {
                        if let Some(il) = int_of(ilsb) {
                            return Expr::Index(inner.clone(), boxed(num(il + i)));
                        }
                    }
                    // a bit of an expression: the expression shifted down, then one bit
                    _ if is_expression(&base) => return part_of_expression(&base, num(i), 1, env),
                    _ => {}
                }
            }
            // a bit of an expression at a signal position: SV cannot select from an expression, so it is shifted too
            if is_expression(&base) {
                return part_of_expression(&base, idx, 1, env);
            }
            Expr::Index(boxed(base), boxed(idx))
        }
        Expr::Slice(base, msb, lsb, op) => {
            let (base, msb, lsb) = (fold_expr(base, env), fold_expr(msb, env), fold_expr(lsb, env));
            if op.as_str() == ":" {
                if let (Some(m), Some(l)) = (int_of(&msb), int_of(&lsb)) {
                    // a part of a constant is a constant
                    if let Some(v) = int_of(&base).filter(|_| m >= l && m - l < 62) {
                        return num((v >> l) & ((1i64 << (m - l + 1)) - 1));
                    }
                    // a part of a sized literal: a literal of the part's width
                    if let (Some((_, v)), true) = (sized_literal(&base), m >= l && m - l < 62) {
                        return Expr::Num(format!("{}'h{:x}", m - l + 1, (v >> l) & ((1i64 << (m - l + 1)) - 1)));
                    }
                    match &base {
                        // a part of a part: the bits of the inner vector
                        Expr::Slice(inner, _, ilsb, iop) if iop.as_str() == ":" => {
                            if let Some(il) = int_of(ilsb) {
                                return Expr::Slice(inner.clone(), boxed(num(il + m)), boxed(num(il + l)), ":".into());
                            }
                        }
                        // a part of an expression: the expression shifted down, then cut to size
                        _ if m >= l && is_expression(&base) => return part_of_expression(&base, num(l), m - l + 1, env),
                        _ => {}
                    }
                }
            }
            // `v[start +: count]` of an expression: the same, from a start that may be a signal
            if op.as_str() == "+:" && is_expression(&base) {
                if let Some(count) = int_of(&lsb) {
                    return part_of_expression(&base, msb, count, env);
                }
            }
            Expr::Slice(boxed(base), boxed(msb), boxed(lsb), op.clone())
        }
        Expr::Member(base, name, pos) => {
            // a value of a manifest config: `Manifest.entry`
            if let Expr::Ident(v, _) = &**base {
                if let Some(value) = env.global_expr(&format!("{v}.{name}")) {
                    return value;
                }
            }
            // the opcode of an instruction of an instruction set: `Isa.NAME.opcode`
            if let (Expr::Member(inner, instr, _), "opcode") = (&**base, name.as_str()) {
                if let Expr::Ident(isa, _) = &**inner {
                    if let Some(code) = env.isa_info(isa).and_then(|i| i.codes.get(instr)) {
                        return num(*code);
                    }
                }
            }
            // a member of a struct field of an element of a vector of bundles (`m.r.data[k].id`): the member of the
            // element's signal, a multiplexer over the elements when `k` is a signal
            if let Expr::Index(elem, idx) = &**base {
                if let Expr::Member(vec, field, _) = &**elem {
                    if super::bundle::rooted_in_vector(vec, env) {
                        if let Some(prefixes) = super::bundle::bundle_prefixes(vec, env).filter(|p| !p.is_empty()) {
                            let part = |p: &String| -> Option<Expr> {
                                let sig = format!("{p}_{field}");
                                let s = env.struct_of(&sig)?;
                                let (_, msb, lsb) = s.fields.iter().find(|(f, _, _)| f == name)?.clone();
                                Some(Expr::Slice(boxed(Expr::Ident(sig, *pos)), boxed(num(msb)), boxed(num(lsb)), ":".into()))
                            };
                            if let Some(parts) = prefixes.iter().map(part).collect::<Option<Vec<Expr>>>() {
                                let count = parts.len();
                                return match int_of(&fold_expr(idx, env)) {
                                    Some(k) => parts[usize::try_from(k).unwrap_or(0).min(count - 1)].clone(),
                                    None => {
                                        let mut acc = parts[count - 1].clone();
                                        for (k, part) in parts.iter().enumerate().take(count - 1).rev() {
                                            let hit = Expr::Binary("==".into(), boxed(fold_expr(idx, env)), boxed(num(k as i64)), (0, 0));
                                            acc = Expr::Ternary(boxed(hit), boxed(part.clone()), boxed(acc));
                                        }
                                        acc
                                    }
                                };
                            }
                        }
                    }
                }
            }
            // a leaf field of a whole vector of bundles (`db.resp.valid`): the field of every element, the last element
            // first (the most significant)
            if matches!(&**base, Expr::Ident(..) | Expr::Member(..)) && super::bundle::rooted_in_vector(base, env) && no_index(base) {
                if let Some(prefixes) = super::bundle::bundle_prefixes(base, env).filter(|p| !p.is_empty()) {
                    if prefixes.iter().all(|p| env.bundle_of(&format!("{p}_{name}")).is_none()) {
                        return Expr::Concat(prefixes.iter().rev().map(|p| Expr::Ident(format!("{p}_{name}"), *pos)).collect());
                    }
                }
            }
            // an element of a vector of bundles: `v[k].field`
            if let Expr::Index(vec, idx) = &**base {
                if let (Expr::Ident(v, _), Some(k)) = (&**vec, int_of(&fold_expr(idx, env))) {
                    if let Some(b) = env.bundle_of(&format!("{v}_{k}")).filter(|b| b.fields.iter().any(|(f, _)| f == name)) {
                        return Expr::Ident(format!("{}_{name}", b.prefix), *pos);
                    }
                }
                // with an index that is a signal: the field of the vector, indexed (`consumer_read_ready[cur]`)
                if let (Expr::Ident(v, _), None) = (&**vec, int_of(&fold_expr(idx, env))) {
                    let has_field = env.bundle_of(&format!("{v}_0")).is_some_and(|b| b.fields.iter().any(|(f, _)| f == name));
                    if has_field {
                        return Expr::Index(Box::new(Expr::Ident(format!("{v}_{name}"), *pos)), Box::new(fold_expr(idx, env)));
                    }
                }
            }
            // a memory map: `io.DEV` (the device's index), `io.count`, `io.regions`, `io.irqs`, `io.DEV.base`
            if let Expr::Member(inner, dev, _) = &**base {
                if let Expr::Ident(map, _) = &**inner {
                    if let Some(v) = env.memmap_field(map, dev, name) {
                        return v;
                    }
                }
            }
            if let Expr::Ident(map, _) = &**base {
                if let Some(v) = env.memmap_member(map, name) {
                    return v;
                }
            }
            if let Expr::Ident(v, _) = &**base {
                if let Some((signal, isa, format)) = env.view(v) {
                    if let Some(info) = env.isa_info(&isa) {
                        if name == "opcode" {
                            return Expr::Slice(Box::new(Expr::Ident(signal, *pos)), Box::new(num(info.opcode_bits - 1)), Box::new(num(0)), ":".into());
                        }
                        if let Some(f) = super::isa::field_bits(info, &format, name).filter(|f| f.count.is_none()) {
                            let (msb, lsb) = (f.start + f.width - 1, f.start);
                            return Expr::Slice(Box::new(Expr::Ident(signal, *pos)), Box::new(num(msb)), Box::new(num(lsb)), ":".into());
                        }
                    }
                }
                if let Some(info) = env.isa_info(v) {
                    if name == "lengths" {
                        env.note_isa(v);
                        let _ = info;
                        return Expr::Ident(format!("{v}_lengths"), *pos);
                    }
                }
            }
            if let Expr::Ident(signal, _) = &**base {
                if let Some(v) = env.config_field(signal, name) {
                    return v;
                }
                // a section or entry of the manifest of a config parameter: `C.tables` (a record, read by its keys)
                if let Some(arg) = env.config_arg(signal) {
                    if let Some(v) = env.global_expr(&format!("{}.{name}", arg.name)) {
                        return v;
                    }
                }
                if let Some(b) = env.bundle_of(signal) {
                    if b.fields.iter().any(|(f, _)| f == name) {
                        return Expr::Ident(format!("{}_{name}", b.prefix), *pos);
                    }
                }
                if let Some(s) = env.struct_of(signal) {
                    if let Some((_, msb, lsb)) = s.fields.iter().find(|(f, _, _)| f == name) {
                        // a signal of a packed struct type names its field; a bit view of a vector is sliced
                        if env.signal(signal).is_some_and(|(dt, _)| env.typedef_width(&dt.name).is_some()) {
                            return Expr::Member(boxed(Expr::Ident(signal.clone(), *pos)), name.clone(), *pos);
                        }
                        return Expr::Slice(Box::new(Expr::Ident(signal.clone(), *pos)), Box::new(num(*msb)), Box::new(num(*lsb)), ":".into());
                    }
                }
            }
            if let Expr::Ident(enum_name, _) = &**base {
                if env.enum_info(enum_name).is_some_and(|info| info.members.iter().any(|(m, _)| m == name)) {
                    env.note_enum(enum_name);
                    return Expr::Ident(format!("{enum_name}_{name}"), *pos);
                }
            }
            // the base may fold to a signal (a nested bundle's signal): its member is then looked up again
            let folded = fold_expr(base, env);
            // a field of a record (a row of a manifest)
            if let Expr::Pattern(items) = &folded {
                if let Some(value) = items.iter().find_map(|i| match i {
                    Expr::Keyed(k, v) if k == name => Some((**v).clone()),
                    _ => None,
                }) {
                    return value;
                }
            }
            if matches!(folded, Expr::Ident(..)) && !matches!(&**base, Expr::Ident(..)) {
                return fold_expr(&Expr::Member(boxed(folded), name.clone(), *pos), env);
            }
            Expr::Member(boxed(folded), name.clone(), *pos)
        }
        // `.wrap()` marks an intentional truncation; the target width already implies it
        // `.wrap()` truncates the result to the width it is assigned to; the arithmetic under it is computed exactly
        // first (SV would size an operand by itself and lose its carry: `((a * b) >> s).wrap()` needs `W'(a * b)`)
        Expr::Call(func, args) if args.is_empty() && matches!(&**func, Expr::Member(_, m, _) if m == "wrap") => match &**func {
            Expr::Member(base, _, _) => exact_operands(&fold_expr(base, env), env),
            _ => unreachable!(),
        },
        Expr::Call(func, args) => {
            let args: Vec<Expr> = args.iter().map(|a| fold_expr(a, env)).collect();
            if let Expr::Member(base, method, pos) = &**func {
                // a view of bits as a struct (`x.as(Pte)`): the same bits, the struct is named by the signal it is given to
                if method == "as" && matches!(args.as_slice(), [Expr::Ident(n, _)] if env.struct_info(n).is_some()) || method == "as" && matches!(args.as_slice(), [Expr::Type(dt)] if struct_key(dt, env).is_some()) {
                    return fold_expr(base, env);
                }
                // the receiver may be a nested bundle's signal (`b.aw.fire()`): it is folded to that signal first
                let receiver = if matches!(&**base, Expr::Member(..)) { fold_expr(base, env) } else { (**base).clone() };
                // a method of a struct-typed signal: its fields are slices of the receiver
                if let Expr::Ident(recv, _) = &receiver {
                    if let Some(s) = env.struct_of(recv) {
                        if let Some(def) = s.methods.get(method).filter(|d| d.params.len() == args.len()) {
                            // a packed struct signal: its fields are members of it; a bit view: slices of it
                            let packed = env.signal(recv).is_some_and(|(dt, _)| env.typedef_width(&dt.name).is_some());
                            let fields: Vec<(String, Expr)> = s
                                .fields
                                .iter()
                                .map(|(f, msb, lsb)| {
                                    let field = if packed {
                                        Expr::Member(Box::new(Expr::Ident(recv.clone(), (0, 0))), f.clone(), (0, 0))
                                    } else {
                                        Expr::Slice(Box::new(Expr::Ident(recv.clone(), (0, 0))), Box::new(num(*msb)), Box::new(num(*lsb)), ":".into())
                                    };
                                    (f.clone(), field)
                                })
                                .collect();
                            let names: Vec<String> = s.methods.keys().cloned().collect();
                            let bound = FnDef { params: def.params.clone(), ret: def.ret.clone(), body: rebind(&def.body, &def.params, recv, &fields, &names) };
                            return inline(method, &bound, &args, env);
                        }
                    }
                    // a method of a bundle signal: its signals are the flattened ports
                    if let Some(b) = env.bundle_of(recv) {
                        let def_bundle = env.bundle_def(&b.bundle);
                        let def = def_bundle.and_then(|d| {
                            d.methods.iter().find_map(|m| match m {
                                ItemKind::Fn { name, params, ret, body, .. } if name == method && params.len() == args.len() => {
                                    Some(FnDef { params: params.clone(), ret: ret.clone(), body: body.clone() })
                                }
                                _ => None,
                            })
                        });
                        if let Some(def) = def {
                            let fields: Vec<(String, Expr)> =
                                b.fields.iter().map(|(f, _)| (f.clone(), Expr::Ident(format!("{}_{f}", b.prefix), (0, 0)))).collect();
                            let names: Vec<String> = def_bundle
                                .map(|d| d.methods.iter().filter_map(|m| match m {
                                    ItemKind::Fn { name, .. } => Some(name.clone()),
                                    _ => None,
                                }).collect())
                                .unwrap_or_default();
                            let bound = FnDef { params: def.params.clone(), ret: def.ret.clone(), body: rebind(&def.body, &def.params, recv, &fields, &names) };
                            return inline(method, &bound, &args, env);
                        }
                    }
                }
                if let Some(e) = list_method(&fold_expr(base, env), method, &args, env, *pos) {
                    return e;
                }
            }
            if let Expr::Ident(name, _) = &**func {
                // a signal of the module hides a function of the same name (the port wins)
                if env.signal(name).is_none() {
                    if let Some(def) = env.fn_def(name).filter(|d| d.params.len() == args.len()) {
                        return inline(name, def, &args, env);
                    }
                }
            }
            let values: Option<Vec<i64>> = args.iter().map(int_of).collect();
            if let (Expr::Ident(name, _), Some(values)) = (&**func, values) {
                if let Some(v) = builtin(name, &values) {
                    return num(v);
                }
            }
            Expr::Call(func.clone(), args)
        }
        // a sum or product inside a concatenation has the width of its lossless value: an unsized constant in it
        // would widen the part to 32 bits and shift every field after it
        Expr::Concat(parts) => Expr::Concat(
            parts
                .iter()
                .map(|p| {
                    let f = fold_expr(p, env);
                    // a truncated sum in a concatenation keeps the width of its widest sized operand
                    let truncated = matches!(p, Expr::Call(func, args) if args.is_empty() && matches!(&**func, Expr::Member(_, m, _) if m == "wrap"));
                    if let (true, Expr::Binary(op, ..), Some(bits)) = (truncated, &f, wrap_width(&f, env)) {
                        if matches!(op.as_str(), "+" | "-" | "*") {
                            return Expr::Cast(boxed(num(i64::from(bits))), boxed(f));
                        }
                    }
                    match (&f, width_of(&f, env)) {
                        (Expr::Binary(op, ..), Some(w)) if matches!(op.as_str(), "+" | "-" | "*" | "<<") => {
                            // a difference is as wide as its operands (the borrow stays inside them)
                            let bits = if op.as_str() == "-" { w.width - 1 } else { w.width };
                            Expr::Cast(boxed(num(i64::from(bits))), boxed(f))
                        }
                        _ => f,
                    }
                })
                .collect(),
        ),
        Expr::Repl(count, parts) => Expr::Repl(boxed(fold_expr(count, env)), parts.iter().map(|p| fold_expr(p, env)).collect()),
        Expr::Cast(width, arg) => Expr::Cast(boxed(fold_expr(width, env)), boxed(fold_expr(arg, env))),
        Expr::Lambda(params, body) => Expr::Lambda(params.clone(), boxed(fold_expr(body, env))),
        Expr::List(items) => Expr::List(items.iter().map(|i| fold_expr(i, env)).collect()),
        Expr::Pattern(items) => Expr::Pattern(items.iter().map(|i| fold_expr(i, env)).collect()),
        Expr::Keyed(k, v) => Expr::Keyed(k.clone(), boxed(fold_expr(v, env))),
        Expr::With(base, overrides, pos) => Expr::With(
            boxed(fold_expr(base, env)),
            overrides.iter().map(|(n, v)| (n.clone(), fold_expr(v, env))).collect(),
            *pos,
        ),
        Expr::InSet(x, set) => in_set_lowered(&fold_expr(x, env), set, env),
        Expr::Match(subject, arms) => {
            // a chain of conditionals in the order of the arms; the `default` arm is the last `else`
            let subject = fold_expr(subject, env);
            let Some((_, default)) = arms.iter().find(|(labels, _)| labels.is_none()) else {
                return Expr::Match(boxed(subject), arms.clone());
            };
            let mut acc = fold_expr(default, env);
            for (labels, value) in arms.iter().rev() {
                if let Some(set) = labels {
                    // folded: a constant subject picks its arm at compile time (a list stays a list)
                    let cond = fold_expr(&in_set_lowered(&subject, set, env), env);
                    acc = Expr::Ternary(boxed(cond), boxed(fold_expr(value, env)), boxed(acc));
                }
            }
            // folded again: a constant condition takes its arm
            fold_expr(&acc, env)
        }
    }
}

/// The width of a truncated sum or product (`(a + 1).wrap()`): its widest sized operand; an unsized literal does not
/// count.
fn wrap_width(e: &Expr, env: &Env) -> Option<u32> {
    match e {
        Expr::Binary(op, l, r, _) if matches!(op.as_str(), "+" | "-" | "*") => [wrap_width(l, env), wrap_width(r, env)].into_iter().flatten().max(),
        Expr::Unary(op, arg) if matches!(op.as_str(), "-" | "~") => wrap_width(arg, env),
        Expr::Num(text) if !text.contains('\'') => None,
        _ => width_of(e, env).map(|w| w.width),
    }
}

/// `x in {a, lo..hi}` as a condition: `x == a`, or `x >= lo && x <= hi` for a range, joined by `||`.
fn in_set_lowered(x: &Expr, set: &[crate::ast::SetItem], env: &Env) -> Expr {
    let terms = set.iter().map(|s| {
        let lo = fold_expr(&s.lo, env);
        match &s.hi {
            None => Expr::Binary("==".into(), boxed(x.clone()), boxed(lo), (0, 0)),
            Some(hi) => {
                let hi = fold_expr(hi, env);
                let above = Expr::Binary(">=".into(), boxed(x.clone()), boxed(lo), (0, 0));
                // `lo..hi` excludes `hi`, as in the spec (and as in `for i in lo..hi`)
                let below = Expr::Binary("<".into(), boxed(x.clone()), boxed(hi), (0, 0));
                Expr::Binary("&&".into(), boxed(above), boxed(below), (0, 0))
            }
        }
    });
    terms
        .reduce(|a, b| Expr::Binary("||".into(), boxed(a), boxed(b), (0, 0)))
        .unwrap_or_else(|| Expr::Num("0".into()))
}

/// Whether an expression still holds a constant list (it is not folded all the way yet).
fn has_list(e: &Expr) -> bool {
    let mut found = false;
    let _ = crate::pipeline::rewrite::map_expr(e, &mut |x| {
        if matches!(x, Expr::List(_)) {
            found = true;
        }
        Ok(None)
    });
    found
}

/// A shift of arithmetic: its value is shifted at the exact width of that arithmetic (SV would cut a product to the
/// shift's own width first, and lose the bits the shift brings down). Other expressions are sized by their context.
fn exact_operands(e: &Expr, env: &Env) -> Expr {
    match e {
        Expr::Binary(op, l, r, pos) if matches!(op.as_str(), ">>" | ">>>") => {
            Expr::Binary(op.clone(), boxed(exact_operand(*l.clone(), env)), r.clone(), *pos)
        }
        // a signed operand of a sum or difference is widened to the sum's width: SV would make the sum unsigned and
        // extend the signed operand with zeros (`acc + $signed(a) * $signed(b)`)
        Expr::Binary(op, l, r, pos) if matches!(op.as_str(), "+" | "-") => {
            let Some(bits) = wrap_width(e, env) else { return e.clone() };
            let widen = |x: &Expr| if has_signed(x) { Expr::Cast(boxed(num(i64::from(bits))), boxed(x.clone())) } else { x.clone() };
            Expr::Binary(op.clone(), boxed(widen(l)), boxed(widen(r)), *pos)
        }
        other => other.clone(),
    }
}

/// Whether an expression has a `$signed(...)` operand in it.
fn has_signed(e: &Expr) -> bool {
    match e {
        Expr::Call(func, _) => matches!(&**func, Expr::Ident(name, _) if name == "$signed"),
        Expr::Binary(_, l, r, _) => has_signed(l) || has_signed(r),
        Expr::Unary(_, a) => has_signed(a),
        _ => false,
    }
}

/// `x` cast to its lossless width when it is arithmetic that SV would size by its operands.
fn exact_operand(x: Expr, env: &Env) -> Expr {
    let Expr::Binary(op, l, r, _) = &x else { return x };
    if !matches!(op.as_str(), "*" | "+" | "-" | "<<") {
        return x;
    }
    let (Some(wl), Some(wr)) = (width_of(l, env), width_of(r, env)) else { return x };
    let sv = wl.width.max(wr.width);
    // the exact width: `+` and `*` hold their largest value (from the operands' largest values, not their declared
    // widths: a zero-extended operand is as wide as its value); `-` is one bit wider than its operands
    let exact = match op.as_str() {
        "*" | "+" => {
            let Some(w) = width_of(&x, env) else { return x };
            w.width
        }
        "-" => sv + 1,
        _ => return x,
    };
    if exact > sv {
        Expr::Cast(boxed(num(i64::from(exact))), boxed(x))
    } else {
        x
    }
}

/// Whether an expression names its signal without an index on the way (`v`, `v.f`).
fn no_index(e: &Expr) -> bool {
    match e {
        Expr::Ident(..) => true,
        Expr::Member(base, _, _) => no_index(base),
        _ => false,
    }
}

/// A method body with its receiver bound: the struct's fields become slices of `recv`, and the sibling methods it
/// calls become calls on `recv`.
fn rebind(e: &Expr, params: &[String], recv: &str, fields: &[(String, Expr)], methods: &[String]) -> Expr {
    crate::pipeline::rewrite::map_expr(e, &mut |x| {
        Ok(match x {
            Expr::Ident(n, _) if !params.contains(n) => fields.iter().find(|(f, _)| f == n).map(|(_, e)| e.clone()),
            Expr::Call(f, args) => match &**f {
                Expr::Ident(m, _) if methods.contains(m) => Some(Expr::Call(
                    Box::new(Expr::Member(Box::new(Expr::Ident(recv.to_string(), (0, 0))), m.clone(), (0, 0))),
                    args.iter().map(|a| rebind(a, params, recv, fields, methods)).collect(),
                )),
                _ => None,
            },
            _ => None,
        })
    })
    .unwrap_or_else(|_| e.clone())
}

/// `e` with the names `params` replaced by `args`.
pub(crate) fn substitute(e: &Expr, params: &[String], args: &[Expr]) -> Expr {
    crate::pipeline::rewrite::map_expr(e, &mut |node| {
        Ok(match node {
            Expr::Ident(n, _) => params.iter().position(|p| p == n).map(|i| args[i].clone()),
            _ => None,
        })
    })
    .unwrap_or_else(|_| e.clone())
}

/// The list methods: `v.list()` of an array, `(lo..hi).map(f)`, `.reduce(op)`, `.tree_reduce(op)`, and
/// The name a delay line is built on: the signal it delays, with the constant indices and members of its access
/// (`a[2]` is `a_2`, `s.hi` is `s_hi`).
fn delay_root(base: &Expr, env: &Env) -> String {
    match base {
        Expr::Ident(name, _) => name.clone(),
        Expr::Index(inner, idx) => match int_of(&fold_expr(idx, env)) {
            Some(k) => format!("{}_{k}", delay_root(inner, env)),
            None => delay_root(inner, env),
        },
        Expr::Member(inner, field, _) => format!("{}_{field}", delay_root(inner, env)),
        Expr::Slice(inner, msb, lsb, op) if op.as_str() == ":" => match (int_of(&fold_expr(msb, env)), int_of(&fold_expr(lsb, env))) {
            (Some(m), Some(l)) => format!("{}_{m}_{l}", delay_root(inner, env)),
            _ => delay_root(inner, env),
        },
        _ => "dly".into(),
    }
}

/// `v.reverse()` of a packed vector. `None` when the call is not one of these on a known shape.
fn list_method(base: &Expr, method: &str, args: &[Expr], env: &Env, pos: Pos) -> Option<Expr> {
    let items: Option<Vec<Expr>> = match base {
        Expr::List(items) => Some(items.clone()),
        Expr::Binary(op, lo, hi, _) if op == ".." => {
            let (lo, hi) = (int_of(lo)?, int_of(hi)?);
            Some((lo..hi).map(num).collect())
        }
        _ => None,
    };
    match (method, args, items, base) {
        ("list", [], None, Expr::Ident(name, pos)) => match env.signal(name) {
            Some((_, dims)) => {
                let [n] = dims.as_slice() else { return None };
                let n = int_of(n)?;
                Some(Expr::List((0..n).map(|i| Expr::Index(Box::new(base.clone()), Box::new(num(i)))).collect()))
            }
            // a vector of bundles: its elements are the bundles `name_0`, `name_1`, ...
            None => {
                let count = (0..).take_while(|k| env.bundle_of(&format!("{name}_{k}")).is_some()).count();
                (count > 0).then(|| Expr::List((0..count).map(|k| Expr::Ident(format!("{name}_{k}"), *pos)).collect()))
            }
        },
        ("len", [], Some(items), _) => Some(num(items.len() as i64)),
        // the bits of a list of signals, first element most significant: `[a, b].cat()` is `{a, b}`
        ("cat", [], Some(items), _) if !items.is_empty() => Some(Expr::Concat(items)),
        // `list.filter(x => cond)`: the elements for which the condition (a constant) holds
        ("filter", [Expr::Lambda(params, body)], Some(items), _) => {
            let [param] = params.as_slice() else { return None };
            let mut kept = Vec::new();
            for item in items {
                let keep = fold_expr(&substitute(body, std::slice::from_ref(param), std::slice::from_ref(&item)), env);
                if int_of(&keep)? != 0 {
                    kept.push(item);
                }
            }
            Some(Expr::List(kept))
        }
        ("reverse", [], Some(items), _) => Some(Expr::List(items.iter().rev().cloned().collect())),
        // `list.enumerate()` pairs each element with its position: `[i, x]`, as the reference does
        ("enumerate", [], Some(items), _) => Some(Expr::List(
            items.iter().enumerate().map(|(k, x)| Expr::List(vec![num(k as i64), x.clone()])).collect(),
        )),
        // `pairs.select(k, default)`: the value of the pair whose key is `k`, or `default` when there is none
        ("select", [key, default], Some(pairs), _) if !pairs.is_empty() => {
            let mut acc = default.clone();
            for pair in pairs.iter().rev() {
                let Expr::List(kv) = pair else { return None };
                let [k, value] = kv.as_slice() else { return None };
                let hit = Expr::Binary("==".into(), Box::new(key.clone()), Box::new(k.clone()), pos);
                acc = Expr::Ternary(Box::new(hit), Box::new(value.clone()), Box::new(acc));
            }
            Some(fold_expr(&acc, env))
        }
        // `list.fold(init, (acc, x) => e)`: the elements combined in order, from `init`
        ("fold", [init, Expr::Lambda(params, body)], Some(items), _) => {
            let [acc_name, x_name] = params.as_slice() else { return None };
            let mut acc = init.clone();
            for item in items {
                let args = [acc, item.clone()];
                acc = fold_expr(&substitute(body, &[acc_name.clone(), x_name.clone()], &args), env);
                // a value that uses its accumulator twice doubles at each step: refused once it is too big
                if term_count(&acc) > FOLD_TERMS {
                    FOLD_OVERFLOW.with(|f| f.set(true));
                    break;
                }
            }
            Some(acc)
        }
        ("map", [Expr::Lambda(params, body)], Some(items), _) => {
            let [param] = params.as_slice() else { return None };
            let mapped = items.iter().map(|x| fold_expr(&substitute(body, std::slice::from_ref(param), std::slice::from_ref(x)), env)).collect();
            Some(Expr::List(mapped))
        }
        // a function combines two values: `xs.reduce(f)`, `xs.tree_reduce(f)`. A tree's intermediate values are
        // named wires, so a body that uses its operands twice does not write them out twice at each level.
        ("reduce" | "tree_reduce", [Expr::Ident(fname, _)], Some(items), _) if !items.is_empty() && env.fn_def(fname).is_some_and(|d| d.params.len() == 2) => {
            let def = env.fn_def(fname)?.clone();
            if method == "reduce" {
                return items.into_iter().reduce(|a, b| inline(fname, &def, &[a, b], env));
            }
            let mut level = items;
            while level.len() > 1 {
                let mut next: Vec<Expr> = level.chunks(2).map(|pair| match pair {
                    [a, b] => inline(fname, &def, &[a.clone(), b.clone()], env),
                    [only] => only.clone(),
                    _ => unreachable!(),
                }).collect();
                if next.len() > 1 {
                    next = next.into_iter().map(|v| tree_wire(v, env)).collect();
                }
                level = next;
            }
            Some(level.remove(0))
        }
        ("reduce" | "tree_reduce", [Expr::OpRef(op)], Some(items), _) if !items.is_empty() => {
            let combine = |a: Expr, b: Expr| Expr::Binary(op.clone(), Box::new(a), Box::new(b), pos);
            Some(if method == "reduce" { items.into_iter().reduce(combine)? } else { tree(&items, &combine) })
        }
        ("msb" | "lsb", [], None, Expr::Ident(name, _)) => {
            let (dtype, _) = env.signal(name)?;
            let [(msb, lsb)] = dtype.packed.as_slice() else { return None };
            let bit = if method == "msb" { int_of(msb)? } else { int_of(lsb)? };
            Some(Expr::Index(Box::new(base.clone()), Box::new(num(bit))))
        }
        ("delay", [count], _, _) => {
            // a delay line of `k` registers: `x.delay(k)` is the value k cycles ago
            let k = usize::try_from(int_of(count)?).ok()?;
            if k == 0 {
                return Some(base.clone());
            }
            let w = super::width::width_of(base, env)?.width;
            // the registers are named after the signal they delay (`a_2_d1`: the first register after `a[2]`), with a
            // number only when that name is taken
            let stem = env.delay_stem(&delay_root(base, env));
            let dtype = DType { name: "logic".into(), packed: vec![(num(i64::from(w) - 1), num(0))], ..DType::default() };
            let reg = |j: usize| format!("{stem}_d{j}");
            for j in 1..=k {
                // a register is a signal too, so the delay of a delay (`x.delay(1).delay(1)`) knows its width
                env.define_signal(&reg(j), dtype.clone(), Vec::new());
                env.push_delay_decl(Item {
                    pos,
                    kind: ItemKind::Decl(Decl { dtype: dtype.clone(), decls: vec![Declarator { name: reg(j), dims: Vec::new(), init: None }], is_const: false, is_async: false }),
                });
            }
            let mut stmts = vec![Stmt::Assign { lhs: Expr::Ident(reg(1), pos), rhs: base.clone(), nonblocking: true, pos }];
            for j in 2..=k {
                stmts.push(Stmt::Assign { lhs: Expr::Ident(reg(j), pos), rhs: Expr::Ident(reg(j - 1), pos), nonblocking: true, pos });
            }
            env.push_delay_block(Item {
                pos,
                kind: ItemKind::Always {
                    sens: Sens::Edges(vec![("posedge".into(), Expr::Ident("clk".into(), pos))]),
                    body: Stmt::Block(stmts),
                    keyword: "always_ff".into(),
                },
            });
            Some(Expr::Ident(reg(k), pos))
        }
        ("fill", [count], None, _) => Some(Expr::Repl(Box::new(count.clone()), vec![base.clone()])),
        ("sext", [width], None, Expr::Ident(name, _)) => {
            let (dtype, _) = env.signal(name)?;
            let [(msb, lsb)] = dtype.packed.as_slice() else { return None };
            let w = int_of(msb)? - int_of(lsb)? + 1;
            let target = int_of(width)?;
            // the added bits: zeros, or copies of the sign bit
            let fill = if method == "zext" {
                Expr::Num("1'b0".into())
            } else {
                Expr::Index(Box::new(base.clone()), Box::new(num(w - 1)))
            };
            Some(Expr::Concat(vec![Expr::Repl(Box::new(num(target - w)), vec![fill]), base.clone()]))
        }
        // sign extension of a part of a vector (its sign bit is the most significant bit of the part)
        ("sext", [width], None, Expr::Slice(vec, msb, _, op)) if op.as_str() == ":" => {
            let w = i64::from(super::width::width_of(base, env)?.width);
            let target = int_of(width)?;
            let sign = fold_expr(&Expr::Index(vec.clone(), msb.clone()), env);
            Some(Expr::Concat(vec![Expr::Repl(Box::new(num(target - w)), vec![sign]), base.clone()]))
        }
        // zero extension of any expression of known width
        ("zext", [width], None, _) => {
            let target = int_of(width)?;
            // a constant extended is a constant of that width
            if let Some(v) = int_of(base) {
                return Some(Expr::Num(format!("{target}'d{v}")));
            }
            let w = i64::from(super::width::width_of(base, env)?.width);
            Some(Expr::Concat(vec![Expr::Repl(Box::new(num(target - w)), vec![Expr::Num("1'b0".into())]), base.clone()]))
        }
        ("reverse", [], None, Expr::Ident(name, _)) => {
            let (dtype, _) = env.signal(name)?;
            let [(msb, lsb)] = dtype.packed.as_slice() else { return None };
            let (msb, lsb) = (int_of(msb)?, int_of(lsb)?);
            Some(Expr::Concat((lsb..=msb).map(|i| Expr::Index(Box::new(base.clone()), Box::new(num(i)))).collect()))
        }
        _ => None,
    }
}

/// An intermediate value of a reduction as a named wire, when its width is known (a plain name is kept as it is).
fn tree_wire(v: Expr, env: &Env) -> Expr {
    if matches!(v, Expr::Ident(..) | Expr::Num(_)) {
        return v;
    }
    let Some(w) = width_of(&v, env) else { return v };
    let name = format!("ov_tree{}", env.next_delay_id());
    env.push_wire(&name, w.width, v);
    Expr::Ident(name, (0, 0))
}

/// A balanced reduction: the first half, then the second half, as `(a op b) op (c op d)`.
fn tree(items: &[Expr], combine: &dyn Fn(Expr, Expr) -> Expr) -> Expr {
    match items {
        [only] => only.clone(),
        _ => {
            let (left, right) = items.split_at(items.len() / 2);
            combine(tree(left, combine), tree(right, combine))
        }
    }
}

/// A call of a pure function: the body with the arguments substituted, folded in the function's own scope.
fn inline(name: &str, def: &FnDef, args: &[Expr], env: &Env) -> Expr {
    let value_body = substitute(&def.body, &def.params, args);
    let value = fold_expr(&value_body, &env.without_fn(name));
    // a packed return type sets the width of the result
    match def.ret.as_ref().map(|r| r.packed.as_slice()) {
        Some([(msb, lsb)]) => {
            let width = Expr::Binary(
                "+".into(),
                Box::new(Expr::Binary("-".into(), Box::new(msb.clone()), Box::new(lsb.clone()), (0, 0))),
                Box::new(Expr::Num("1".into())),
                (0, 0),
            );
            match int_of(&fold_expr(&width, env)) {
                Some(w) => Expr::Cast(Box::new(num(w)), Box::new(value)),
                None => value,
            }
        }
        _ => value,
    }
}

fn unary(op: &str, v: i64) -> Option<i64> {
    Some(match op {
        "-" => v.checked_neg()?,
        "+" => v,
        "!" => (v == 0) as i64,
        "~" => !v,
        _ => return None,
    })
}

fn binary(op: &str, a: i64, b: i64) -> Option<i64> {
    Some(match op {
        "+" => a.checked_add(b)?,
        "-" => a.checked_sub(b)?,
        "*" => a.checked_mul(b)?,
        "/" => a.checked_div(b)?,
        "%" => a.checked_rem(b)?,
        "**" => a.checked_pow(u32::try_from(b).ok()?)?,
        "<<" | "<<<" => a.checked_shl(u32::try_from(b).ok()?)?,
        ">>" | ">>>" => a.checked_shr(u32::try_from(b).ok()?)?,
        "&" => a & b,
        "|" => a | b,
        "^" => a ^ b,
        "==" | "===" => (a == b) as i64,
        "!=" | "!==" => (a != b) as i64,
        "<" => (a < b) as i64,
        "<=" => (a <= b) as i64,
        ">" => (a > b) as i64,
        ">=" => (a >= b) as i64,
        "&&" => (a != 0 && b != 0) as i64,
        "||" => (a != 0 || b != 0) as i64,
        _ => return None,
    })
}

/// Built-in compile-time functions.
fn builtin(name: &str, args: &[i64]) -> Option<i64> {
    match (name, args) {
        ("clog2" | "$clog2", [x]) => Some(if *x <= 1 { 0 } else { 64 - i64::from((x - 1).leading_zeros()) }),
        ("min", [a, b]) => Some(*a.min(b)),
        ("max", [a, b]) => Some(*a.max(b)),
        ("abs", [x]) => x.checked_abs(),
        // the integer square root, rounded down
        ("isqrt", [x]) if *x >= 0 => {
            let mut r = (*x as f64).sqrt() as i64;
            while r * r > *x {
                r -= 1;
            }
            while (r + 1) * (r + 1) <= *x {
                r += 1;
            }
            Some(r)
        }
        _ => None,
    }
}

/// A type with its parameters bound: a bound type name takes the bound type's own packed dimensions first.
/// The key of a struct type: its name, or for a parameterized struct the key of its instance with these arguments
/// (the instance is laid out on first use).
pub fn struct_key(dt: &DType, env: &Env) -> Option<String> {
    if dt.targs.is_empty() {
        // a type parameter bound to a struct stands for that struct
        if let Some(bound) = env.ty(&dt.name).filter(|t| env.struct_info(&t.name).is_some()) {
            return Some(bound.name.clone());
        }
        return env.struct_info(&dt.name).map(|_| dt.name.clone());
    }
    let def = env.struct_def(&dt.name)?;
    let mut values = Vec::new();
    for (k, p) in def.params.iter().enumerate() {
        let arg = dt.targs.get(k).or(p.default.as_ref())?;
        let Arg::Expr(e) = arg else { return None };
        values.push(int_of(&fold_expr(e, env))?);
    }
    let key = format!("{}<{}>", dt.name, values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "));
    if env.struct_info(&key).is_none() {
        let mut bound = env.scope();
        for (p, v) in def.params.iter().zip(&values) {
            bound.bind_int(&p.name, *v);
        }
        let info = super::build_struct(&def, &bound, "", &key).ok()?;
        env.insert_struct_instance(&key, info);
    }
    Some(key)
}

pub fn fold_dtype(dt: &DType, env: &Env) -> DType {
    // `Vec<Struct, N>`: a struct named as an expression is the struct type (the element of the array)
    if dt.name == "Vec" {
        if let Some(Arg::Expr(Expr::Ident(name, _))) = dt.targs.first() {
            if env.struct_info(name).is_some() {
                let mut dt = dt.clone();
                dt.targs[0] = Arg::Type(DType { name: name.clone(), ..DType::default() });
                return fold_dtype(&dt, env);
            }
        }
    }
    if let Some(key) = struct_key(dt, env).filter(|_| !dt.targs.is_empty()) {
        // an instance of a parameterized struct: its package type when it is a packed struct type
        if env.struct_info(&key).is_some_and(|s| s.typed) {
            return DType { name: crate::outputs::typedef_name(&key), packed: Vec::new(), ..DType::default() };
        }
        let w = env.struct_info(&key).map_or(0, |s| i64::from(s.width));
        return DType { name: "logic".into(), packed: vec![(num(w - 1), num(0))], ..DType::default() };
    }
    // a type parameter bound to a struct is that struct
    if let Some(s) = env.ty(&dt.name).filter(|_| dt.targs.is_empty()).and_then(|bound| env.struct_info(&bound.name)) {
        if s.typed {
            let key = env.ty(&dt.name).map(|bound| bound.name.clone()).unwrap_or_default();
            return DType { name: crate::outputs::typedef_name(&key), packed: Vec::new(), ..DType::default() };
        }
        let w = i64::from(s.width);
        return DType { name: "logic".into(), packed: vec![(num(w - 1), num(0))], ..DType::default() };
    }
    if let Some(s) = env.struct_info(&dt.name).filter(|_| dt.targs.is_empty()) {
        if s.typed {
            return DType { name: crate::outputs::typedef_name(&dt.name), packed: Vec::new(), ..DType::default() };
        }
        // a struct is a bit vector of its width
        let w = i64::from(s.width);
        return DType { name: "logic".into(), packed: vec![(num(w - 1), num(0))], ..DType::default() };
    }
    if let Some(info) = env.enum_info(&dt.name).filter(|_| dt.targs.is_empty()) {
        env.note_enum(&dt.name);
        return info.dtype.clone();
    }
    let packed: Vec<(Expr, Expr)> = dt.packed.iter().map(|(m, l)| (fold_expr(m, env), fold_expr(l, env))).collect();
    match env.ty(&dt.name).filter(|_| dt.targs.is_empty()) {
        Some(bound) => DType {
            name: bound.name.clone(),
            targs: bound.targs.clone(),
            signed: bound.signed || dt.signed,
            packed: bound.packed.iter().cloned().chain(packed).collect(),
        },
        None => DType {
            name: dt.name.clone(),
            targs: dt.targs.iter().map(|a| fold_arg(a, env)).collect(),
            signed: dt.signed,
            packed,
        },
    }
}

pub fn fold_arg(arg: &Arg, env: &Env) -> Arg {
    match arg {
        Arg::Type(t) => Arg::Type(fold_dtype(t, env)),
        Arg::Expr(e) => Arg::Expr(fold_expr(e, env)),
    }
}

pub fn fold_stmt(s: &Stmt, env: &Env) -> Stmt {
    match s {
        Stmt::Block(stmts) => Stmt::Block(fold_block(stmts, env)),
        Stmt::Assign { lhs, rhs, nonblocking, pos } => Stmt::Assign { lhs: fold_expr(lhs, env), rhs: fold_expr(rhs, env), nonblocking: *nonblocking, pos: *pos },
        Stmt::If { cond, then, els } => {
            // a constant condition keeps its branch only (the other one is not in the design)
            let cond = fold_expr(cond, env);
            match int_of(&cond) {
                Some(v) if v != 0 => fold_stmt(then, env),
                Some(_) => els.as_ref().map_or(Stmt::Block(Vec::new()), |e| fold_stmt(e, env)),
                None => Stmt::If { cond, then: boxed_stmt(then, env), els: els.as_ref().map(|e| boxed_stmt(e, env)) },
            }
        }
        Stmt::For { header, body } if is_row_loop(header) && loop_values(header, env).is_some() => {
            Stmt::Block(unroll(header, body, &loop_values(header, env).unwrap_or_default(), env))
        }
        Stmt::For { header, body } if header.bound.is_none() => match sv_loop(header, body, env) {
            Some(looped) => looped,
            None => match loop_values(header, env) {
                Some(values) => Stmt::Block(unroll(header, body, &values, env)),
                None => match row_items(header, env) {
                    Some(items) => Stmt::Block(unroll_rows(header, body, &items, env)),
                    None => Stmt::For { header: fold_header(header, env), body: boxed_stmt(body, env) },
                },
            },
        },
        Stmt::For { header, body } => {
            let header = fold_header(header, env);
            let mut inner = env.clone();
            inner.unbind(&header.var);
            Stmt::For { header, body: boxed_stmt(body, &inner) }
        }
        Stmt::Case { subject, kind, arms, pos } => {
            let subject = fold_expr(subject, env);
            let arms: Vec<(Option<Vec<Expr>>, Stmt)> = arms
                .iter()
                .map(|(labels, body)| (labels.as_ref().map(|ls| ls.iter().map(|l| fold_expr(l, env)).collect()), fold_stmt(body, env)))
                .collect();
            if env.cover_on() && int_of(&subject).is_none() {
                cover_case(&subject, &arms, *pos, env);
            }
            Stmt::Case { subject, kind: kind.clone(), arms, pos: *pos }
        }
        Stmt::Val(name, e) => Stmt::Val(name.clone(), fold_expr(e, env)),
        Stmt::Decl(d) => Stmt::Decl(crate::ast::Decl {
            dtype: fold_dtype(&d.dtype, env),
            decls: d
                .decls
                .iter()
                .map(|x| crate::ast::Declarator {
                    name: x.name.clone(),
                    dims: x.dims.iter().map(|e| fold_expr(e, env)).collect(),
                    init: x.init.as_ref().map(|e| fold_expr(e, env)),
                })
                .collect(),
            is_const: d.is_const,
            is_async: d.is_async,
        }),
        Stmt::Return(e) => Stmt::Return(fold_expr(e, env)),
        Stmt::Goto(e) => Stmt::Goto(fold_expr(e, env)),
        Stmt::ExprStmt(e) => Stmt::ExprStmt(fold_expr(e, env)),
    }
}

/// Statements of a block; unrolled loops contribute their statements directly, as in the reference output.
/// The statements of a block. A `val` is substituted into the statements after it.
/// The statements of a task body, folded.
pub fn fold_body(stmts: &[Stmt], env: &Env) -> Vec<Stmt> {
    fold_block(stmts, env)
}

fn fold_block(stmts: &[Stmt], env: &Env) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut bound: Vec<(String, Expr)> = Vec::new();
    for s in stmts {
        if let Stmt::Val(name, e) = s {
            let value = fold_expr(&substitute_all(e, &bound), env);
            bound.push((name.clone(), value));
            continue;
        }
        // a block of bindings (`val (a, b) = ...`): its names are bound for the statements after it
        if let Stmt::Block(inner) = s {
            if inner.iter().all(|x| matches!(x, Stmt::Val(..))) {
                for x in inner {
                    if let Stmt::Val(name, e) = x {
                        let value = fold_expr(&substitute_all(e, &bound), env);
                        bound.push((name.clone(), value));
                    }
                }
                continue;
            }
        }
        let s = if bound.is_empty() { s.clone() } else { substitute_stmt(s, &bound) };
        match &s {
            Stmt::For { header, body } if is_row_loop(header) && loop_values(header, env).is_some() => {
                out.extend(unroll(header, body, &loop_values(header, env).unwrap_or_default(), env));
            }
            Stmt::For { header, body } if header.bound.is_none() => match sv_loop(header, body, env) {
                Some(looped) => out.push(looped),
                None => match loop_values(header, env) {
                    Some(values) => out.extend(unroll(header, body, &values, env)),
                    None => match row_items(header, env) {
                        Some(items) => out.extend(unroll_rows(header, body, &items, env)),
                        None => out.push(fold_stmt(&s, env)),
                    },
                },
            },
            _ => out.push(fold_stmt(&s, env)),
        }
    }
    out
}

/// The coverage items (`--cover`) of the `match` expressions in a value: each arm taken (its condition and no earlier
/// arm's), and the default arm, when no arm is taken. `pos` is the line of the assignment that holds the value.
pub(crate) fn cover_matches(e: &Expr, pos: Pos, env: &Env) {
    let _ = crate::pipeline::rewrite::map_expr(e, &mut |x| {
        if let Expr::Match(subject, arms) = x {
            if int_of(&fold_expr(subject, env)).is_none() {
                cover_match(subject, arms, pos, env);
            }
        }
        Ok(None)
    });
}

fn cover_match(subject: &Expr, arms: &[(Option<Vec<crate::ast::SetItem>>, Expr)], pos: Pos, env: &Env) {
    let at = format!("{}:{}", env.cover_file(), pos.0);
    let and = |a: Option<Expr>, b: Expr| match a {
        Some(a) => Expr::Binary("&&".into(), boxed(a), boxed(b), pos),
        None => b,
    };
    // `None` while no earlier arm can have been taken: the condition is then true
    let mut none: Option<Expr> = None;
    for (labels, _) in arms {
        let Some(set) = labels else { continue };
        let text: Vec<String> = set
            .iter()
            .map(|i| match &i.hi {
                Some(hi) => format!("{}..{}", crate::emit::expr::expr(&i.lo, -1, false).unwrap_or_default(), crate::emit::expr::expr(hi, -1, false).unwrap_or_default()),
                None => crate::emit::expr::expr(&i.lo, -1, false).unwrap_or_default(),
            })
            .collect();
        let cond = in_set_lowered(subject, set, env);
        env.add_cover(format!("{at} match arm {}", text.join(", ")), and(none.clone(), cond.clone()));
        none = Some(and(none.take(), Expr::Unary("!".into(), boxed(cond))));
    }
    if arms.iter().any(|(labels, _)| labels.is_none()) {
        env.add_cover(format!("{at} match arm default"), none.unwrap_or_else(|| num(1)));
    }
}

/// The coverage items (`--cover`) of a case on a hardware value: each arm taken (its condition and no earlier arm's),
/// the default arm, and for a case on an enum, each state that only the default arm handles.
fn cover_case(subject: &Expr, arms: &[(Option<Vec<Expr>>, Stmt)], pos: Pos, env: &Env) {
    let at = format!("{}:{}", env.cover_file(), pos.0);
    let and = |a: Option<Expr>, b: Expr| match a {
        Some(a) => Expr::Binary("&&".into(), boxed(a), boxed(b), pos),
        None => b,
    };
    // `None` while no earlier arm can have been taken: the condition is then true
    let mut none: Option<Expr> = None;
    let mut enum_seen: Option<String> = None;
    let mut named: Vec<String> = Vec::new();
    for (labels, _) in arms {
        let Some(labels) = labels else { continue };
        let text: Vec<String> = labels.iter().map(|l| crate::emit::expr::expr(l, -1, false).unwrap_or_default()).collect();
        let Some(c) = labels.iter().map(|l| Expr::Binary("==".into(), boxed(subject.clone()), boxed(l.clone()), pos)).reduce(|a, b| Expr::Binary("||".into(), boxed(a), boxed(b), pos)) else { continue };
        env.add_cover(format!("{at} case arm {}", text.join(", ")), and(none.clone(), c.clone()));
        none = Some(and(none.take(), Expr::Unary("!".into(), boxed(c))));
        for l in labels {
            if let Expr::Ident(n, _) = l {
                if let Some((e, m)) = n.rsplit_once('_') {
                    if env.enum_info(e).is_some() {
                        enum_seen.get_or_insert_with(|| e.to_string());
                        if enum_seen.as_deref() == Some(e) {
                            named.push(m.to_string());
                        }
                    }
                }
            }
        }
    }
    let Some(rest) = none else { return };
    let enum_info = enum_seen.as_deref().and_then(|e| env.enum_info(e).map(|i| (e.to_string(), i.members.clone())));
    let complete = enum_info.as_ref().is_some_and(|(_, ms)| ms.iter().all(|(m, _)| named.contains(m)));
    if !complete {
        env.add_cover(format!("{at} case arm default"), rest.clone());
    }
    if let Some((e, members)) = enum_info {
        for (m, _) in members.iter().filter(|(m, _)| !named.contains(m)) {
            let state = Expr::Binary("==".into(), boxed(subject.clone()), boxed(Expr::Ident(format!("{e}_{m}"), pos)), pos);
            env.add_cover(format!("{at} case value {e}.{m} (default arm)"), and(Some(rest.clone()), state));
        }
    }
}

/// A loop that is a priority select: its body is `if (c) { v = e; }` with no `else`, so the last true iteration wins.
/// Unrolled, it is one `if` per iteration, each overriding the one before (the text grows with the range); kept as a
/// loop, SV says the same in one statement.
fn sv_loop(header: &ForHeader, body: &Stmt, env: &Env) -> Option<Stmt> {
    let vars = [header.var.clone()];
    let plain = is_plain_body(body) && !super::stmt_any_expr(body, &|x| super::variable_range(x, &vars));
    if !is_priority_select(body) && !plain {
        return None;
    }
    let Expr::Binary(op, lo, hi, _) = fold_expr(&header.lo, env) else { return None };
    if op.as_str() != ".." {
        return None;
    }
    int_of(&fold_expr(&lo, env))?;
    int_of(&fold_expr(&hi, env))?;
    let mut inner = env.clone();
    inner.unbind(&header.var);
    Some(Stmt::For {
        header: ForHeader {
            var: header.var.clone(),
            lo: fold_expr(&lo, env),
            bound: Some(fold_expr(&hi, env)),
            cmp: "<".into(),
            step: None,
            pos: header.pos,
        },
        body: boxed_stmt(body, &inner),
    })
}

/// A loop body made of assignments, conditions, cases and loops, with no declaration: the loop keeps its SystemVerilog
/// form, which says the same as the unrolled statements.
pub(crate) fn is_plain_body(s: &Stmt) -> bool {
    match s {
        Stmt::Assign { .. } => true,
        Stmt::Block(v) => v.iter().all(is_plain_body),
        Stmt::If { then, els, .. } => is_plain_body(then) && els.as_deref().is_none_or(is_plain_body),
        Stmt::Case { arms, .. } => arms.iter().all(|(_, b)| is_plain_body(b)),
        Stmt::For { body, .. } => is_plain_body(body),
        _ => false,
    }
}

fn is_priority_select(body: &Stmt) -> bool {
    let single = match body {
        Stmt::Block(stmts) if stmts.len() == 1 => &stmts[0],
        other => other,
    };
    matches!(single, Stmt::If { then, els: None, .. } if is_plain_assign(then))
}

/// `v = e;` (possibly in a block) with `v` a plain name.
fn is_plain_assign(s: &Stmt) -> bool {
    match s {
        Stmt::Block(stmts) if stmts.len() == 1 => is_plain_assign(&stmts[0]),
        Stmt::Assign { lhs, nonblocking: false, .. } => matches!(lhs, Expr::Ident(..)),
        _ => false,
    }
}

/// `e` with the names of `bound` replaced (later bindings shadow earlier ones).
fn substitute_all(e: &Expr, bound: &[(String, Expr)]) -> Expr {
    crate::pipeline::rewrite::map_expr(e, &mut |x| {
        Ok(match x {
            Expr::Ident(n, _) => bound.iter().rev().find(|(v, _)| v == n).map(|(_, value)| value.clone()),
            _ => None,
        })
    })
    .unwrap_or_else(|_| e.clone())
}

fn substitute_stmt(s: &Stmt, bound: &[(String, Expr)]) -> Stmt {
    crate::pipeline::rewrite::map_stmt(s, &mut |x| {
        Ok(match x {
            Expr::Ident(n, _) => bound.iter().rev().find(|(v, _)| v == n).map(|(_, value)| value.clone()),
            _ => None,
        })
    })
    .unwrap_or_else(|_| s.clone())
}

/// A loop over the rows of a list (`for (a, b) in list`): the parser writes it over the row indices, named `ov_row_L_C`.
fn is_row_loop(h: &ForHeader) -> bool {
    h.var.starts_with("ov_row_")
}

/// The rows of a loop over a constant list whose elements are not integers (`for (k, v) in [[1, x], [0, y]]`).
fn row_items(h: &ForHeader, env: &Env) -> Option<Vec<Expr>> {
    if h.bound.is_some() {
        return None;
    }
    match fold_expr(&h.lo, env) {
        Expr::List(items) => Some(items),
        _ => None,
    }
}

/// A loop over rows: each row is substituted for the loop name in the body, which is then folded.
fn unroll_rows(header: &ForHeader, body: &Stmt, rows: &[Expr], env: &Env) -> Vec<Stmt> {
    let mut out = Vec::new();
    for row in rows {
        match substitute_stmt(body, &[(header.var.clone(), row.clone())]) {
            Stmt::Block(stmts) => out.extend(fold_block(&stmts, env)),
            other => out.push(fold_stmt(&other, env)),
        }
    }
    out
}

fn unroll(header: &ForHeader, body: &Stmt, values: &[i64], env: &Env) -> Vec<Stmt> {
    let mut out = Vec::new();
    for &v in values {
        let mut inner = env.clone();
        inner.bind_int(&header.var, v);
        match body {
            Stmt::Block(stmts) => out.extend(fold_block(stmts, &inner)),
            other => out.push(fold_stmt(other, &inner)),
        }
    }
    out
}

fn boxed_stmt(s: &Stmt, env: &Env) -> Box<Stmt> {
    Box::new(fold_stmt(s, env))
}

/// The values a generate loop takes: a list, or a range `lo..hi`, or `for (i = lo; i < hi; i += k)`.
/// Upper bound on the iterations of one unrolled loop, to stop runaway elaboration.
const MAX_ITERATIONS: usize = 1_000_000;

/// The values of a range or list loop (`for i in lo..hi`, `for i in [a, b]`, `for (i = lo; i < hi; i += k)`),
/// or `None` when the bounds are not constant.
pub fn loop_values(h: &ForHeader, env: &Env) -> Option<Vec<i64>> {
    let constant = |e: &Expr| int_of(&fold_expr(e, env));
    // the range or list may be an expression (`(1..N).reverse()`): it is folded first
    let lo_expr = fold_expr(&h.lo, env);
    let (lo, hi, cmp) = match (&h.bound, &lo_expr) {
        (None, Expr::List(items)) => return items.iter().map(constant).collect(),
        (None, Expr::Binary(op, a, b, _)) if op.as_str() == ".." => (constant(a)?, constant(b)?, "<"),
        (None, _) => return None,
        (Some(bound), lo) => (constant(lo)?, constant(bound)?, h.cmp.as_str()),
    };
    let step = match &h.step {
        Some(s) => constant(s)?,
        None => 1,
    };
    if step == 0 {
        return None;
    }
    let holds = |v: i64| match cmp {
        "<" => v < hi,
        "<=" => v <= hi,
        ">" => v > hi,
        ">=" => v >= hi,
        "!=" => v != hi,
        _ => false,
    };
    let mut values = Vec::new();
    let mut v = lo;
    while holds(v) {
        values.push(v);
        if values.len() > MAX_ITERATIONS {
            return None;
        }
        v += step;
    }
    Some(values)
}

fn fold_header(h: &ForHeader, env: &Env) -> ForHeader {
    // `for i in lo..hi` is `i = lo; i < hi`
    let (lo, bound) = match (&h.bound, &h.lo) {
        (None, Expr::Binary(op, a, b, _)) if op.as_str() == ".." => (a.as_ref(), Some(b.as_ref())),
        (bound, lo) => (lo, bound.as_ref()),
    };
    ForHeader {
        var: h.var.clone(),
        lo: fold_expr(lo, env),
        bound: bound.map(|b| fold_expr(b, env)),
        cmp: h.cmp.clone(),
        step: h.step.as_ref().map(|s| fold_expr(s, env)),
        pos: h.pos,
    }
}
