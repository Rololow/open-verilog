//! Compile-time bindings: integer values (parameters, loop variables) and type parameters.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use crate::ast::DType;

/// An enum after elaboration: its storage type and the value of each member.
#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub dtype: DType,
    pub members: Vec<(String, i64)>,
}

/// A pure function: its parameters, its return type and the returned expression.
#[derive(Debug, Clone)]
pub struct FnDef {
    pub params: Vec<String>,
    pub ret: Option<DType>,
    pub body: crate::ast::Expr,
}

/// A view: the signal it names, its instruction set and its format.
pub type ViewTable = HashMap<String, (String, String, String)>;

/// The type and unpacked dimensions of each signal of a module, by name.
pub type SignalTable = HashMap<String, (DType, Vec<crate::ast::Expr>)>;

/// Compile-time bindings. Scopes share the enum table and the record of the enums they used.
#[derive(Debug, Clone, Default)]
pub struct Env {
    ints: HashMap<String, i64>,
    types: HashMap<String, DType>,
    enums: Rc<BTreeMap<String, EnumInfo>>,
    used: Rc<RefCell<BTreeSet<String>>>,
    fns: Rc<HashMap<String, FnDef>>,
    signals: Rc<RefCell<SignalTable>>,
    reset: Option<ResetSpec>,
    reg_inits: Rc<RefCell<Vec<(String, crate::ast::Expr)>>>,
    /// The registers and the blocks of `.delay()` lines, placed by the module, and the number of lines so far.
    delay_decls: Rc<RefCell<Vec<crate::ast::Item>>>,
    /// The constant tables declared in the module.
    tables: Rc<RefCell<BTreeSet<String>>>,
    delay_blocks: Rc<RefCell<Vec<crate::ast::Item>>>,
    delay_count: Rc<Cell<usize>>,
    /// The names of the delay lines made so far (shared by the copies of the environment).
    delay_names: Rc<RefCell<BTreeSet<String>>>,
    structs: Rc<HashMap<String, StructInfo>>,
    /// Signal name -> the struct type it was declared with (only for struct-typed signals).
    struct_of: Rc<RefCell<HashMap<String, String>>>,
    /// Parameterized structs, by name, and the instances built so far (keyed by name and arguments).
    struct_defs: Rc<HashMap<String, crate::ast::StructDef>>,
    struct_instances: Rc<RefCell<HashMap<String, StructInfo>>>,
    /// The packages of the packed structs (origin file, text), shared by every scope.
    struct_packages: Rc<RefCell<Vec<(String, String)>>>,
    bundles: Rc<HashMap<String, crate::ast::BundleDef>>,
    configs: Rc<HashMap<String, crate::ast::ConfigDef>>,
    /// Config parameter -> its field values in this module.
    config_vals: HashMap<String, HashMap<String, crate::ast::Expr>>,
    /// Config parameter -> the configuration it is bound to (name and overrides).
    config_args: HashMap<String, super::config::ConfigArg>,
    /// Module parameter -> the module it is bound to.
    module_alias: HashMap<String, String>,
    isas: Rc<HashMap<String, super::isa::IsaInfo>>,
    /// View name -> (signal, instruction set, format): `val v = instr.as(Isa.FORMAT)`.
    views: Rc<RefCell<ViewTable>>,
    isa_used: Rc<RefCell<BTreeSet<String>>>,
    bundle_of: Rc<HashMap<String, BundleRef>>,
    /// Element signal of a vector of bundles -> (merged signal, index).
    vec_elements: Rc<HashMap<String, (String, usize)>>,
    /// The `val`s that name a bundle, by name.
    bundle_aliases: Rc<RefCell<HashMap<String, BundleRef>>>,
    /// `val` names whose value is a constant: compile-time, like parameters.
    consts: Rc<RefCell<HashMap<String, i64>>>,
    /// `val` names whose value is a list: the folded elements, bound in the module.
    lists: Rc<RefCell<HashMap<String, Vec<crate::ast::Expr>>>>,
    /// Top-level lists, visible to every module.
    global_lists: Rc<RefCell<HashMap<String, Vec<crate::ast::Expr>>>>,
    /// Top-level constants that are not integers or lists (a sized literal, say): substituted where they are named.
    global_exprs: Rc<RefCell<HashMap<String, crate::ast::Expr>>>,
    /// The non-integer parameters of the module (a string, say): substituted where they are named.
    exprs: Rc<RefCell<HashMap<String, crate::ast::Expr>>>,
    /// The memory maps of the module, by name.
    memmaps: Rc<RefCell<HashMap<String, MapVal>>>,
}

