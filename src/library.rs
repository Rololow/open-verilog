//! The standard library: definitions available in every file without an import. A definition with the same name in
//! the user's file takes precedence, and a library module is emitted only where it is instantiated.

use crate::ast::File;
use crate::error::Result;
use crate::parser;

/// The library source (`ovc/stdlib.ov` is the reference; this copy holds the part the compiler supports).
pub const SOURCE: &str = include_str!("stdlib/stdlib.ov");

/// Adds the library's definitions that the file does not define.
pub fn merge(source: &mut File) -> Result<()> {
    let lib = parser::parse_file(SOURCE, "<stdlib>")?;
    let modules: Vec<String> = source.modules.iter().map(|m| m.name.clone()).collect();
    let bundles: Vec<String> = source.bundles.iter().map(|b| b.name.clone()).collect();
    let structs: Vec<String> = source.structs.iter().map(|s| s.name.clone()).collect();
    let enums: Vec<String> = source.enums.iter().map(|e| e.name.clone()).collect();
    for m in lib.modules {
        if !modules.contains(&m.name) {
            source.library.push(m.name.clone());
            source.modules.push(m);
        }
    }
    for b in lib.bundles {
        if !bundles.contains(&b.name) {
            source.bundles.push(b);
        }
    }
    for s in lib.structs {
        if !structs.contains(&s.name) {
            source.structs.push(s);
        }
    }
    for e in lib.enums {
        if !enums.contains(&e.name) {
            source.enums.push(e);
        }
    }
    Ok(())
}
