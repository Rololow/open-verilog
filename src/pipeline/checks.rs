//! Simulation-only checks of the "nothing is lost" contract, as SystemVerilog assertions.

use super::control::{Carried, Plan};
use crate::ast::Expr;
use crate::emit::expr::expr;
use crate::error::Result;

/// What the checks are about: the pipeline, its stages and its clock and reset.
pub struct Subject<'a> {
    pub name: &'a str,
    pub names: &'a [&'a str],
    pub clock: &'a str,
    pub rst: &'a str,
    pub has_valid_in: bool,
    /// Input ports read as data in a stage: they must hold while the pipeline is stalled.
    pub data_inputs: &'a [String],
}

fn text(e: &Expr) -> Result<String> {
    expr(e, -1, false)
}

/// The negation of an optional condition; `None` when there is no condition.
fn neg(e: &Option<Expr>) -> Result<Option<String>> {
    Ok(match e {
        None => None,
        Some(Expr::Ident(name, _)) => Some(format!("!{name}")),
        Some(e) => Some(format!("!({})", text(e)?)),
    })
}

fn conj(terms: Vec<Option<String>>) -> String {
    terms.into_iter().flatten().collect::<Vec<_>>().join(" && ")
}

/// The check lines, in the form the reference emits them.
pub fn checks(s: &Subject, plan: &Plan, valid: &[String], carried: &[Carried]) -> Result<Vec<String>> {
    let n = s.names.len();
    let clk = format!("@(posedge {}) disable iff ({})", s.clock, s.rst);
    let mut out = vec!["`ifndef SYNTHESIS".to_string(), format!("// ovc: pipeline checks for '{}' (nothing may be lost)", s.name)];
    for j in 1..n {
        let ante = conj(vec![Some(valid[j - 1].clone()), neg(&plan.stall[j - 1])?, neg(&plan.hold[j])?, neg(&plan.kill[j])?]);
        out.push(format!("ov_a_advance_{}: assert property ({clk} {ante} |=> {})", s.names[j], valid[j]));
        out.push(format!("  else $error(\"{}: an instruction leaving stage '{}' did not reach stage '{}'\");", s.name, s.names[j - 1], s.names[j]));
        if let Some(h) = &plan.hold[j] {
            let mut regs = vec![valid[j].clone()];
            regs.extend(carried.iter().filter(|c| c.owner < j && j <= c.last).map(|c| format!("{}__s{j}", c.name)));
            let stable = regs.iter().map(|r| format!("$stable({r})")).collect::<Vec<_>>().join(" && ");
            let cond = conj(vec![Some(text(h)?), neg(&plan.kill[j])?]);
            out.push(format!("ov_a_hold_{}: assert property ({clk} {cond} |=> {stable})", s.names[j]));
            out.push(format!("  else $error(\"{}: stage '{}' changed while it was stalled\");", s.name, s.names[j]));
        }
    }
    if let Some(blocked) = plan.blocked() {
        if s.has_valid_in || !s.data_inputs.is_empty() {
            let mut ante = Vec::new();
            let mut cons = Vec::new();
            if s.has_valid_in {
                ante.push("valid_in".to_string());
                cons.push("valid_in".to_string());
            }
            ante.push(text(&blocked)?);
            cons.extend(s.data_inputs.iter().map(|d| format!("$stable({d})")));
            out.push(format!("ov_a_input_held: assert property ({clk} {} |=> {})", ante.join(" && "), cons.join(" && ")));
            out.push(format!(
                "  else $error(\"{}: the producer changed valid_in or its data while the pipeline was stalled (ready_out = 0): that input is lost\");",
                s.name
            ));
        }
    }
    if n > 1 {
        out.push(format!("ov_a_valid_known: assert property ({clk} !$isunknown({{{}}}))", valid[1..].join(", ")));
        out.push(format!("  else $error(\"{}: a valid bit is X (missing reset?)\");", s.name));
    }
    out.push("`endif".to_string());
    Ok(out)
}