/// A memory map, placed: its devices, and the interrupt lines as one vector (`io.irqs`).
#[derive(Debug, Clone)]
pub struct MapVal {
    pub devices: Vec<MapDev>,
    pub irqs: crate::ast::Expr,
}

/// A device of a memory map: its region and its interrupt number.
#[derive(Debug, Clone)]
pub struct MapDev {
    pub name: String,
    pub base: i64,
    pub size: i64,
    pub irq: Option<i64>,
    /// The device tree's `compatible` string, its extra properties, and the description of the device.
    pub compatible: String,
    pub dt: String,
    pub desc: String,
}

/// A struct, flattened: its width, its fields (name, msb, lsb; the first field is the most significant) and
/// its methods.
#[derive(Debug, Clone)]
pub struct StructInfo {
    pub width: u32,
    pub fields: Vec<(String, i64, i64)>,
    pub methods: HashMap<String, FnDef>,
    /// Declared as a SystemVerilog packed struct (`pkg::name_t`): not parameterized, not exported, its struct fields
    /// are packed too.
    pub typed: bool,
    /// The fields that are structs: field name to the struct's key (its type is that struct's package type).
    pub field_types: HashMap<String, String>,
}

/// A bundle signal of a module: the prefix of its flattened signals, its fields with their directions in the
/// module, and the role of the port (`master` keeps the directions, `slave` flips them).
#[derive(Debug, Clone)]
pub struct BundleRef {
    pub bundle: String,
    pub prefix: String,
    pub fields: Vec<(String, crate::ast::Dir)>,
    pub role: crate::ast::Dir,
}

/// The module's reset: its port, its polarity and whether it is asynchronous.
#[derive(Debug, Clone)]
pub struct ResetSpec {
    pub name: String,
    pub low: bool,
    pub is_async: bool,
}

impl Env {
    /// Replaces the enums this scope knows (a module's own enums are added to the top-level ones).
    pub fn set_enums(&mut self, enums: Rc<BTreeMap<String, EnumInfo>>) {
        self.enums = enums;
    }

    /// An empty environment that knows the enums, with a fresh record of the enums used.
    pub fn with_enums(enums: Rc<BTreeMap<String, EnumInfo>>) -> Env {
        Env { enums, ..Env::default() }
    }

    /// A new scope: the same bindings and enums, with its own record of the enums used.
    pub fn scope(&self) -> Env {
        Env {
            used: Rc::default(),
            delay_decls: Rc::default(),
            tables: Rc::default(),
            delay_blocks: Rc::default(),
            delay_count: Rc::default(),
            delay_names: Rc::default(),
            views: Rc::default(),
            isa_used: Rc::default(),
            consts: Rc::default(),
            lists: Rc::default(),
            exprs: Rc::default(),
            memmaps: Rc::default(),
            ..self.clone()
        }
    }

    /// The ports and declarations of the module: type and unpacked dimensions.
    pub fn set_signals(&mut self, signals: SignalTable) {
        self.signals = Rc::new(RefCell::new(signals));
    }

    /// The type and unpacked dimensions of a signal (a clone: the table can grow meanwhile).
    pub fn signal(&self, name: &str) -> Option<(DType, Vec<crate::ast::Expr>)> {
        self.signals.borrow().get(name).cloned()
    }

    /// Records a signal defined while elaborating (a `val`), for the uses after it.
    pub fn define_signal(&self, name: &str, dtype: DType, dims: Vec<crate::ast::Expr>) {
        self.signals.borrow_mut().insert(name.to_string(), (dtype, dims));
    }

