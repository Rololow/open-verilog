//! The `[lints]` table of `ovc.toml` through the command line: the binary reads the manifest of the working directory,
//! and the flags given on the command line override only the categories they name.

use std::fs;
use std::path::Path;
use std::process::Command;

mod common;

/// A folder with a design that loses bits (an 8-bit value assigned to 4 bits: a `width` finding), and optionally a
/// manifest with the given text.
fn project(name: &str, manifest: Option<&str>) -> std::path::PathBuf {
    let dir = common::scratch(name);
    fs::write(
        dir.join("a.ov"),
        "module Top(input logic [7:0] a, output logic [3:0] y) {\n  assign y = a;\n}\n",
    )
    .unwrap();
    if let Some(text) = manifest {
        fs::write(dir.join("ovc.toml"), text).unwrap();
    }
    dir
}

/// Runs `ovc` with `args` in `dir`: whether it succeeded.
fn ovc(dir: &Path, args: &[&str]) -> bool {
    Command::new(env!("CARGO_BIN_EXE_ovc")).args(args).current_dir(dir).output().expect("ovc runs").status.success()
}

#[test]
fn the_manifest_sets_the_level_the_compiler_uses() {
    let dir = project("manifest-warn", Some("[lints]\nwidth = \"warn\"\n"));
    assert!(ovc(&dir, &["a.ov"]), "width = \"warn\" in ovc.toml: the finding is not an error");
}

#[test]
fn without_a_manifest_the_finding_is_an_error() {
    let dir = project("manifest-none", None);
    assert!(!ovc(&dir, &["a.ov"]), "width is deny by default");
}

#[test]
fn a_flag_on_the_command_line_overrules_the_manifest() {
    let dir = project("manifest-flag", Some("[lints]\nwidth = \"warn\"\n"));
    assert!(!ovc(&dir, &["--deny", "width", "a.ov"]), "--deny width wins over width = \"warn\"");
}

#[test]
fn a_flag_on_another_category_leaves_the_manifest_level_alone() {
    let dir = project("manifest-other", Some("[lints]\nwidth = \"warn\"\n"));
    assert!(ovc(&dir, &["--allow", "sizing", "a.ov"]), "--allow sizing does not reset width to its default");
}

#[test]
fn lint_reads_the_manifest_too() {
    let dir = project("manifest-lint", Some("[lints]\nwidth = \"warn\"\n"));
    assert!(ovc(&dir, &["lint", "a.ov"]), "ovc lint: a warn finding does not fail the lint");
    let dir = project("manifest-lint-none", None);
    assert!(!ovc(&dir, &["lint", "a.ov"]), "ovc lint: width is deny by default");
}
