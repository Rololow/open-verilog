use super::expr::{Arg, DType, Expr, Pos};
use super::stmt::{ForHeader, Stmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    /// `module` or `pipeline`.
    pub kind: String,
    pub name: String,
    pub params: Vec<Param>,
    pub ports: Vec<Port>,
    pub extends: Option<(String, Vec<Arg>)>,
    pub traits: Vec<String>,
    pub items: Vec<Item>,
    pub pos: Pos,
    /// The file the module is defined in (empty for a module made by elaboration without one).
    pub origin: String,
    /// The enums the module declares: they are its own, and shadow the top-level ones of the same name.
    pub enums: Vec<EnumDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// `int`, `bool`, `type`, `module`, `config`, ...
    pub kind: String,
    pub name: String,
    pub default: Option<Arg>,
    pub bound: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Input,
    Output,
    Inout,
    Master,
    Slave,
    Monitor,
}

impl Dir {
    pub fn from_keyword(word: &str) -> Option<Dir> {
        Some(match word {
            "input" => Dir::Input,
            "output" => Dir::Output,
            "inout" => Dir::Inout,
            "master" => Dir::Master,
            "slave" => Dir::Slave,
            "monitor" => Dir::Monitor,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub dir: Dir,
    pub dtype: DType,
    pub name: String,
    pub dims: Vec<Expr>,
    /// Set for conditional ports: `if (C) input x`.
    pub cond: Option<Expr>,
    pub is_async: bool,
    pub default: Option<Expr>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declarator {
    pub name: String,
    pub dims: Vec<Expr>,
    pub init: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    pub dtype: DType,
    pub decls: Vec<Declarator>,
    pub is_const: bool,
    pub is_async: bool,
}

/// Sensitivity of an `always` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sens {
    /// `@(posedge clk or negedge rst)`; empty means the default clock of the scope.
    Edges(Vec<(String, Expr)>),
    /// `@(*)` or level-sensitive.
    Comb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conn {
    /// `None` for a positional connection; `Some("*")` for `.*`.
    pub port: Option<String>,
    /// `None` when `.port()` leaves the port unconnected.
    pub expr: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub module: String,
    /// Positional (`None` name) or named (`.W(8)`) generic values.
    pub targs: Vec<(Option<String>, Arg)>,
    pub name: String,
    pub conns: Vec<Conn>,
    pub pos: Pos,
}

/// A module item and the position of its first token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub pos: Pos,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    Decl(Decl),
    ContAssign { lhs: Expr, rhs: Expr },
    AlwaysComb(Stmt),
    /// `always_ff` / `always`; `keyword` is the one written in the source.
    Always { sens: Sens, body: Stmt, keyword: String },
    Instance(Instance),
    GenIf { cond: Expr, then: Vec<Item>, els: Option<Vec<Item>> },
    GenFor { header: ForHeader, body: Vec<Item> },
    TieOff(Expr),
    /// A pipeline stage: combinational items, lowered by the pipeline pass.
    Stage { name: String, items: Vec<Item> },
    /// `stall_when(c)` / `flush_when(c)` / `flush_younger_when(c)` in a stage; `kind` is `stall`, `flush` or `younger`.
    Ctrl { kind: String, cond: Expr },
    /// `initial` block (simulation only): its statements run once at the start.
    Initial(Stmt),
    /// A comment in the output.
    Comment(String),
    /// Lines copied verbatim (the checks of a pipeline).
    Raw(Vec<String>),
    /// `val name = expr;`: a named signal whose width is inferred losslessly.
    Val { name: String, expr: Expr },
    /// `reset NAME [low] [async];`: the module's reset.
    Reset { name: String, low: bool, is_async: bool },
    /// `fsm Enum state = INIT { defaults; LABEL: { ... } }`: a state register and its next-state logic.
    Fsm { enum_name: String, name: String, init: String, defaults: Vec<Stmt>, arms: Vec<(String, Stmt)> },
    /// A pure function `fn name(params) [-> type] { return body; }`; calls are inlined.
    Fn { name: String, params: Vec<String>, ptypes: Vec<DType>, ret: Option<DType>, body: Expr },
    /// A procedural function: statements, inlined at each statement-level call (the parser expands them).
    Proc { name: String, params: Vec<String>, body: Vec<Stmt> },
    /// `memmap io "desc" { DEV @ base size n [irq k [= line]] ... }`: the devices of a bus, placed in the module.
    MemMap { name: String, desc: String, entries: Vec<MapEntryDef> },
    /// A task (simulation code): `task name(input logic [31:0] p); decls; statements endtask`.
    Task { name: String, args: Vec<(DType, String)>, body: Vec<Stmt> },
}

/// A file-level enum: `enum Name [: type] { A, B = 3, C }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDef {
    pub name: String,
    /// The storage type when given; otherwise just wide enough for the values.
    pub dtype: Option<DType>,
    pub members: Vec<(String, Option<Expr>)>,
    pub pos: Pos,
}

/// The contents of a source file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct File {
    pub modules: Vec<Module>,
    /// The register maps (their RTL is in `modules`; the C header and the documentation come from these).
    pub regmaps: Vec<crate::regmap::RegMap>,
    pub enums: Vec<EnumDef>,
    /// Top-level constants: compile-time only, visible to every module.
    pub consts: Vec<Decl>,
    /// Top-level `val`s: a constant, or a list of constants, visible to every module.
    pub values: Vec<(String, Expr)>,
    /// Top-level functions, visible to every module.
    pub fns: Vec<ItemKind>,
    /// Bundles: groups of signals with directions, flattened into ports and wires.
    pub bundles: Vec<BundleDef>,
    /// Configurations: compile-time records of named values, with `extends`.
    pub configs: Vec<ConfigDef>,
    /// Traits: the functions a module with `: Trait` must define.
    pub traits: Vec<TraitDef>,
    /// Modules of the standard library: emitted only where they are instantiated.
    pub library: Vec<String>,
    /// Instruction sets: the formats of the instructions, with their fields.
    pub isas: Vec<IsaDef>,
    /// `test "name" for Module<args> { ... }`: a test of the design (its testbench is written by --emit-tests).
    pub tests: Vec<TestDef>,
    /// `for t in list { test ... }`: tests made from the rows of a list.
    pub test_loops: Vec<TestLoop>,
    /// `probe name on Module when (...) { ... }`: a debug probe (written by --emit-probes).
    pub probes: Vec<ProbeDef>,
    /// Structs (flattened to bit vectors) and typedefs of data types, visible to every module.
    pub structs: Vec<StructDef>,
    pub typedefs: Vec<(String, DType)>,
}

/// An instruction set: `isa Name "desc" : opcode N [max M] { FORMAT = code "desc" { field : W [N]; } ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsaDef {
    pub name: String,
    pub desc: String,
    pub opcode_bits: Expr,
    pub max: Option<Expr>,
    pub formats: Vec<IsaFormat>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsaFormat {
    pub name: String,
    pub code: Expr,
    /// `like Other`: the fields of another format.
    pub like: Option<String>,
    pub fields: Vec<IsaField>,
    pub pos: Pos,
    pub desc: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsaField {
    pub name: String,
    pub desc: String,
    pub width: Expr,
    /// `[N]`: an array of N elements of `width` bits.
    pub count: Option<Expr>,
    pub pos: Pos,
}

/// A test: its name, the module it tests and its position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestDef {
    pub name: String,
    /// A name computed from a loop variable (`test t.name for ...` inside `for t in list`).
    pub name_expr: Option<Expr>,
    pub module: String,
    pub targs: Vec<Arg>,
    /// `timeout N`: the cycles the test may run.
    pub timeout: Option<Expr>,
    pub body: Vec<Stmt>,
    pub pos: Pos,
    /// The file the test is written in.
    pub origin: String,
}

/// `for t in list { test ... }`: tests made from the rows of a list (a manifest), one set per row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestLoop {
    pub var: String,
    pub iter: Expr,
    pub tests: Vec<TestDef>,
    pub loops: Vec<TestLoop>,
    pub pos: Pos,
}

