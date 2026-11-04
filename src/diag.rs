//! Diagnostics of the checks: the category of each finding, and the level it is reported at.
//!
//! A category is a family of checks (`cdc`, `width`, `driver`...). Its name is the one `ovc lint` prints and that
//! `--only`, `--ignore` and `// ovc: ignore` take. A level says what a finding of the category does: `allow` drops it,
//! `warn` reports it (the default), `deny` reports it and makes the run fail.

use std::collections::HashMap;

use crate::ast::Expr;

/// A family of checks. The names (`as_str`) are stable: they are part of the command line and of the source comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Cdc,
    Sign,
    Handshake,
    Reset,
    Syntax,
    Duplicate,
    Carry,
    Sizing,
    Width,
    Latch,
    Driver,
    Connection,
    Process,
    Slice,
    Other,
    NotAnalyzed,
}

/// Every category, in the order of the documentation.
pub const ALL: [Category; 16] = [
    Category::Cdc,
    Category::Sign,
    Category::Handshake,
    Category::Reset,
    Category::Syntax,
    Category::Duplicate,
    Category::Carry,
    Category::Sizing,
    Category::Width,
    Category::Latch,
    Category::Driver,
    Category::Connection,
    Category::Process,
    Category::Slice,
    Category::Other,
    Category::NotAnalyzed,
];

impl Category {
    /// The name of the category, as the command line and the source comments write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Cdc => "cdc",
            Category::Sign => "sign",
            Category::Handshake => "handshake",
            Category::Reset => "reset",
            Category::Syntax => "syntax",
            Category::Duplicate => "duplicate",
            Category::Carry => "carry",
            Category::Sizing => "sizing",
            Category::Width => "width",
            Category::Latch => "latch",
            Category::Driver => "driver",
            Category::Connection => "connection",
            Category::Process => "process",
            Category::Slice => "slice",
            Category::Other => "other",
            Category::NotAnalyzed => "not analyzed",
        }
    }

    /// The category with this name, if there is one.
    pub fn from_name(name: &str) -> Option<Category> {
        ALL.iter().copied().find(|c| c.as_str() == name)
    }

    /// The category of a message of the checks. The first rule that matches is the one.
    pub fn of_message(msg: &str) -> Category {
        let has = |s: &str| msg.contains(s);
        if has("clock-domain crossing") {
            Category::Cdc
        } else if has("is signed but this expression is unsigned") || has("is extended from") {
            Category::Sign
        } else if has("depends combinationally on") && has(".ready") {
            Category::Handshake
        } else if msg.starts_with("reset '") {
            Category::Reset
        } else if has("not valid SystemVerilog") {
            Category::Syntax
        } else if has("is already defined at") {
            Category::Duplicate
        } else if has("needs") && has(" bits") {
            Category::Carry
        } else if ["carry is lost", "is evaluated on", "bits wide (an unsized literal", "before the shift"].iter().any(|s| has(s)) {
            Category::Sizing
        } else if has("bits but") || has("does not fit") || has("are dropped") || (has(" bits wide") && has("is ")) {
            Category::Width
        } else if has("latch") {
            Category::Latch
        } else if has("driven") || has("never driven") || has("several places") {
            Category::Driver
        } else if has("not connected") || (has("port '") && has(" of")) {
            Category::Connection
        } else if ["blocking assignment", "non-blocking", "initial value", "initializer", "reg '"].iter().any(|s| has(s)) {
            Category::Process
        } else if has("opposite way") || has("out of range") {
            Category::Slice
        } else if ["negative", "signed", "unsigned"].iter().any(|s| has(s)) {
            Category::Sign
        } else {
            Category::Other
        }
    }
}

/// The levels of the project: the `[lints]` of `ovc.toml` in the working directory, or none when there is no such file.
pub fn project_levels() -> Result<Levels, String> {
    match std::fs::read_to_string("ovc.toml") {
        Ok(text) => Levels::from_manifest(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Levels::default()),
        Err(e) => Err(format!("ovc.toml: {e}")),
    }
}

/// What a finding of a category does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The finding is dropped.
    Allow,
    /// The finding is reported (the default).
    Warn,
    /// The finding is reported, and the run fails.
    Deny,
}

/// The level of each category. A category without a setting is at `warn`.
#[derive(Debug, Clone, Default)]
pub struct Levels(HashMap<Category, Level>);

impl Levels {
    pub fn set(&mut self, cat: Category, level: Level) {
        self.0.insert(cat, level);
    }

    pub fn get(&self, cat: Category) -> Level {
        self.0.get(&cat).copied().unwrap_or(Level::Warn)
    }

    /// Whether some category is at `deny`: the compiler then runs the checks, and their `deny` findings are errors.
    pub fn any_deny(&self) -> bool {
        self.0.values().any(|l| *l == Level::Deny)
    }