    /// The stem of the registers of a delay line on `root`: `root` itself when no other delay line has it, else a
    /// numbered one.
    pub fn delay_stem(&self, root: &str) -> String {
        let mut names = self.delay_names.borrow_mut();
        let stem = if names.contains(root) { format!("{root}_{}", self.next_delay_id()) } else { root.to_string() };
        names.insert(stem.clone());
        stem
    }

    pub fn next_delay_id(&self) -> usize {
        self.delay_count.set(self.delay_count.get() + 1);
        self.delay_count.get()
    }

    /// Declares a constant table (`localparam logic [W-1:0] NAME [N] = '{...}`) once in the module; false when its
    /// name is taken already.
    pub fn declare_table(&self, name: &str, width: u32, values: Vec<crate::ast::Expr>) -> bool {
        use crate::ast::{Declarator, DType, Decl, Expr, Item, ItemKind};
        if !self.tables.borrow_mut().insert(name.to_string()) {
            return true;
        }
        let dtype = DType { name: "logic".into(), packed: vec![(Expr::Num((width - 1).to_string()), Expr::Num("0".into()))], ..DType::default() };
        let len = values.len();
        let decl = Decl {
            dtype,
            decls: vec![Declarator { name: name.to_string(), dims: vec![Expr::Num(len.to_string())], init: Some(Expr::Pattern(values)) }],
            is_const: true,
            is_async: false,
        };
        self.delay_decls.borrow_mut().push(Item { pos: (0, 0), kind: ItemKind::Decl(decl) });
        true
    }

    pub fn push_delay_decl(&self, item: crate::ast::Item) {
        self.delay_decls.borrow_mut().push(item);
    }

    pub fn push_delay_block(&self, item: crate::ast::Item) {
        self.delay_blocks.borrow_mut().push(item);
    }

    /// A named wire holding `value` (an intermediate of a reduction), declared with its width.
    pub fn push_wire(&self, name: &str, width: u32, value: crate::ast::Expr) {
        use crate::ast::{Declarator, DType, Decl, Item, ItemKind};
        let dtype = DType { name: "logic".into(), packed: vec![(crate::ast::Expr::Num((width - 1).to_string()), crate::ast::Expr::Num("0".into()))], ..DType::default() };
        self.define_signal(name, dtype.clone(), Vec::new());
        let decl = Decl { dtype, decls: vec![Declarator { name: name.to_string(), dims: Vec::new(), init: None }], is_const: false, is_async: false };
        self.delay_decls.borrow_mut().push(Item { pos: (0, 0), kind: ItemKind::Decl(decl) });
        self.delay_blocks.borrow_mut().push(Item { pos: (0, 0), kind: ItemKind::ContAssign { lhs: crate::ast::Expr::Ident(name.to_string(), (0, 0)), rhs: value } });
    }

    pub fn take_delay_items(&self) -> (Vec<crate::ast::Item>, Vec<crate::ast::Item>) {
        (std::mem::take(&mut *self.delay_decls.borrow_mut()), std::mem::take(&mut *self.delay_blocks.borrow_mut()))
    }

    pub fn set_reset(&mut self, reset: Option<ResetSpec>, reg_inits: Vec<(String, crate::ast::Expr)>) {
        self.reset = reset;
        self.reg_inits = Rc::new(RefCell::new(reg_inits));
    }

    pub fn reset(&self) -> Option<&ResetSpec> {
        self.reset.as_ref()
    }

    /// The registers with an initial value: their reset value in the implicit `always_ff`.
    pub fn reg_inits(&self) -> Vec<(String, crate::ast::Expr)> {
        self.reg_inits.borrow().clone()
    }

    /// A register declared inside a generate block (under its final name): reset like the module's own.
    pub fn add_reg_init(&self, name: &str, init: crate::ast::Expr) {
        let mut inits = self.reg_inits.borrow_mut();
        if !inits.iter().any(|(n, _)| n == name) {
            inits.push((name.to_string(), init));
        }
    }

    pub fn set_structs(&mut self, structs: HashMap<String, StructInfo>) {
        self.structs = Rc::new(structs);
    }

