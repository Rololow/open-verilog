//! Bundles: a group of signals with directions, flattened into ports (`out_valid`, `out_ready`, ...) and wires.
//! A `master` port keeps the directions of the bundle, a `slave` port flips them. The valid/ready protocol gets
//! generated assertions (simulation only), as the reference does.

use super::env::{BundleRef, Env};
use super::fold::{fold_dtype, fold_expr, int_of};
use crate::ast::{Arg, BundleDef, Conn, Decl, Declarator, Dir, DType, Expr, Pos, Port};
use crate::error::{Error, Result};

/// The environment of a bundle instance: its type and value parameters bound to the given arguments (or defaults).
pub fn bind(def: &BundleDef, targs: &[Arg], env: &Env, file: &str, pos: (usize, usize)) -> Result<Env> {
    let mut bound = env.scope();
    for (k, param) in def.params.iter().enumerate() {
        let given = targs.get(k).or(param.default.as_ref());
        match (param.kind.as_str(), given) {
            ("type", Some(Arg::Type(t))) => match super::fold::struct_key(t, env) {
                // a struct stays a struct (its signals keep their fields)
                Some(key) => bound.bind_type(&param.name, DType { name: key, ..DType::default() }),
                None => bound.bind_type(&param.name, fold_dtype(t, env)),
            },
            // a type parameter of the module that instantiates the bundle, as in Stream<T> in module M<type T>

            // a struct name as the argument: a struct type
            ("type", Some(Arg::Expr(Expr::Ident(n, _)))) if super::fold::struct_key(&DType { name: n.clone(), ..DType::default() }, env).is_some() => {
                let key = super::fold::struct_key(&DType { name: n.clone(), ..DType::default() }, env).unwrap_or_default();
                bound.bind_type(&param.name, DType { name: key, ..DType::default() });
            }
            ("type", Some(Arg::Expr(Expr::Ident(n, _)))) if env.ty(n).is_some() => {
                let t = env.ty(n).cloned().expect("checked");
                bound.bind_type(&param.name, t);
            }
            ("int" | "bool", Some(Arg::Expr(e))) => {
                let v = int_of(&fold_expr(e, env)).ok_or_else(|| {
                    Error::new(format!("the value of parameter '{}' of bundle '{}' is not a constant", param.name, def.name), file, pos.0, pos.1)
                })?;
                bound.bind_int(&param.name, v);
            }
            _ => {
                return Err(Error::new(format!("bundle '{}' needs a value for parameter '{}'", def.name, param.name), file, pos.0, pos.1));
            }
        }
    }
    Ok(bound)
}

fn flip(dir: Dir) -> Dir {
    match dir {
        Dir::Input => Dir::Output,
        Dir::Output => Dir::Input,
        other => other,
    }
}

/// The signals of a bundle port: the flattened ports, the references, and the struct types of the signals.
pub type Expanded = (Vec<Port>, Vec<(String, BundleRef)>, Vec<(String, String)>);
/// The references of a bundle signal (the signal first, then the bundles nested in it), and the struct types.
pub type Refs = (Vec<(String, BundleRef)>, Vec<(String, String)>);

/// The end of a bundle a port is: `master` keeps the directions, `slave` flips them, `monitor` observes (every
/// signal is an input).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Master,
    Slave,
    Monitor,
}

/// The flattened ports of a bundle port (signals of the bundles nested in it included), and the references the
/// body uses to reach them: the port first, then each nested bundle.
pub fn expand_port(def: &BundleDef, p: &Port, env: &Env, file: &str) -> Result<Expanded> {
    let side = match p.dir {
        Dir::Master => Side::Master,
        Dir::Slave => Side::Slave,
        Dir::Monitor => Side::Monitor,
        other => {
            return Err(Error::new(format!("a bundle port needs the role 'master', 'slave' or 'monitor', not '{other:?}'"), file, p.pos.0, p.pos.1));
        }
    };
    let mut ports = Vec::new();
    let mut refs = Vec::new();
    let mut structs = Vec::new();
    let fields = flatten(def, &p.dtype.targs, &p.name, side, side == Side::Slave, env, file, p.pos, &mut ports, &mut refs, &mut structs)?;
    let role = match side {
        Side::Master => Dir::Master,
        Side::Slave => Dir::Slave,
        Side::Monitor => Dir::Monitor,
    };
    refs.insert(0, (p.name.clone(), BundleRef { bundle: def.name.clone(), prefix: p.name.clone(), fields, role }));
    Ok((ports, refs, structs))
}

