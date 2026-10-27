//! Helpers shared by the integration tests: the Questa simulator, when it is installed. Each test crate uses some of them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Whether the program is on the PATH (`where` on Windows, `which` elsewhere).
pub fn on_path(program: &str) -> bool {
    let finder = if cfg!(windows) { "where" } else { "which" };
    Command::new(finder).arg(program).output().map(|o| o.status.success()).unwrap_or(false)
}

/// Runs one Questa command in `dir`, and returns its standard output (the command must succeed).
pub fn questa(dir: &Path, program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).current_dir(dir).output().unwrap_or_else(|e| panic!("{program}: {e}"));
    assert!(out.status.success(), "{program} {args:?} failed:\n{}", String::from_utf8_lossy(&out.stdout));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Compiles the files (`vlog` with `args`, which name them) into a fresh library and runs `top` (`vsim -c`): its
/// standard output. The simulation's own exit code is not checked: a failing check is reported in the output.
pub fn simulate(dir: &Path, args: &[&str], top: &str) -> String {
    simulate_with(dir, args, &[], top)
}

/// As `simulate`, with options for `vsim` before the top module (the generics: `-gN=3`).
pub fn simulate_with(dir: &Path, args: &[&str], vsim_options: &[&str], top: &str) -> String {
    // the Questa licence allows one run at a time: the tests of a binary run in parallel, so the runs wait for each other
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    questa(dir, "vlib", &["work"]);
    questa(dir, "vlog", args);
    let mut vsim: Vec<&str> = vec!["-c"];
    vsim.extend_from_slice(vsim_options);
    vsim.extend_from_slice(&[top, "-do", "run -all; quit -f"]);
    // the nodelocked licence refuses a second session while one is still closing: that is retried after a pause
    let mut out = String::new();
    for attempt in 0..5 {
        let run = Command::new("vsim").args(&vsim).current_dir(dir).output().expect("vsim runs");
        out = String::from_utf8_lossy(&run.stdout).into_owned();
        if !out.contains("License checkout has been disallowed") || attempt == 4 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    out
}

/// A scratch folder for one test, empty.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ovc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}
