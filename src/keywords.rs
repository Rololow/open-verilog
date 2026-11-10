//! The words of Open Verilog that are not SystemVerilog. The editors colour them and the language server completes them:
//! `ovc --keywords zed` and `ovc --keywords vscode` print the pattern for the editor grammars, so that the lists cannot
//! diverge.

/// The words of Open Verilog itself (they are identifiers to the SystemVerilog grammar, so the editors match them by
/// name). The words of the test blocks are here too.
pub const OV_WORDS: &[&str] = &[
    "pipeline", "stage", "bundle", "trait", "isa", "regmap", "memmap", "fsm", "fn", "val", "const", "probe", "config",
    "test", "expect", "wait_until", "timeout", "stall_when", "flush_when", "kill_when", "younger_when", "tie_off",
    "reset", "clock", "domain", "same_domain", "async", "monitor", "master", "slave",
];

/// The SystemVerilog words the language server completes as well.
pub const SV_WORDS: &[&str] = &[
    "module", "input", "output", "inout", "logic", "reg", "wire", "int", "always_ff", "always_comb", "assign", "if",
    "else", "for", "in", "match", "case", "return", "task", "struct", "enum",
];

/// The pattern of the Zed highlight query: `^(a|b|...)$`, matched against an identifier.
pub fn zed_pattern() -> String {
    format!("^({})$", OV_WORDS.join("|"))
}

/// The alternation of a VS Code (TextMate) rule: `a|b|...`, to put between `\b(?:` and `)\b`.
pub fn vscode_alternation() -> String {
    OV_WORDS.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_are_identifiers_and_distinct() {
        let mut seen = std::collections::HashSet::new();
        for w in OV_WORDS.iter().chain(SV_WORDS) {
            assert!(w.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{w}");
            assert!(seen.insert(*w), "{w} is listed twice");
        }
    }

    #[test]
    fn the_patterns_hold_every_word() {
        assert!(zed_pattern().starts_with("^(pipeline|"));
        assert!(zed_pattern().contains("|reset|clock|"));
        assert_eq!(vscode_alternation().split('|').count(), OV_WORDS.len());
    }
}
