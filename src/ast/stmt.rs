use super::expr::{Expr, Pos};
use super::item::Decl;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Block(Vec<Stmt>),
    /// `val name = expr;` in a block: a name for a value, substituted into the statements after it (no hardware).
    Val(String, Expr),
    /// `lhs = rhs` (blocking) or `lhs <= rhs` (non-blocking).
    /// The position is the first token of the target.
    Assign { lhs: Expr, rhs: Expr, nonblocking: bool, pos: Pos },
    If { cond: Expr, then: Box<Stmt>, els: Option<Box<Stmt>> },
    For { header: ForHeader, body: Box<Stmt> },
    /// `kind` is `case`, `casez` or `casex`; `None` labels the `default` arm.
    /// The position is the `case` keyword.
    Case { subject: Expr, kind: String, arms: Vec<(Option<Vec<Expr>>, Stmt)>, pos: Pos },
    Decl(Decl),
    Return(Expr),
    /// `goto State;` inside an fsm: the next state.
    Goto(Expr),
    ExprStmt(Expr),
}

/// `for (int i = lo; i < bound; i += step)`, or `for i in range` (then `bound` is `None` and `lo` is the range).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForHeader {
    pub var: String,
    pub lo: Expr,
    pub bound: Option<Expr>,
    pub cmp: String,
    pub step: Option<Expr>,
    pub pos: Pos,
}
