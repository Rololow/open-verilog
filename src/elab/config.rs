//! Configurations: compile-time records of named values. A config's fields are evaluated in order, so a field can
//! depend on the ones before it; a `with` override replaces a field and the fields that follow are recomputed.

use std::collections::HashMap;

use super::env::Env;
use super::fold::{fold_expr, int_of};
use crate::ast::{ConfigDef, Expr};
use crate::error::{Error, Result};

/// A configuration given as an argument: its name and the fields overridden by `with`.
#[derive(Debug, Clone, Default)]
pub struct ConfigArg {
    pub name: String,
    pub overrides: Vec<(String, i64)>,
}

/// The fields of config `name` (base fields first, then its own), with the overrides applied, evaluated in order.
pub fn evaluate(name: &str, globals: &Env, overrides: &[(String, i64)], file: &str, pos: (usize, usize)) -> Result<Vec<(String, crate::ast::Expr)>> {
    let fail = |msg: String| Error::new(msg, file, pos.0, pos.1);
    // the field list, base first; a field defined again keeps its place and takes the new expression
    let mut chain = Vec::new();
    let mut current = Some(name.to_string());
    while let Some(n) = current {
        let def = globals.config_def(&n).ok_or_else(|| fail(format!("'{n}' is not a config")))?;
        chain.push(def.clone());
        current = def.base.clone();
        if chain.len() > 32 {
            return Err(fail(format!("config '{name}' extends itself")));
        }
    }
    let mut fields: Vec<(String, Expr)> = Vec::new();
    for def in chain.iter().rev() {
        for (f, e) in &def.fields {
            match fields.iter_mut().find(|(g, _)| g == f) {
                Some(slot) => slot.1 = e.clone(),
                None => fields.push((f.clone(), e.clone())),
            }
        }
    }
    for (f, _) in overrides {
        if !fields.iter().any(|(g, _)| g == f) {
            return Err(fail(format!("config '{name}' has no field '{f}'")));
        }
    }
    let mut env = globals.scope();
    let mut values = Vec::new();
    for (f, e) in &fields {
        // a field is a constant: an integer, or a string (a file name), which is kept as an expression
        let value = match overrides.iter().find(|(g, _)| g == f) {
            Some((_, v)) => Expr::Num(v.to_string()),
            None => fold_expr(e, &env),
        };
        if let Some(v) = int_of(&value) {
            env.bind_int(f, v);
        }
        values.push((f.clone(), value));
    }
    Ok(values)
}

/// The configs of a file, as a table for the environment.
pub fn table(defs: &[ConfigDef]) -> HashMap<String, ConfigDef> {
    defs.iter().map(|d| (d.name.clone(), d.clone())).collect()
}
