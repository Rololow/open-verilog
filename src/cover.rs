//! `ovc cover LOG...`: the coverage of a simulation built with `--cover` and run with `OVC_COVER` defined. Every
//! instance prints, at the end of the simulation, one line per coverage item: `ovc-cover <0|1> <file:line> <what>` (an
//! arm of a case taken, a state handled by a default arm visited). An item is covered when any instance, in any test,
//! covered it. The report lists what nothing covered, by file and line.

use std::collections::BTreeMap;

/// The items of the logs: (file, line, what) -> covered.
pub type Items = BTreeMap<(String, usize, String), bool>;

/// The coverage lines of the logs.
pub fn collect<'a>(lines: impl IntoIterator<Item = &'a str>) -> Items {
    let mut items: Items = BTreeMap::new();
    for raw in lines {
        let Some(at) = raw.find("ovc-cover ") else { continue };
        let rest = &raw[at + "ovc-cover ".len()..];
        let mut parts = rest.splitn(3, ' ');
        let (Some(flag), Some(location), Some(what)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Some((file, line)) = location.rsplit_once(':') else { continue };
        let Ok(line) = line.parse::<usize>() else { continue };
        if flag != "0" && flag != "1" {
            continue;
        }
        let entry = items.entry((file.to_string(), line, what.trim().to_string())).or_insert(false);
        *entry |= flag == "1";
    }
    items
}

/// The report, and its exit code: 1 when there is no coverage line at all.
pub fn report(items: &Items) -> (String, i32) {
    if items.is_empty() {
        return ("ovc cover: no coverage lines (build with --cover, simulate with OVC_COVER defined)\n".into(), 1);
    }
    let missed: Vec<&(String, usize, String)> = items.iter().filter(|(_, hit)| !**hit).map(|(k, _)| k).collect();
    let hits = items.len() - missed.len();
    let mut out = format!("ovc cover: {hits} of {} items covered ({:.1}%)\n", items.len(), 100.0 * hits as f64 / items.len() as f64);
    let mut last: Option<&str> = None;
    for (file, line, what) in missed {
        if last != Some(file.as_str()) {
            out.push_str(&format!("  {file}\n"));
            last = Some(file.as_str());
        }
        out.push_str(&format!("    {line}: {what}\n"));
    }
    (out, 0)
}

/// `ovc cover LOG...` on the command line.
pub fn main(logs: &[String]) -> i32 {
    if logs.is_empty() {
        eprintln!("usage: ovc cover LOG...");
        return 2;
    }
    let mut text = String::new();
    for path in logs {
        match std::fs::read(path) {
            Ok(bytes) => text.push_str(&String::from_utf8_lossy(&bytes)),
            Err(e) => {
                eprintln!("ovc cover: {path}: {e}");
                return 2;
            }
        }
    }
    let (out, code) = report(&collect(text.lines()));
    print!("{out}");
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_covered_when_any_run_covered_it() {
        let items = collect(["ovc-cover 0 a.ov:3 case arm 1", "ovc-cover 1 a.ov:3 case arm 1", "ovc-cover 0 a.ov:9 case arm default"]);
        let (out, code) = report(&items);
        assert_eq!(code, 0);
        assert!(out.starts_with("ovc cover: 1 of 2 items covered (50.0%)"), "{out}");
        assert!(out.contains("    9: case arm default"), "{out}");
    }

    #[test]
    fn no_coverage_line_is_an_error() {
        let (out, code) = report(&collect(["nothing here"]));
        assert_eq!(code, 1);
        assert!(out.contains("no coverage lines"));
    }
}