    pub fn set_struct_defs(&mut self, defs: HashMap<String, crate::ast::StructDef>) {
        self.struct_defs = Rc::new(defs);
    }

    /// The definition of a parameterized struct.
    pub fn struct_def(&self, name: &str) -> Option<crate::ast::StructDef> {
        self.struct_defs.get(name).cloned()
    }

    /// Records an instance of a parameterized struct (its key names the struct and its arguments).
    pub fn insert_struct_instance(&self, key: &str, info: StructInfo) {
        self.struct_instances.borrow_mut().insert(key.to_string(), info);
    }

    /// The packed struct (plain or instance) whose SystemVerilog type is `ty` (`pixel_pkg::pixel_t`).
    pub fn typed_struct_named(&self, ty: &str) -> Option<StructInfo> {
        let is_it = |name: &str, s: &StructInfo| s.typed && crate::outputs::typedef_name(name) == ty;
        self.structs
            .iter()
            .find(|(name, s)| is_it(name, s))
            .map(|(_, s)| s.clone())
            .or_else(|| self.struct_instances.borrow().iter().find(|(name, s)| is_it(name, s)).map(|(_, s)| s.clone()))
    }

    /// The width of a packed struct type (`pixel_pkg::pixel_t`), when `ty` names one.
    pub fn typedef_width(&self, ty: &str) -> Option<u32> {
        self.typed_struct_named(ty).map(|s| s.width)
    }

    /// Records the package of a packed struct, in the order the structs are built (a struct's fields come first).
    pub fn push_package(&self, origin: &str, text: String) {
        self.struct_packages.borrow_mut().push((origin.to_string(), text));
    }

    /// The packages of the packed structs, in the order they were built.
    pub fn packages(&self) -> Vec<(String, String)> {
        self.struct_packages.borrow().clone()
    }

    pub fn struct_info(&self, name: &str) -> Option<StructInfo> {
        self.structs.get(name).cloned().or_else(|| self.struct_instances.borrow().get(name).cloned())
    }

    pub fn set_configs(&mut self, configs: HashMap<String, crate::ast::ConfigDef>) {
        self.configs = Rc::new(configs);
    }

    pub fn config_def(&self, name: &str) -> Option<&crate::ast::ConfigDef> {
        self.configs.get(name)
    }

    pub fn set_isas(&mut self, isas: HashMap<String, super::isa::IsaInfo>) {
        self.isas = Rc::new(isas);
    }

    pub fn isa_info(&self, name: &str) -> Option<&super::isa::IsaInfo> {
        self.isas.get(name)
    }

    pub fn define_view(&self, name: &str, signal: &str, isa: &str, format: &str) {
        self.views.borrow_mut().insert(name.to_string(), (signal.to_string(), isa.to_string(), format.to_string()));
    }

    pub fn view(&self, name: &str) -> Option<(String, String, String)> {
        self.views.borrow().get(name).cloned()
    }

    /// Records that an instruction set's length table is used in this module.
    pub fn note_isa(&self, name: &str) {
        self.isa_used.borrow_mut().insert(name.to_string());
    }

    pub fn used_isas(&self) -> BTreeSet<String> {
        self.isa_used.borrow().clone()
    }

    pub fn bind_module(&mut self, param: &str, module: &str) {
        self.module_alias.insert(param.to_string(), module.to_string());
    }

    /// The module a name stands for: a module parameter is bound to a module, any other name is itself.
    pub fn module_alias(&self, name: &str) -> Option<&str> {
        self.module_alias.get(name).map(String::as_str)
    }

    pub fn bind_config(&mut self, param: &str, arg: super::config::ConfigArg, values: Vec<(String, crate::ast::Expr)>) {
        self.config_vals.insert(param.to_string(), values.into_iter().collect());
        self.config_args.insert(param.to_string(), arg);
    }

    /// The configuration a parameter is bound to, with its overrides: an instance that takes the parameter takes it too.
    pub fn config_arg(&self, param: &str) -> Option<super::config::ConfigArg> {
        self.config_args.get(param).cloned()
    }

