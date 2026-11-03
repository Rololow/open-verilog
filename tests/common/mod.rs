//! Helpers shared by the integration tests. The simulations run with Verilator: on Windows through WSL (the distribution
//! `OVC_WSL_DISTRO`, `Ubuntu-24.04` by default), on Linux directly. Each test crate uses some of them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

/// The WSL distribution that has Verilator (Windows only).
fn distro() -> String {
    std::env::var("OVC_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu-24.04".into())
}

/// A Windows path as WSL sees it: `C:\x\y` is `/mnt/c/x/y`.
fn wsl_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        format!("/mnt/{}{}", (b[0] as char).to_ascii_lowercase(), &s[2..])
    } else {
        s
    }
}

/// Runs a tool in `dir` (through WSL on Windows), and returns its output.
fn run_tool(dir: &Path, program: &str, args: &[String]) -> Output {
    let res = if cfg!(windows) {
        Command::new("wsl.exe").args(["-d", &distro(), "--cd", &wsl_path(dir), "-e", program]).args(args).output()
    } else {
        Command::new(program).args(args).current_dir(dir).output()
    };
    res.unwrap_or_else(|e| panic!("{program}: {e}"))
}

/// The text of a run: standard output, then standard error.
fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// Whether Verilator runs here (in WSL on Windows).
fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| run_tool(&std::env::temp_dir(), "verilator", &["--version".to_string()]).status.success())
}

/// Whether the simulations of a test run. `OVC_SIM=off` skips them (a quick run of the suite), and `OVC_SIM=require`
/// makes a missing Verilator a failure, so that a job with Verilator cannot pass without running them. By default they
/// run when Verilator is available, and are skipped (with a note) when it is not.
pub fn simulator_enabled() -> bool {
    match std::env::var("OVC_SIM").as_deref() {
        Ok("off") => false,
        Ok("require") => {
            assert!(available(), "OVC_SIM=require, and Verilator is not available (WSL distribution {})", distro());
            true
        }
        _ => {
            if !available() {
                eprintln!("skipped: Verilator is not available");
                return false;
            }
            true
        }
    }
}

/// Whether a simulation output reports an error: a failed `$error` / `$fatal`, or a design that does not compile.
pub fn has_error(out: &str) -> bool {
    out.contains("%Error") || out.contains("** Error")
}

/// Verilator's arguments for the options a test gives: `-sv` is implied, `+define+A=B` is `-DA=B`, and the files are kept.
fn translate(args: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        if *a == "-sv" {
            continue;
        }
        match a.strip_prefix("+define+") {
            Some(defines) => out.extend(defines.split('+').filter(|d| !d.is_empty()).map(|d| format!("-D{d}"))),
            None => out.push(if cfg!(windows) { wsl_path(Path::new(a)) } else { a.to_string() }),
        }
    }
    out
}

/// Lints the files `args` name (as SystemVerilog) with Verilator, and returns its output. Fails when Verilator refuses
/// them.
pub fn lint_only(dir: &Path, args: &[&str]) -> String {
    let mut vargs: Vec<String> = vec!["--lint-only".into(), "-Wno-fatal".into()];
    vargs.extend(translate(args));
    let out = run_tool(dir, "verilator", &vargs);
    assert!(out.status.success(), "verilator {vargs:?} failed:\n{}", text(&out));
    text(&out)
}

/// Compiles the files `args` name (as SystemVerilog) with the top module `top`, and runs it: the output of both. The
/// simulation's own exit code is not checked: a failing check is reported in the output.
pub fn simulate(dir: &Path, args: &[&str], top: &str) -> String {
    simulate_with(dir, args, &[], top)
}

/// As `simulate`, with the parameters of the top module (`-gN=3`).
pub fn simulate_with(dir: &Path, args: &[&str], sim_options: &[&str], top: &str) -> String {
    // the C++ build is the slow part: unoptimised, and with the make jobs in parallel
    let mut vargs: Vec<String> = ["--binary", "--timing", "--assert", "-Wno-fatal", "-Wno-TIMESCALEMOD", "-O0", "-j", "0", "-CFLAGS", "-O0"].map(String::from).to_vec();
    vargs.extend(translate(args));
    vargs.extend(sim_options.iter().map(|o| o.replacen("-g", "-G", 1)));
    vargs.extend(["--top-module".to_string(), top.to_string()]);
    let build = run_tool(dir, "verilator", &vargs);
    if !build.status.success() {
        return text(&build);
    }
    let run = run_tool(dir, &format!("./obj_dir/V{top}"), &[]);
    format!("{}{}", text(&build), text(&run))
}

/// A scratch folder for one test, empty.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ovc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

/// The SystemVerilog of a design that compiles with no warning. A refused design panics with its rendered error.
pub fn sv_of(src: &str) -> String {
    let out = ovc::compile_full(src, "t.ov").unwrap_or_else(|e| panic!("{}", e.render(src)));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    out.sv
}

/// The message of a design that must be refused.
pub fn error_of(src: &str) -> String {
    match ovc::compile(src, "t.ov") {
        Ok(_) => panic!("the design must be refused:\n{src}"),
        Err(e) => e.message,
    }
}