/// The ports of one bundle under `prefix`. `flipped` says whether the directions are flipped at this level (a
/// slave, or inside an input of a slave's or a master's bundle). A nested bundle's references go to `refs`.
#[allow(clippy::too_many_arguments)]
fn flatten(
    def: &BundleDef,
    targs: &[Arg],
    prefix: &str,
    side: Side,
    flipped: bool,
    env: &Env,
    file: &str,
    pos: Pos,
    ports: &mut Vec<Port>,
    refs: &mut Vec<(String, BundleRef)>,
    structs: &mut Vec<(String, String)>,
) -> Result<Vec<(String, Dir)>> {
    let bound = bind(def, targs, env, file, pos)?;
    let mut fields = Vec::new();
    for f in &def.fields {
        let dir = match side {
            Side::Monitor => Dir::Input,
            _ if flipped => flip(f.dir),
            _ => f.dir,
        };
        let name = format!("{prefix}_{}", f.name);
        if let Some(inner) = env.bundle_def(&f.dtype.name).cloned() {
            // a bundle nested in the bundle: its signals are named after this one
            let sub = flatten(&inner, &f.dtype.targs, &name, side, flipped ^ (f.dir == Dir::Input), &bound, file, pos, ports, refs, structs)?;
            let role = match side {
                Side::Master => Dir::Master,
                Side::Slave => Dir::Slave,
                Side::Monitor => Dir::Monitor,
            };
            refs.push((name.clone(), BundleRef { bundle: inner.name.clone(), prefix: name.clone(), fields: sub, role }));
            fields.push((f.name.clone(), dir));
            continue;
        }
        if let Some(key) = super::fold::struct_key(&f.dtype, &bound) {
            structs.push((name.clone(), key));
        }
        let (dtype, extra) = super::types::lower(fold_dtype(&f.dtype, &bound)).map_err(|e| Error::new(e, file, pos.0, pos.1))?;
        // an input may keep its idle value as a default; an output's value is not a default. A bundle that declares
        // idle values gives its other scalar inputs 0 (as the reference does)
        let declares_idle = def.fields.iter().any(|g| g.default.is_some());
        let default = match (dir, &f.default) {
            (Dir::Input, Some(d)) => Some(fold_expr(d, &bound)),
            (Dir::Input, None) if declares_idle && vec_of(&f.dtype).is_none() => Some(Expr::Num("0".into())),
            _ => None,
        };
        ports.push(Port { dir, dtype, name, dims: extra, cond: None, is_async: false, default, pos });
        fields.push((f.name.clone(), dir));
    }
    Ok(fields)
}

/// The monitors of a bundle port: each module with its arguments (bound in the bundle's own parameters).
pub fn monitors(def: &BundleDef, targs: &[Arg], env: &Env, file: &str, pos: Pos) -> Result<Vec<(String, Vec<Arg>)>> {
    let bound = bind(def, targs, env, file, pos)?;
    Ok(def
        .monitors
        .iter()
        .map(|m| {
            let args = m
                .targs
                .iter()
                .map(|a| match a {
                    Arg::Expr(e) => Arg::Expr(fold_expr(e, &bound)),
                    Arg::Type(t) => Arg::Type(t.clone()),
                })
                .collect();
            (m.name.clone(), args)
        })
        .collect())
}

/// The reference of a bundle declared as a local signal (`Stream<T> link;`).
/// The references of a local bundle signal `name` (the bundles nested in it included), and the struct types of its
/// signals.
pub fn local_refs(def: &BundleDef, targs: &[Arg], name: &str, env: &Env, file: &str, pos: Pos) -> Result<Refs> {
    let mut ports = Vec::new();
    let mut refs = Vec::new();
    let mut structs = Vec::new();
    let fields = flatten(def, targs, name, Side::Master, false, env, file, pos, &mut ports, &mut refs, &mut structs)?;
    refs.insert(0, (name.to_string(), BundleRef { bundle: def.name.clone(), prefix: name.to_string(), fields, role: Dir::Master }));
    Ok((refs, structs))
}

/// Wires of a local bundle declaration: one per signal of the bundle (nested bundles included).
pub fn expand_decl(def: &BundleDef, d: &Decl, env: &Env, file: &str, pos: (usize, usize)) -> Result<Vec<Decl>> {
    let mut out = Vec::new();
    for x in &d.decls {
        let mut ports = Vec::new();
        let mut refs = Vec::new();
        let mut structs = Vec::new();
        flatten(def, &d.dtype.targs, &x.name, Side::Master, false, env, file, pos, &mut ports, &mut refs, &mut structs)?;
        for port in ports {
            let mut dims = port.dims;
            dims.extend(x.dims.iter().map(|e| fold_expr(e, env)));
            out.push(Decl { dtype: port.dtype, decls: vec![Declarator { name: port.name, dims, init: None }], is_const: false, is_async: false });
        }
    }
    Ok(out)
}