    /// The value of a field of a config parameter (`C.xlen`).
    pub fn config_field(&self, param: &str, field: &str) -> Option<crate::ast::Expr> {
        self.config_vals.get(param)?.get(field).cloned()
    }

    pub fn set_bundles(&mut self, bundles: HashMap<String, crate::ast::BundleDef>) {
        self.bundles = Rc::new(bundles);
    }

    pub fn bundle_def(&self, name: &str) -> Option<&crate::ast::BundleDef> {
        self.bundles.get(name)
    }

    /// The element signals of the vectors of bundles of the module (element -> merged signal and index).
    pub fn set_vec_elements(&mut self, elements: HashMap<String, (String, usize)>) {
        self.vec_elements = Rc::new(elements);
    }

    /// The merged signal and the index an element signal of a vector of bundles is.
    pub fn vec_element(&self, name: &str) -> Option<(String, usize)> {
        self.vec_elements.get(name).cloned()
    }

    pub fn set_bundle_of(&mut self, bundle_of: HashMap<String, BundleRef>) {
        self.bundle_of = Rc::new(bundle_of);
        // the aliases of a module are its own
        self.bundle_aliases = Rc::default();
    }

    /// The bundle a signal of the module belongs to, if it is one (a copy).
    pub fn bundle_of(&self, signal: &str) -> Option<BundleRef> {
        self.bundle_aliases.borrow().get(signal).cloned().or_else(|| self.bundle_of.get(signal).cloned())
    }

    /// A `val` that names a bundle (an element of a vector of bundles): its signals are those of the element.
    pub fn define_bundle_alias(&self, name: &str, bundle: BundleRef) {
        self.bundle_aliases.borrow_mut().insert(name.to_string(), bundle);
    }

    pub fn set_struct_of(&mut self, struct_of: HashMap<String, String>) {
        self.struct_of = Rc::new(RefCell::new(struct_of));
    }

    /// A signal made at elaboration (a `val` view inside a generate loop, under its final name) read as a struct.
    pub fn add_struct_of(&self, signal: &str, key: String) {
        self.struct_of.borrow_mut().insert(signal.to_string(), key);
    }

    /// The struct a signal was declared with, if any.
    pub fn struct_of(&self, signal: &str) -> Option<StructInfo> {
        let key = self.struct_of.borrow().get(signal).cloned()?;
        self.struct_info(&key)
    }

    /// The storage type of an enum member named as `Enum_MEMBER` (the name it has once folded), when it is one.
    pub fn enum_member_type(&self, qualified: &str) -> Option<DType> {
        self.enums
            .iter()
            .find(|(e, info)| qualified.strip_prefix(&format!("{e}_")).is_some_and(|m| info.members.iter().any(|(x, _)| x == m)))
            .map(|(_, info)| info.dtype.clone())
    }

    /// The enum that has a member of this name, for the members used bare after `import`.
    pub fn enum_of_member(&self, member: &str) -> Option<String> {
        self.enums.iter().find(|(_, info)| info.members.iter().any(|(m, _)| m == member)).map(|(name, _)| name.clone())
    }

    pub fn set_fns(&mut self, fns: HashMap<String, FnDef>) {
        self.fns = Rc::new(fns);
    }

    pub fn fns(&self) -> HashMap<String, FnDef> {
        (*self.fns).clone()
    }

    pub fn fn_def(&self, name: &str) -> Option<&FnDef> {
        self.fns.get(name)
    }

    /// The same scope without one function: a function is not inlined into its own body.
    pub fn without_fn(&self, name: &str) -> Env {
        let mut fns = (*self.fns).clone();
        fns.remove(name);
        Env { fns: Rc::new(fns), ..self.clone() }
    }

    pub fn enum_info(&self, name: &str) -> Option<&EnumInfo> {
        self.enums.get(name)
    }

    /// Records that an enum is used, so its members are declared in the module.
    pub fn note_enum(&self, name: &str) {
        self.used.borrow_mut().insert(name.to_string());
    }

