//! Expression and type text. Parentheses only where the SystemVerilog precedence needs them.

use crate::ast::{Arg, DType, Dir, Expr};
use crate::error::{Error, Result};
use crate::parser::binop_prec;

const TERNARY_PREC: i32 = 0;
const UNARY_PREC: i32 = 13;
const PRIMARY_PREC: i32 = 99;

/// The methods of values, for the suggestion when a call is not one of them.
const METHODS: &[&str] = &["wrap", "reverse", "msb", "lsb", "zext", "sext", "fill", "cat", "delay", "list", "len", "filter", "enumerate", "select", "fold", "map"];

fn prec(e: &Expr) -> i32 {
    match e {
        Expr::Binary(op, ..) => binop_prec(op).map_or(PRIMARY_PREC, i32::from),
        Expr::Ternary(..) => TERNARY_PREC,
        Expr::Unary(..) => UNARY_PREC,
        _ => PRIMARY_PREC,
    }
}

/// `parent` is the precedence of the enclosing operator (-1 at the top); `right` marks the right operand,
/// where an equal precedence also needs parentheses (the operators are left-associative).
pub fn expr(e: &Expr, parent: i32, right: bool) -> Result<String> {
    let s = text(e)?;
    let p = prec(e);
    let same_binary = right && p == parent && matches!(e, Expr::Binary(..));
    Ok(if p < parent || same_binary { format!("({s})") } else { s })
}

fn text(e: &Expr) -> Result<String> {
    Ok(match e {
        Expr::Num(t) => t.clone(),
        Expr::Ident(name, _) => name.clone(),
        Expr::Unary(op, arg) => {
            let mut a = expr(arg, UNARY_PREC, false)?;
            if matches!(**arg, Expr::Unary(..)) {
                a = format!("({a})"); // SV: a unary operand must be a primary
            }
            format!("{op}{a}")
        }
        Expr::Binary(op, l, r, _) => {
            let p = binop_prec(op).map_or(PRIMARY_PREC, i32::from);
            format!("{} {op} {}", expr(l, p, false)?, expr(r, p, true)?)
        }
        Expr::Ternary(c, a, b) => format!("{} ? {} : {}", expr(c, 1, false)?, expr(a, 1, false)?, expr(b, -1, false)?),
        Expr::Index(base, idx) => format!("{}[{}]", expr(base, PRIMARY_PREC, false)?, expr(idx, -1, false)?),
        Expr::Slice(base, msb, lsb, op) => format!(
            "{}[{}{op}{}]",
            expr(base, PRIMARY_PREC, false)?,
            expr(msb, -1, false)?,
            expr(lsb, -1, false)?
        ),
        // a field of a packed struct signal, or of a field of one: `p.r`, `p.inner.x`; any other member is a
        // hierarchical reference or a method
        Expr::Member(base, name, _) if is_named_chain(base) => format!("{}.{name}", expr(base, PRIMARY_PREC, false)?),
        Expr::Member(_, name, pos) => {
            return Err(Error::new(format!("'.{name}' (a hierarchical reference or a method) is not supported yet"), "", pos.0, pos.1));
        }
        Expr::Call(func, args) => {
            if let Expr::Member(base, name, pos) = &**func {
                // `a.cat(b)`: the bits of `a` followed by those of `b`; every operand needs a width
                if name == "cat" && !args.is_empty() {
                    if args.iter().any(|a| matches!(a, Expr::Num(t) if !t.contains('\''))) {
                        return Err(Error::new("cat() needs sized operands", "", pos.0, pos.1));
                    }
                    let parts: Vec<Expr> = std::iter::once((**base).clone()).chain(args.iter().cloned()).collect();
                    return Ok(format!("{{{}}}", list(&parts)?));
                }
                let hint = crate::elab::suggest(name, METHODS.iter().map(|m| m.to_string()));
                return Err(Error::new(format!("method '{name}' is not supported yet{hint}"), "", pos.0, pos.1));
            }
            format!("{}({})", expr(func, PRIMARY_PREC, false)?, list(args)?)
        }
        Expr::Concat(parts) => format!("{{{}}}", list(parts)?),
        Expr::Repl(count, parts) => format!("{{{}{{{}}}}}", expr(count, -1, false)?, list(parts)?),
        Expr::Cast(width, arg) => {
            let w = expr(width, -1, false)?;
            let w = if matches!(**width, Expr::Ident(..) | Expr::Num(_)) { w } else { format!("({w})") };
            format!("{w}'({})", expr(arg, -1, false)?)
        }
        Expr::Pattern(items) => format!("'{{{}}}", list(items)?),
        Expr::Keyed(k, v) => format!("{k}: {}", expr(v, -1, false)?),
        Expr::With(..) => {
            return Err(Error::new("a configuration with overrides is only valid as a parameter value", "", 0, 0));
        }
        Expr::Type(_) => return Err(Error::new("a type is not a value here", "", 0, 0)),
        Expr::OpRef(_) | Expr::Lambda(..) | Expr::List(_) | Expr::InSet(..) | Expr::Match(..) => {
            return Err(Error::new("this expression is not supported in SystemVerilog output yet", "", 0, 0));
        }
    })
}

/// A signal, an element of an array of them, or a field of one (`p`, `px[2]`, `p.inner`): the names a field select
/// can be written on.
fn is_named_chain(e: &Expr) -> bool {
    match e {
        Expr::Ident(..) => true,
        Expr::Index(base, _) | Expr::Member(base, _, _) => is_named_chain(base),
        _ => false,
    }
}

fn list(items: &[Expr]) -> Result<String> {
    let parts: Result<Vec<String>> = items.iter().map(|x| expr(x, -1, false)).collect();
    Ok(parts?.join(", "))
}

/// A data type: name (`logic` when empty), generic arguments, `signed`, packed dimensions.
pub fn dtype(dt: &DType) -> Result<String> {
    let mut s = if dt.name.is_empty() { "logic".to_string() } else { dt.name.clone() };
    if !dt.targs.is_empty() {
        let args: Result<Vec<String>> = dt
            .targs
            .iter()
            .map(|a| match a {
                Arg::Type(t) => dtype(t),
                Arg::Expr(e) => expr(e, -1, false),
            })
            .collect();
        s += &format!("#({})", args?.join(", "));
    }
    if dt.signed {
        s += " signed";
    }
    for (msb, lsb) in &dt.packed {
        s += &format!(" [{}:{}]", expr(msb, -1, false)?, expr(lsb, -1, false)?);
    }
    Ok(s)
}

/// Unpacked dimensions after a name: ` [N]` per dimension.
pub fn dims(ds: &[Expr]) -> Result<String> {
    let mut s = String::new();
    for d in ds {
        s += &format!(" [{}]", expr(d, -1, false)?);
    }
    Ok(s)
}

pub fn dir_keyword(dir: Dir) -> &'static str {
    match dir {
        Dir::Input => "input",
        Dir::Output => "output",
        Dir::Inout => "inout",
        Dir::Master => "master",
        Dir::Slave => "slave",
        Dir::Monitor => "monitor",
    }
}