    /// Copies the levels of `other` over these: a category that `other` sets takes its level there.
    pub fn extend(&mut self, other: &Levels) {
        for (cat, level) in &other.0 {
            self.0.insert(*cat, *level);
        }
    }

    /// The levels of a manifest (`ovc.toml`): a `[lints]` table, `category = "allow" | "warn" | "deny"`.
    pub fn from_manifest(text: &str) -> Result<Levels, String> {
        let mut levels = Levels::default();
        for (key, value) in crate::toml::parse(text).map_err(|e| format!("ovc.toml: {e}"))? {
            if key != "lints" {
                continue;
            }
            let Expr::Pattern(entries) = value else { return Err("ovc.toml: [lints] must be a table".into()) };
            for entry in entries {
                let Expr::Keyed(cat, value) = entry else { continue };
                let Expr::Num(text) = *value else { return Err(format!("ovc.toml: [lints] {cat}: the level must be a string")) };
                let level = match text.trim_matches('"') {
                    "allow" => Level::Allow,
                    "warn" => Level::Warn,
                    "deny" => Level::Deny,
                    other => return Err(format!("ovc.toml: [lints] {cat}: unknown level '{other}' (allow, warn or deny)")),
                };
                levels.set_list(&cat, level).map_err(|e| format!("ovc.toml: [lints]: {e}"))?;
            }
        }
        Ok(levels)
    }

    /// Sets the level of each category of a comma-separated list (`cdc,width`). An unknown name is an error, which
    /// names the known ones.
    pub fn set_list(&mut self, list: &str, level: Level) -> Result<(), String> {
        for name in list.split(',').filter(|n| !n.is_empty()) {
            match Category::from_name(name) {
                Some(cat) => self.set(cat, level),
                None => {
                    let known: Vec<&str> = ALL.iter().map(|c| c.as_str()).collect();
                    return Err(format!("unknown category '{name}' (the categories are: {})", known.join(", ")));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_map_to_their_category() {
        assert_eq!(Category::of_message("clock-domain crossing in module 'M'"), Category::Cdc);
        assert_eq!(Category::of_message("'x' is driven from several places (assign and always_comb)"), Category::Driver);
        assert_eq!(Category::of_message("reset 'rst' is used both ways"), Category::Reset);
        assert_eq!(Category::of_message("'n' is not assigned on every path of this always_comb (a latch would be inferred)"), Category::Latch);
        assert_eq!(Category::of_message("something nobody has classified"), Category::Other);
    }

    #[test]
    fn names_round_trip() {
        for c in ALL {
            assert_eq!(Category::from_name(c.as_str()), Some(c));
        }
        assert_eq!(Category::from_name("nope"), None);
    }

    #[test]
    fn levels_default_to_warn_and_take_a_list() {
        let mut levels = Levels::default();
        assert_eq!(levels.get(Category::Width), Level::Warn);
        levels.set_list("width,cdc", Level::Deny).unwrap();
        levels.set_list("latch", Level::Allow).unwrap();
        assert_eq!(levels.get(Category::Width), Level::Deny);
        assert_eq!(levels.get(Category::Cdc), Level::Deny);
        assert_eq!(levels.get(Category::Latch), Level::Allow);
        assert_eq!(levels.get(Category::Driver), Level::Warn);
    }

    #[test]
    fn a_manifest_sets_the_levels_of_its_lints_table() {
        let levels = Levels::from_manifest("name = \"top\"\n\n[lints]\nwidth = \"deny\" # the carries\ncdc = \"allow\"\n").unwrap();
        assert_eq!(levels.get(Category::Width), Level::Deny);
        assert_eq!(levels.get(Category::Cdc), Level::Allow);
        assert_eq!(levels.get(Category::Latch), Level::Warn);
    }

    #[test]
    fn a_manifest_refuses_an_unknown_category_or_level() {
        let err = Levels::from_manifest("[lints]\nwidht = \"deny\"\n").unwrap_err();
        assert!(err.contains("unknown category 'widht'"), "{err}");
        let err = Levels::from_manifest("[lints]\nwidth = \"error\"\n").unwrap_err();
        assert!(err.contains("unknown level 'error'"), "{err}");
    }

    #[test]
    fn explicit_levels_override_the_manifest() {
        let mut levels = Levels::from_manifest("[lints]\nwidth = \"deny\"\n").unwrap();
        let mut cli = Levels::default();
        cli.set(Category::Width, Level::Allow);
        levels.extend(&cli);
        assert_eq!(levels.get(Category::Width), Level::Allow);
    }

    #[test]
    fn an_unknown_name_is_refused_with_the_known_ones() {
        let err = Levels::default().set_list("widht", Level::Deny).unwrap_err();
        assert!(err.contains("unknown category 'widht'") && err.contains("width"), "{err}");
    }
}
