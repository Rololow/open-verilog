//! Type shorthands lowered to plain SystemVerilog dimensions:
//! `logic<8>` is `logic [7:0]`, and `Vec<T, N>` is `T` with an unpacked dimension `[N]`.

use super::fold::fold_expr;
use super::env::Env;
use crate::ast::{Arg, DType, Expr};

const BIT_TYPES: &[&str] = &["logic", "wire", "reg", "bit", "byte", "int", "integer", "shortint", "longint"];

/// The lowered type and the unpacked dimensions the shorthand contributed (outermost first).
/// `Err` names the shape that is not supported.
pub fn lower(dt: DType) -> Result<(DType, Vec<Expr>), String> {
    match (dt.name.as_str(), dt.targs.as_slice()) {
        ("Vec", [Arg::Type(elem), Arg::Expr(n)]) => {
            let (base, inner) = lower(elem.clone())?;
            let mut dims = vec![n.clone()];
            dims.extend(inner);
            Ok((DType { signed: base.signed || dt.signed, ..base }, dims))
        }
        ("Vec", _) => Err("Vec takes an element type and a size: Vec<T, N>".into()),
        (name, [Arg::Expr(n)]) if BIT_TYPES.contains(&name) => {
            let msb = Expr::Binary("-".into(), Box::new(n.clone()), Box::new(Expr::Num("1".into())), (0, 0));
            let mut packed = vec![(fold_expr(&msb, &Env::default()), Expr::Num("0".into()))];
            packed.extend(dt.packed);
            Ok((DType { name: dt.name, targs: Vec::new(), signed: dt.signed, packed }, Vec::new()))
        }
        _ => Ok((dt, Vec::new())),
    }
}