/// A probe: its name and the module it is bound into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeDef {
    pub name: String,
    pub module: String,
    /// `when (cond)`.
    pub cond: Expr,
    /// `limit N`: at most N times per instance.
    pub limit: Option<Expr>,
    pub body: Vec<Stmt>,
    pub pos: Pos,
    /// The file the probe is written in.
    pub origin: String,
}

/// A trait: a set of function names (their signatures are checked where a module implements it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitDef {
    pub name: String,
    pub fns: Vec<String>,
    pub pos: Pos,
}

/// A configuration: `config Name [extends Base] { field = expr; ... }`. Fields are evaluated in order, so a field can
/// depend on the ones before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDef {
    pub name: String,
    pub base: Option<String>,
    pub fields: Vec<(String, Expr)>,
    pub pos: Pos,
    /// `config Name from "file.toml"`: the values come from a manifest, read relative to the source file.
    pub source: Option<String>,
    /// The file the config is written in.
    pub origin: String,
}

/// A bundle: its type parameters, its signals (from the master's point of view) and its methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleDef {
    pub name: String,
    pub params: Vec<Param>,
    pub fields: Vec<BundleField>,
    pub methods: Vec<ItemKind>,
    /// `monitor M<args>;`: a module that observes each port of this bundle (instantiated with it).
    pub monitors: Vec<DType>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleField {
    pub dir: Dir,
    pub dtype: DType,
    pub name: String,
    pub default: Option<Expr>,
}

/// A struct: its fields, first to last (the first field is the most significant), and its methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub name: String,
    /// Parameters (`struct AxiAddr<int AW = 32>`): each instance is laid out with its own arguments.
    pub params: Vec<Param>,
    pub fields: Vec<(DType, String)>,
    pub methods: Vec<ItemKind>,
    pub pos: Pos,
    /// `export struct`: a memory layout shared with software, laid out as memory is (first field lowest).
    pub export: bool,
    pub desc: String,
    /// The description of each field (`logic [31:0] src "source address";`).
    pub field_desc: std::collections::BTreeMap<String, String>,
    /// The file the struct is defined in (its SystemVerilog package goes there).
    pub origin: String,
}


/// A device of a memory map: `NAME @ base size n [irq k [= line]] [compatible "c"] [dt "props"] [if (c)] ["desc"];`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntryDef {
    pub name: String,
    pub base: Expr,
    pub size: Expr,
    pub irq: Option<Expr>,
    /// The interrupt signal of the device (`irq 10 = uart_irq`).
    pub line: Option<Expr>,
    pub compatible: String,
    pub dt: String,
    pub desc: String,
    pub cond: Option<Expr>,
    pub pos: Pos,
}
