/// Source position (line, column), 1-based.
pub type Pos = (usize, usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Num(String),
    Ident(String, Pos),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>, Pos),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `base[idx]`
    Index(Box<Expr>, Box<Expr>),
    /// `base[msb:lsb]`, `base[start +: w]`, `base[start -: w]`; `op` is `":"`, `"+:"` or `"-:"`.
    Slice(Box<Expr>, Box<Expr>, Box<Expr>, String),
    /// `base.name`; the position is that of `name`.
    Member(Box<Expr>, String, Pos),
    Call(Box<Expr>, Vec<Expr>),
    /// `{a, b}`
    Concat(Vec<Expr>),
    /// `{n{a, b}}`
    Repl(Box<Expr>, Vec<Expr>),
    /// `width'(value)`
    Cast(Box<Expr>, Box<Expr>),
    /// `reduce(+)`: an operator used as a function argument.
    OpRef(String),
    /// `(a, b) => body`
    Lambda(Vec<String>, Box<Expr>),
    /// `[a, b, c]`
    List(Vec<Expr>),
    /// `x in {a, lo..hi}`
    InSet(Box<Expr>, Vec<SetItem>),
    /// `match (x) { a, lo..hi: value; ...; default: value }`: lowered to a conditional chain when folded.
    Match(Box<Expr>, Vec<(Option<Vec<SetItem>>, Expr)>),
    /// `'{a, b, c}`: an assignment pattern (the values of an array).
    Pattern(Vec<Expr>),
    /// `key: value` inside an assignment pattern (`'{default: 0}`).
    Keyed(String, Box<Expr>),
    /// A type as an argument (`a.as(Word<C.W>)`): a struct, possibly parameterized.
    Type(DType),
    /// `config with { field = value, ... }`: a configuration with some fields overridden.
    With(Box<Expr>, Vec<(String, Expr)>, Pos),
}

/// One element of a set: a value, or the range `lo..hi`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetItem {
    pub lo: Expr,
    pub hi: Option<Expr>,
}

/// A generic argument: a type (`logic [7:0]`) or a compile-time expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Type(DType),
    Expr(Expr),
}

/// A data type: a base or user type name (empty means `logic`), generic arguments, signedness and packed dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DType {
    pub name: String,
    pub targs: Vec<Arg>,
    pub signed: bool,
    /// `[msb:lsb]` pairs, outermost first.
    pub packed: Vec<(Expr, Expr)>,
}
