//! `fsm`: a state register and its next-state logic. The states are members of an enum; `goto S` sets the
//! next state, and the arm `S: { ... }` holds what happens in state `S`. The clock is `clk` and the reset `rst`.

use super::env::Env;
use crate::ast::{Declarator, Decl, Expr, Item, ItemKind, Pos, Sens, Stmt};
use crate::error::{Error, Result};

/// The lowered fsm: the state register, the next-state wire, the combinational logic and the register.
pub fn lower(env: &Env, pos: Pos, file: &str, names: (&str, &str, &str), defaults: &[Stmt], arms: &[(String, Stmt)]) -> Result<Vec<Item>> {
    let (enum_name, name, init) = names;
    let fail = |msg: String| Error::new(msg, file, pos.0, pos.1);
    let info = env.enum_info(enum_name).ok_or_else(|| fail(format!("'{enum_name}' is not an enum")))?;
    env.note_enum(enum_name);
    let member = |m: &str| -> Result<Expr> {
        if info.members.iter().any(|(x, _)| x == m) {
            Ok(Expr::Ident(format!("{enum_name}_{m}"), pos))
        } else {
            Err(fail(format!("'{m}' is not a member of enum '{enum_name}'")))
        }
    };
    let next = format!("{name}_next");
    let init_state = member(init)?;
    let goto = |target: &Expr| -> Result<Expr> {
        match target {
            // the target may already be folded to the enum's localparam name (a bare member is resolved first); a
            // target that is not a state is reported where it is written
            Expr::Ident(state, at) => {
                let m = state.strip_prefix(&format!("{enum_name}_")).unwrap_or(state);
                if info.members.iter().any(|(x, _)| x == m) {
                    Ok(Expr::Ident(format!("{enum_name}_{m}"), pos))
                } else {
                    Err(Error::new(format!("'{m}' is not a state of '{enum_name}'"), file, at.0, at.1))
                }
            }
            _ => Err(fail("goto takes a state name".into())),
        }
    };
    let defaults = defaults.iter().map(|s| resolve_gotos(s, &next, &goto)).collect::<Result<Vec<_>>>()?;
    let mut comb = vec![assign(&next, ident(name, pos), false, pos)];
    comb.extend(defaults);
    let mut cases = Vec::new();
    for (label, body) in arms {
        cases.push((Some(vec![member(label)?]), resolve_gotos(body, &next, &goto)?));
    }
    comb.push(Stmt::Case { subject: ident(name, pos), kind: "case".into(), arms: cases, pos });

    // the module's reset (see `implicit_clocking`): its port and its polarity
    let reset = env.reset().cloned().unwrap_or(super::env::ResetSpec { name: "rst".into(), low: false, is_async: false });
    let active = ident(&reset.name, pos);
    let register = Stmt::If {
        cond: if reset.low { Expr::Unary("!".into(), Box::new(active)) } else { active },
        then: Box::new(Stmt::Block(vec![assign(name, init_state.clone(), true, pos)])),
        els: Some(Box::new(Stmt::Block(vec![assign(name, ident(&next, pos), true, pos)]))),
    };
    let dtype = info.dtype.clone();
    Ok(vec![
        Item { pos, kind: decl(dtype.clone(), name, Some(init_state)) },
        Item { pos, kind: decl(dtype, &next, None) },
        Item { pos, kind: ItemKind::AlwaysComb(Stmt::Block(comb)) },
        Item {
            pos,
            kind: ItemKind::Always {
                sens: Sens::Edges(vec![("posedge".into(), ident("clk", pos))]),
                body: register,
                keyword: "always_ff".into(),
            },
        },
    ])
}

fn ident(name: &str, pos: Pos) -> Expr {
    Expr::Ident(name.to_string(), pos)
}

fn assign(lhs: &str, rhs: Expr, nonblocking: bool, pos: Pos) -> Stmt {
    Stmt::Assign { lhs: ident(lhs, pos), rhs, nonblocking, pos }
}

fn decl(dtype: crate::ast::DType, name: &str, init: Option<Expr>) -> ItemKind {
    ItemKind::Decl(Decl {
        dtype,
        decls: vec![Declarator { name: name.to_string(), dims: Vec::new(), init }],
        is_const: false,
        is_async: false,
    })
}

/// Replaces each `goto S;` by an assignment of the next state.
fn resolve_gotos(s: &Stmt, next: &str, goto: &dyn Fn(&Expr) -> Result<Expr>) -> Result<Stmt> {
    Ok(match s {
        Stmt::Goto(target) => Stmt::Assign { lhs: Expr::Ident(next.to_string(), (0, 0)), rhs: goto(target)?, nonblocking: false, pos: (0, 0) },
        Stmt::Block(stmts) => Stmt::Block(stmts.iter().map(|x| resolve_gotos(x, next, goto)).collect::<Result<_>>()?),
        Stmt::If { cond, then, els } => Stmt::If {
            cond: cond.clone(),
            then: Box::new(resolve_gotos(then, next, goto)?),
            els: match els {
                Some(e) => Some(Box::new(resolve_gotos(e, next, goto)?)),
                None => None,
            },
        },
        Stmt::For { header, body } => Stmt::For { header: header.clone(), body: Box::new(resolve_gotos(body, next, goto)?) },
        Stmt::Case { subject, kind, arms, pos } => {
            let mut out = Vec::with_capacity(arms.len());
            for (labels, body) in arms {
                out.push((labels.clone(), resolve_gotos(body, next, goto)?));
            }
            Stmt::Case { subject: subject.clone(), kind: kind.clone(), arms: out, pos: *pos }
        }
        other => other.clone(),
    })
}
