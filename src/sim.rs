//! `ovc test`: runs the `test` blocks of a design with a SystemVerilog simulator. The design and the testbench (see
//! `testgen`) are written to a folder, the simulator compiles and runs them, and the log says which test passed.
//!
//! The simulators: `iverilog` (Icarus), `verilator` (`--binary`) and `questa` (`vlog` and `vsim`, from the PATH). The
//! testbench prints one `PASS` or `FAIL` line per test and a summary line; a test is passed only when the summary says so
//! and the simulator exited well.

use std::fs;
use std::path::Path;
use std::process::Command;

/// The simulators `ovc test` drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sim {
    Iverilog,
    Verilator,
    Questa,
}

impl Sim {
    pub fn from_name(name: &str) -> Option<Sim> {
        match name {
            "iverilog" => Some(Sim::Iverilog),
            "verilator" => Some(Sim::Verilator),
            "questa" => Some(Sim::Questa),
            _ => None,
        }
    }

    /// The first simulator found on the PATH, in order of preference.
    pub fn detect() -> Option<Sim> {
        [(Sim::Iverilog, "iverilog", "-V"), (Sim::Verilator, "verilator", "--version"), (Sim::Questa, "vsim", "-version")]
            .into_iter()
            .find(|(_, prog, flag)| Command::new(prog).arg(flag).output().is_ok())
            .map(|(sim, _, _)| sim)
    }
}

/// What the log of a run says: the tests that passed and failed, and the summary line when the run reached its end.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub passed: Vec<String>,
    pub failed: Vec<String>,
    /// (passed, failed) as the testbench counted them.
    pub counts: Option<(usize, usize)>,
}

impl Summary {
    /// Whether every test passed: the summary is there, says no failure, and no line reports one.
    pub fn all_passed(&self) -> bool {
        self.failed.is_empty() && matches!(self.counts, Some((_, 0)))
    }
}

/// The verdict lines of a log: `PASS ...`, `FAIL ...` (the test lines and the checks of the design), and the summary
/// `ovc tests: N passed, M failed`. Other lines (the simulator's, the `print` of a test) are not verdicts.
pub fn summarize(log: &str) -> Summary {
    let mut out = Summary::default();
    for line in log.lines() {
        // vsim prints its transcript with a `# ` in front of each line: the verdicts are read without it
        let t = line.trim_start_matches(|c: char| c == '#' || c.is_whitespace());
        if let Some(rest) = t.strip_prefix("ovc tests:") {
            let nums: Vec<usize> = rest.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();
            if let [passed, failed, ..] = nums[..] {
                out.counts = Some((passed, failed));
            }
        } else if t.starts_with("PASS") {
            out.passed.push(t.to_string());
        } else if t.starts_with("FAIL") {
            out.failed.push(t.to_string());
        }
    }
    out
}

/// The commands that compile and run the testbench with `sim`, in the folder `dir` (which holds `design.sv` and
/// `tb.sv`): each is a program and its arguments.
fn commands(sim: Sim) -> Vec<(&'static str, Vec<&'static str>)> {
    match sim {
        Sim::Iverilog => vec![
            ("iverilog", vec!["-g2012", "-s", "ovc_tests", "-o", "ovc_sim", "design.sv", "tb.sv"]),
            ("vvp", vec!["-n", "ovc_sim"]),
        ],
        Sim::Verilator => vec![
            ("verilator", vec!["--binary", "--timing", "-Wno-fatal", "--top-module", "ovc_tests", "design.sv", "tb.sv"]),
            ("obj_dir/Vovc_tests", vec![]),
        ],
        Sim::Questa => vec![
            ("vlib", vec!["work"]),
            ("vlog", vec!["-sv", "design.sv", "tb.sv"]),
            ("vsim", vec!["-c", "-quiet", "-do", "run -all; quit -f", "ovc_tests"]),
        ],
    }
}

/// The result of a run: the log of every command, and whether all of them exited well and every test passed.
pub struct Run {
    pub log: String,
    pub ok: bool,
}

/// Writes the design and the testbench to `dir`, and runs them with `sim`. A command that cannot start is an error (the
/// simulator is not on the PATH).
pub fn run(design_sv: &str, tb_sv: &str, sim: Sim, dir: &Path) -> Result<Run, String> {
    fs::create_dir_all(dir).map_err(|e| format!("ovc test: {}: {e}", dir.display()))?;
    fs::write(dir.join("design.sv"), design_sv).map_err(|e| format!("ovc test: design.sv: {e}"))?;
    fs::write(dir.join("tb.sv"), tb_sv).map_err(|e| format!("ovc test: tb.sv: {e}"))?;
    let mut log = String::new();
    let mut exited_well = true;
    for (program, args) in commands(sim) {
        let out = Command::new(program)
            .args(&args)
            .current_dir(dir)
            .output()
            .map_err(|e| format!("ovc test: cannot run {program}: {e} (is the simulator on the PATH?)"))?;
        log.push_str(&String::from_utf8_lossy(&out.stdout));
        log.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            exited_well = false;
            // a compile error is shown; the run after it is not started
            break;
        }
    }
    let ok = exited_well && summarize(&log).all_passed();
    Ok(Run { log, ok })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "PASS  holds_when_disabled  (Counter<4>)\nPASS  adds_with_carry  (Add)\nthe print of a test\novc tests: 2 passed, 0 failed\n";
    const BAD: &str = "PASS  adds_with_carry  (Add)\n  FAIL Add:12 [adds] expect(s == 510)\nFAIL  adds_with_carry  (Add): 1 check(s) failed\novc tests: 1 passed, 1 failed\n";

    #[test]
    fn a_log_of_passed_tests_is_all_passed() {
        let s = summarize(GOOD);
        assert_eq!(s.passed.len(), 2);
        assert!(s.failed.is_empty());
        assert!(s.all_passed());
    }

    #[test]
    fn a_failed_check_is_a_failure_whatever_the_summary_says() {
        let s = summarize(BAD);
        assert_eq!(s.failed.len(), 2);
        assert!(!s.all_passed());
        // a log without the summary line did not reach its end: not passed
        assert!(!summarize("PASS  a  (M)\n").all_passed());
    }

    #[test]
    fn the_transcript_of_questa_is_read_too() {
        let log = "# run -all
# PASS  counts and wraps  (examples/testbench.ov:13)
# ovc tests: 3 passed, 0 failed
";
        assert!(summarize(log).all_passed());
    }

    #[test]
    fn the_simulators_have_names() {
        assert_eq!(Sim::from_name("questa"), Some(Sim::Questa));
        assert_eq!(Sim::from_name("nope"), None);
    }
}