    pub fn used_enums(&self) -> BTreeSet<String> {
        self.used.borrow().clone()
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.ints.get(name).copied().or_else(|| self.consts.borrow().get(name).copied())
    }

    /// A `val` whose value is a constant.
    pub fn define_const(&self, name: &str, value: i64) {
        self.consts.borrow_mut().insert(name.to_string(), value);
    }

    /// A `val` whose value is a list of folded elements.
    pub fn define_list(&self, name: &str, items: Vec<crate::ast::Expr>) {
        self.lists.borrow_mut().insert(name.to_string(), items);
    }

    /// A top-level `val` list, visible to every module.
    pub fn define_global_list(&self, name: &str, items: Vec<crate::ast::Expr>) {
        self.global_lists.borrow_mut().insert(name.to_string(), items);
    }

    pub fn define_memmap(&self, name: &str, map: MapVal) {
        self.memmaps.borrow_mut().insert(name.to_string(), map);
    }

    /// `map.DEV` (the device's index), `map.count`, `map.regions` (the [base, size] pairs) or `map.irqs`.
    pub fn memmap_member(&self, map: &str, name: &str) -> Option<crate::ast::Expr> {
        use crate::ast::Expr;
        let maps = self.memmaps.borrow();
        let m = maps.get(map)?;
        Some(match name {
            "count" => Expr::Num(m.devices.len().to_string()),
            "regions" => Expr::List(
                m.devices
                    .iter()
                    .map(|d| Expr::List(vec![Expr::Num(d.base.to_string()), Expr::Num(d.size.to_string())]))
                    .collect(),
            ),
            "irqs" => m.irqs.clone(),
            _ => {
                let index = m.devices.iter().position(|d| d.name == name)?;
                Expr::Num(index.to_string())
            }
        })
    }

    /// `map.DEV.base`, `.size` or `.irq`.
    pub fn memmap_field(&self, map: &str, dev: &str, field: &str) -> Option<crate::ast::Expr> {
        let maps = self.memmaps.borrow();
        let d = maps.get(map)?.devices.iter().find(|d| d.name == dev)?;
        let v = match field {
            "base" => d.base,
            "size" => d.size,
            "irq" => d.irq?,
            _ => return None,
        };
        Some(crate::ast::Expr::Num(v.to_string()))
    }

    /// A top-level constant that is an expression, substituted where it is named.
    pub fn define_global_expr(&self, name: &str, value: crate::ast::Expr) {
        self.global_exprs.borrow_mut().insert(name.to_string(), value);
    }

    pub fn global_expr(&self, name: &str) -> Option<crate::ast::Expr> {
        self.exprs.borrow().get(name).cloned().or_else(|| self.global_exprs.borrow().get(name).cloned())
    }

    /// The entries of a manifest config, without the config's name: `array.k` for `V3.array.k`.
    pub fn manifest_entries(&self, config: &str) -> Vec<(String, crate::ast::Expr)> {
        let prefix = format!("{config}.");
        self.global_exprs.borrow().iter().filter_map(|(k, v)| k.strip_prefix(&prefix).map(|rest| (rest.to_string(), v.clone()))).collect()
    }

    /// A parameter of the module that is not an integer.
    pub fn define_expr(&self, name: &str, value: crate::ast::Expr) {
        self.exprs.borrow_mut().insert(name.to_string(), value);
    }

    /// The elements of a list `val`, when `name` is one.
    pub fn list(&self, name: &str) -> Option<Vec<crate::ast::Expr>> {
        self.lists.borrow().get(name).cloned().or_else(|| self.global_lists.borrow().get(name).cloned())
    }

    pub fn ty(&self, name: &str) -> Option<&DType> {
        self.types.get(name)
    }

    pub fn bind_int(&mut self, name: &str, value: i64) {
        self.ints.insert(name.to_string(), value);
    }

    pub fn bind_type(&mut self, name: &str, ty: DType) {
        self.types.insert(name.to_string(), ty);
    }

    /// A name declared again inside a scope (a loop variable) hides the outer binding.
    pub fn unbind(&mut self, name: &str) {
        self.ints.remove(name);
        self.types.remove(name);
    }
}
