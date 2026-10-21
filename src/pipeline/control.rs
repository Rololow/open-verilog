//! Control of a pipeline: stalls, flushes and the valid bits.
//!
//! A stall in stage `k` holds stage `k` and every earlier one. A flush in stage `k` kills the instruction in `k`
//! and the earlier ones; `flush_younger_when` kills only the earlier ones. Conditions count only in stages
//! that hold a valid instruction. A killed instruction leaves the boundary it entered, and the producer of
//! stage 0 is accepted and dropped, so it never has to withdraw what it offered.

use crate::ast::{Expr, Item, ItemKind, Pos, Sens, Stmt};

/// The conditions written in one stage.
#[derive(Clone, Default)]
pub struct Conditions {
    pub stall: Vec<Expr>,
    pub flush: Vec<Expr>,
    pub younger: Vec<Expr>,
}

/// The control signals of a pipeline. Each is `None` when nothing can set it.
pub struct Plan {
    /// Wires to declare and assign, in order: (name, value).
    pub wires: Vec<(String, Expr)>,
    pub stall: Vec<Option<Expr>>,
    pub flush: Vec<Option<Expr>>,
    /// The stage or a later one is stalled.
    pub hold: Vec<Option<Expr>>,
    /// The instruction entering boundary `j` (into stage `j`) is killed.
    pub kill: Vec<Option<Expr>>,
}

fn ident(name: &str, pos: Pos) -> Expr {
    Expr::Ident(name.to_string(), pos)
}

fn or_all(terms: Vec<Expr>) -> Option<Expr> {
    terms.into_iter().reduce(|a, b| Expr::Binary("||".into(), Box::new(a), Box::new(b), (0, 0)))
}

fn binary(op: &str, a: Expr, b: Expr) -> Expr {
    Expr::Binary(op.into(), Box::new(a), Box::new(b), (0, 0))
}

fn not_bit(e: Expr) -> Expr {
    Expr::Unary("~".into(), Box::new(e))
}

/// Builds the control plan. `valid[k]` names the valid bit of stage `k`; `names[k]` is the stage name.
pub fn plan(names: &[&str], valid: &[String], controls: &[Conditions], fires: &[usize], pos: Pos) -> Plan {
    let n = names.len();
    let mut wires = Vec::new();
    let mut wire = |name: String, value: Expr| -> Option<Expr> {
        let reference = ident(&name, pos);
        wires.push((name, value));
        Some(reference)
    };
    let gated = |k: usize, conds: &[Expr]| -> Option<Expr> {
        or_all(conds.to_vec()).map(|c| binary("&&", ident(&valid[k], pos), c))
    };

    let mut stall = vec![None; n];
    let mut flush = vec![None; n];
    let mut younger = vec![None; n];
    for k in 0..n {
        if let Some(e) = gated(k, &controls[k].stall) {
            stall[k] = wire(format!("ov_stall_{}", names[k]), e);
        }
        if let Some(e) = gated(k, &controls[k].flush) {
            flush[k] = wire(format!("ov_flush_{}", names[k]), e);
        }
        if let Some(e) = gated(k, &controls[k].younger) {
            younger[k] = wire(format!("ov_younger_{}", names[k]), e);
        }
    }

    // hold[j]: stage j or a later stage is stalled
    let mut hold = vec![None; n];
    let mut acc: Option<Expr> = None;
    for j in (0..n).rev() {
        let e = or_all(stall[j].iter().chain(acc.iter()).cloned().collect());
        if let Some(e) = e {
            hold[j] = wire(format!("ov_hold_{}", names[j]), e);
        }
        acc = hold[j].clone();
    }

    // kill[j]: the instruction entering stage j is killed by a flush of stage j-1 or later, or by a younger event
    let mut kill = vec![None; n];
    let mut acc = flush[n - 1].clone();
    for j in (1..n).rev() {
        let e = or_all([flush[j - 1].clone(), younger[j].clone(), acc.clone()].into_iter().flatten().collect());
        if let Some(e) = e {
            kill[j] = wire(format!("ov_kill_{}", names[j]), e);
        }
        acc = kill[j].clone();
    }

    for &k in fires {
        let gone: Vec<Expr> = if k < n - 1 {
            [hold[k].clone(), kill[k + 1].clone()].into_iter().flatten().collect()
        } else {
            [stall[k].clone(), flush[k].clone()].into_iter().flatten().collect()
        };
        let mut e = ident(&valid[k], pos);
        for t in gone {
            e = binary("&", e, not_bit(t));
        }
        wire(format!("ov_fire_{}", names[k]), e);
    }
    Plan { wires, stall, flush, hold, kill }
}