/// `Vec<B, N>`: the element type and the count (an expression), when the type is a vector of bundles.
pub fn vec_of(dtype: &DType) -> Option<(DType, Expr)> {
    if dtype.name != "Vec" {
        return None;
    }
    let (Some(elem), Some(Arg::Expr(count))) = (dtype.targs.first(), dtype.targs.get(1)) else { return None };
    let elem = match elem {
        Arg::Type(t) => t.clone(),
        Arg::Expr(Expr::Ident(n, _)) => DType { name: n.clone(), ..DType::default() },
        Arg::Expr(_) => return None,
    };
    Some((elem, count.clone()))
}

/// The number of elements of a vector of bundles: the bundles `<prefix>_0`, `<prefix>_1`, ... that exist.
pub fn element_count(env: &Env, prefix: &str) -> usize {
    (0..).take_while(|k| env.bundle_of(&format!("{prefix}_{k}")).is_some()).count()
}

/// The signal prefixes of a bundle-valued expression, one per element: `x` (a bundle), `x_k` (an element of a vector,
/// `x[k]`), the elements of a vector `x`, and the nested bundle `x.f` (of each element of a vector).
/// Whether an expression is a signal of a vector of bundles, or a part of one (`v`, `v.f`, `v.f.g`).
pub fn rooted_in_vector(expr: &Expr, env: &Env) -> bool {
    match expr {
        Expr::Ident(x, _) => element_count(env, x) > 0,
        Expr::Member(base, _, _) | Expr::Index(base, _) => rooted_in_vector(base, env),
        _ => false,
    }
}

/// The signal prefixes of a bundle-valued expression, one per element (see `expand_conns`).
pub fn bundle_prefixes(expr: &Expr, env: &Env) -> Option<Vec<String>> {
    let elements = |v: &str| -> Vec<String> { (0..element_count(env, v)).map(|k| format!("{v}_{k}")).collect() };
    match expr {
        Expr::Ident(x, _) if element_count(env, x) > 0 => Some(elements(x)),
        Expr::Ident(x, _) => env.bundle_of(x).map(|_| vec![x.clone()]),
        Expr::Index(base, idx) => {
            let Expr::Ident(x, _) = &**base else { return None };
            let k = int_of(&fold_expr(idx, env))?;
            let name = format!("{x}_{k}");
            env.bundle_of(&name).map(|_| vec![name])
        }
        Expr::Member(base, f, _) => {
            let parents = match &**base {
                Expr::Ident(x, _) if element_count(env, x) > 0 => elements(x),
                Expr::Ident(x, _) => vec![x.clone()],
                other => bundle_prefixes(other, env)?,
            };
            let nested: Vec<String> = parents.iter().map(|p| format!("{p}_{f}")).collect();
            nested.iter().all(|n| env.bundle_of(n).is_some()).then_some(nested)
        }
        _ => None,
    }
}

/// The signals a master of the bundle `prefix` drives, with the idle value of each (the field's default, or 0):
/// what `tie_off` assigns. The bundles nested in it are walked.
pub fn idle_outputs(env: &Env, prefix: &str, out: &mut Vec<(String, Expr)>) {
    let Some(b) = env.bundle_of(prefix) else { return };
    let def = env.bundle_def(&b.bundle);
    for (f, dir) in &b.fields {
        let name = format!("{prefix}_{f}");
        if env.bundle_of(&name).is_some() {
            idle_outputs(env, &name, out);
        } else if *dir == Dir::Output {
            let idle = def
                .and_then(|d| d.fields.iter().find(|x| &x.name == f))
                .and_then(|x| x.default.as_ref())
                .map_or_else(|| Expr::Num("0".into()), |e| fold_expr(e, env));
            out.push((name, idle));
        }
    }
}

/// The names of the signals of the bundle `prefix` below it, the nested bundles flattened (`aw_valid`, ...).
fn leaf_suffixes(env: &Env, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(b) = env.bundle_of(prefix) {
        for (f, _) in &b.fields {
            let nested = format!("{prefix}_{f}");
            if env.bundle_of(&nested).is_some() {
                out.extend(leaf_suffixes(env, &nested).into_iter().map(|l| format!("{f}_{l}")));
            } else {
                out.push(f.clone());
            }
        }
    }
    out
}

