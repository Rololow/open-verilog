//! Mutation tests of the lints: a clean design gets a known bug injected, and the lint must report it in the category
//! of the bug. The survivors (a bug that is not reported) are the false negatives of the lints.
//!
//! The clean designs are the examples that compile with no finding. The mutants are made on the text:
//! - `drop_wrap`: one `.wrap()` removed. The truncation it marked is now a bit loss: width, carry, sizing or sign.
//! - `duplicate_driver`: one `assign` repeated. The signal is driven from two places: driver.

use std::fs;
use std::path::Path;

use ovc::diag::Category;

/// The findings of a source: the category of each error or warning (none for a clean design).
fn findings(src: &str) -> Vec<Category> {
    match ovc::compile_with(src, "m.ov", &ovc::Options::default()) {
        Err(e) => vec![Category::of_message(&e.message)],
        Ok(c) => c.warnings.iter().map(|w| Category::of_message(w)).collect(),
    }
}

/// The text with the `n`th occurrence of `pat` replaced by `with` (`None` when there are fewer occurrences).
fn replace_nth(src: &str, pat: &str, n: usize, with: &str) -> Option<String> {
    let at = src.match_indices(pat).nth(n)?.0;
    Some(format!("{}{}{}", &src[..at], with, &src[at + pat.len()..]))
}

/// The lines that are a whole `assign` statement, with their index among the lines.
fn assign_lines(src: &str) -> Vec<usize> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| {
            let t = l.trim_start();
            t.starts_with("assign ") && t.trim_end().ends_with(';') && !t.contains("<=")
        })
        .map(|(i, _)| i)
        .collect()
}

/// The clean examples of the repository, with their text.
fn clean_examples() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).expect("the examples folder") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "ov") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        if findings(&src).is_empty() {
            out.push((path.file_name().unwrap().to_string_lossy().to_string(), src));
        }
    }
    out
}

/// One mutant: its description, its text, and the category the lint must report.
struct Mutant {
    what: String,
    text: String,
    expected: &'static [Category],
}

fn mutants(name: &str, src: &str) -> Vec<Mutant> {
    let mut out = Vec::new();
    let wraps = src.matches(".wrap()").count();
    for n in 0..wraps {
        if let Some(text) = replace_nth(src, ".wrap()", n, "") {
            out.push(Mutant {
                what: format!("{name}: drop_wrap #{n}"),
                text,
                expected: &[Category::Width, Category::Carry, Category::Sizing, Category::Sign],
            });
        }
    }
    for line in assign_lines(src) {
        let lines: Vec<&str> = src.lines().collect();
        let mut dup = lines.clone();
        dup.insert(line + 1, lines[line]);
        out.push(Mutant {
            what: format!("{name}: duplicate_driver at line {}", line + 1),
            text: dup.join("\n") + "\n",
            expected: &[Category::Driver],
        });
    }
    out
}

/// Runs every mutant of the clean examples. Returns (total, detected, the survivors).
fn run_all() -> (usize, usize, Vec<String>) {
    let clean = clean_examples();
    assert!(clean.len() >= 5, "too few clean examples: {}", clean.len());
    let (mut total, mut detected, mut survivors) = (0, 0, Vec::new());
    for (name, src) in &clean {
        for m in mutants(name, src) {
            total += 1;
            let got = findings(&m.text);
            if got.iter().any(|c| m.expected.contains(c)) {
                detected += 1;
            } else {
                survivors.push(format!("{} (reported: {:?})", m.what, got));
            }
        }
    }
    (total, detected, survivors)
}

/// The survivors that are accepted on purpose: an accumulator (`x = x + y`) is exempt from the bit-loss checks, so its
/// carry is not reported. A change of that policy must update this list (and the wiki's Lints page).
const ACCEPTED: &[&str] = &["probes.ov: drop_wrap #1", "testbench.ov: drop_wrap #0"];

#[test]
fn the_lints_catch_the_injected_bugs() {
    let (total, detected, survivors) = run_all();
    eprintln!("mutants: {total}, detected: {detected}, survivors: {}", survivors.len());
    for s in &survivors {
        eprintln!("  survivor: {s}");
    }
    assert!(total >= 20, "too few mutants: {total}");
    assert_eq!(detected + survivors.len(), total);
    let unexpected: Vec<&String> = survivors.iter().filter(|s| !ACCEPTED.iter().any(|a| s.starts_with(a))).collect();
    assert!(unexpected.is_empty(), "false negatives of the lints: {unexpected:?}");
    let missing: Vec<&&str> = ACCEPTED.iter().filter(|a| !survivors.iter().any(|s| s.starts_with(*a))).collect();
    assert!(missing.is_empty(), "accepted survivors that are now caught (update ACCEPTED): {missing:?}");
}