impl Plan {
    /// Stage 0 cannot take its input: it is held, and the instruction entering stage 1 is not killed.
    /// `None` when nothing can hold stage 0.
    pub fn blocked(&self) -> Option<Expr> {
        let n = self.hold.len();
        match (&self.hold[0], n) {
            (Some(h), 1) => Some(match &self.flush[0] {
                Some(f) => binary("&", h.clone(), not_bit(f.clone())),
                None => h.clone(),
            }),
            (Some(h), _) => Some(match &self.kill[1] {
                Some(k) => binary("&", h.clone(), not_bit(k.clone())),
                None => h.clone(),
            }),
            (None, _) => None,
        }
    }

    /// `ready_out`: low while stage 0 cannot take its input.
    pub fn ready_out(&self) -> Expr {
        match self.blocked() {
            Some(b) => not_bit(b),
            None => Expr::Num("1'b1".into()),
        }
    }

    /// `valid_out`: the last stage holds a valid instruction that is neither killed nor stalled.
    pub fn valid_out(&self, valid_last: &str, pos: Pos) -> Expr {
        let n = self.hold.len();
        let mut e = ident(valid_last, pos);
        for t in [self.flush[n - 1].clone(), self.stall[n - 1].clone()].into_iter().flatten() {
            e = binary("&", e, not_bit(t));
        }
        e
    }
}

/// A value carried between stages: the name, the stage that defines it and the last stage that reads it.
pub struct Carried {
    pub name: String,
    pub owner: usize,
    pub last: usize,
}

/// The register boundaries: for each stage after the first, the valid bit and the carried values move forward.
/// A stall freezes them, a kill clears the valid bit, reset clears it too.
pub fn boundaries(plan: &Plan, names: &[&str], valid: &[String], carried: &[Carried], clock: &str, rst: Expr, pos: Pos) -> Vec<Item> {
    let n = names.len();
    let mut out = Vec::new();
    for j in 1..n {
        let mut next = ident(&valid[j - 1], pos);
        if let Some(s) = &plan.stall[j - 1] {
            next = binary("&", next, not_bit(s.clone()));
        }
        let mut advance = vec![nonblocking(&valid[j], next, pos)];
        if let Some(h) = &plan.hold[j] {
            advance = vec![Stmt::If { cond: not_bit(h.clone()), then: Box::new(Stmt::Block(advance)), els: None }];
        }
        if let Some(k) = &plan.kill[j] {
            advance = vec![Stmt::If {
                cond: k.clone(),
                then: Box::new(Stmt::Block(vec![nonblocking_num(&valid[j], "1'b0", pos)])),
                els: Some(Box::new(Stmt::Block(advance))),
            }];
        }
        let mut body = vec![Stmt::If {
            cond: rst.clone(),
            then: Box::new(Stmt::Block(vec![nonblocking_num(&valid[j], "1'b0", pos)])),
            els: Some(Box::new(Stmt::Block(advance))),
        }];

        let mut data = Vec::new();
        for c in carried {
            if c.owner < j && j <= c.last {
                let prev = if j == c.owner + 1 { c.name.clone() } else { format!("{}__s{}", c.name, j - 1) };
                data.push(nonblocking(&format!("{}__s{j}", c.name), ident(&prev, pos), pos));
            }
        }
        match (&plan.hold[j], data.is_empty()) {
            (_, true) => {}
            (Some(h), false) => body.push(Stmt::If { cond: not_bit(h.clone()), then: Box::new(Stmt::Block(data)), els: None }),
            (None, false) => body.extend(data),
        }
        out.push(Item { pos, kind: ItemKind::Comment(format!("stage '{}' takes its values from '{}'", names[j], names[j - 1])) });
        out.push(Item {
            pos,
            kind: ItemKind::Always {
                sens: Sens::Edges(vec![("posedge".into(), ident(clock, pos))]),
                body: Stmt::Block(body),
                keyword: "always_ff".into(),
            },
        });
    }
    out
}

fn nonblocking(lhs: &str, rhs: Expr, pos: Pos) -> Stmt {
    Stmt::Assign { lhs: ident(lhs, pos), rhs, nonblocking: true }
}

fn nonblocking_num(lhs: &str, rhs: &str, pos: Pos) -> Stmt {
    Stmt::Assign { lhs: ident(lhs, pos), rhs: Expr::Num(rhs.into()), nonblocking: true }
}