/// A connection of a bundle port of the target module (`.out(link)`) becomes one connection per signal. A vector
/// port takes one bundle per element; a single port takes one bundle.
/// `spec` is the elaborated child: its ports are the merged signals of a vector of bundles (`port_valid [N-1:0]`).
pub fn expand_conns(target: Option<&crate::ast::Module>, spec: Option<&crate::ast::Module>, def_of: &dyn Fn(&str) -> Option<BundleDef>, conns: Vec<Conn>, env: &Env, file: &str, pos: (usize, usize)) -> Result<Vec<Conn>> {
    let mut out = Vec::new();
    for c in conns {
        let Some(pname) = c.port.clone() else {
            out.push(c);
            continue;
        };
        let Some(port) = target.and_then(|t| t.ports.iter().find(|p| p.name == pname)) else {
            out.push(c);
            continue;
        };
        let (def_name, is_vec) = match vec_of(&port.dtype) {
            Some((elem, _)) => (elem.name, true),
            None => (port.dtype.name.clone(), false),
        };
        let Some(def) = def_of(&def_name) else {
            out.push(c);
            continue;
        };
        let fail = |msg: String| Error::new(msg, file, pos.0, pos.1);
        let Some(expr) = &c.expr else {
            return Err(fail(format!("the bundle port '{pname}' needs a bundle signal to connect to")));
        };
        let Some(prefixes) = bundle_prefixes(expr, env) else {
            return Err(fail(format!("the bundle port '{pname}' needs a bundle signal to connect to")));
        };
        // the signals of a bundle, the nested bundles flattened: `valid`, `aw_valid`, ... (the source's own layout)
        let _ = &def;
        if is_vec {
            // each field of the vector is one signal: the elements' fields are its bits (a packed vector) or its
            // elements (an unpacked array), in order
            let Some(first) = prefixes.first() else { return Err(fail(format!("the bundle port '{pname}' needs a vector of bundles"))) };
            for leaf in leaf_suffixes(env, first) {
                let child = format!("{pname}_{leaf}");
                let packed = spec.and_then(|m| m.ports.iter().find(|p| p.name == child)).is_none_or(|p| p.dims.is_empty());
                let elements: Vec<Expr> = prefixes.iter().map(|prefix| Expr::Ident(format!("{prefix}_{leaf}"), pos)).collect();
                let expr = if packed { Expr::Concat(elements.into_iter().rev().collect()) } else { Expr::Pattern(elements) };
                out.push(Conn { port: Some(child), expr: Some(expr) });
            }
        } else {
            let [prefix] = prefixes.as_slice() else {
                return Err(fail(format!("the bundle port '{pname}' takes one bundle, not a vector")));
            };
            for leaf in leaf_suffixes(env, prefix) {
                out.push(Conn { port: Some(format!("{pname}_{leaf}")), expr: Some(Expr::Ident(format!("{prefix}_{leaf}"), pos)) });
            }
        }
    }
    Ok(out)
}

/// The generated valid/ready checks of a handshake port: its valid must stay high, with stable data, until ready.
pub fn handshake_checks(module: &str, port: &str, role: Dir, fields: &[String], asserts: bool) -> Vec<String> {
    let has = |n: &str| fields.iter().any(|f| f == n);
    if !asserts || !(has("valid") && has("ready")) {
        return Vec::new();
    }
    let data: Vec<String> = fields.iter().filter(|f| *f != "valid" && *f != "ready").map(|f| format!("$stable({port}_{f})")).collect();
    let stable = if data.is_empty() { String::new() } else { format!(" && {}", data.join(" && ")) };
    let who = match role {
        Dir::Slave => "the producer connected to it withdrew or changed it",
        _ => "it withdrew or changed it",
    };
    let held_msg = format!(
        "{module}: {port} offered a value and {who} before it was accepted (valid must stay high, with stable data, until ready)"
    );
    let trap = "`ifdef OVC_TESTS ->ovc_tests.ovc_check_failed; `endif";
    vec![
        "`ifndef SYNTHESIS".to_string(),
        "// ovc: valid/ready protocol checks".to_string(),
        format!("ov_hs_{port}_held: assert property (@(posedge clk) disable iff (rst) {port}_valid && !{port}_ready |=> {port}_valid{stable})"),
        format!("  else begin $error(\"{held_msg}\"); {trap} end"),
        format!("ov_hs_{port}_known: assert property (@(posedge clk) disable iff (rst) !$isunknown({port}_valid))"),
        format!("  else begin $error(\"{module}: {port} valid is X\"); {trap} end"),
        "`endif".to_string(),
    ]
}
