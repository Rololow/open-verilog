# Changelog

All notable changes to `ovc` are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the versions follow [Semantic Versioning](https://semver.org/) (`0.x`: any release may change the language).

## [Unreleased]

### Added
- `ovc --version` (and `-V`) prints the version of the build.
- `ovc build -f LIST`: the inputs are read from a file list (one path per line, `//` comments, directories searched).
- `ovc build --in-place`: each output is written next to its source, and `ovc_files.f` lists them.
- `ovc build` copies the SystemVerilog files it finds into the project, and lists them in order with the others.
- `ovc cover LOG...`: coverage report of simulation logs; `ovc build --cover` and `--cover` on a single file.
- `ovc lint`: preprocessor, lint findings by category, `// ovc: ignore`, and the reset, CDC and handshake checks.
- Protocol monitors: a bundle's `monitor` module watches each slave port (AXI4 burst rules, WLAST / RLAST).
- Async reset for pipelines.
- Diagnostics: "did you mean" suggestions for methods, struct and bundle fields, enum members and instance names;
  instances of a loop listed in "has no instance" messages.

### Changed
- `ovc build` writes `files.f` with paths relative to the working directory when they are below it, absolute otherwise.
  A source outside `--root` is an error (use `--root`).
- A `$error` in the design fails the tests too (the testbench counts it, as it counts the handshake checks).
- `--no-asserts` also leaves out the protocol monitors.
- `ovc build` refuses an option in a file list (`+incdir+...`): the sources of `build` are not preprocessed.- An exported struct, a bundle port and a struct view keep their SystemVerilog struct type (`pkg::t`, `.field`).
- Part-selects of an expression are lowered to shifts (`4'ha` for `8'hA5[7:4]`, `1'(8'(a & b) >> i)` for `(a & b)[i]`).

### Fixed
- A unary operator on a negative number is parenthesized (`|-9` is refused by SV, `|(-9)` is not).
- `&&` and `||` with a constant operand give 0 or 1: `2 && x` is `x != 0` (it was `x`), unless `x` is one bit.
- A signed operand of a sum or difference is widened to the sum's width (`acc + $signed(a) * $signed(b)` was extended with zeros by SV).
- A port hides a function of the same name: the call is not inlined into the port's value (it was an invalid call in SV).
- A top-level `val` that calls a function folds it where the functions are known.
- `fold` refused past 200000 terms (a value that uses its accumulator twice doubles at each step; it used to hang).
- A `fn` of statements used as a value: "a fn of statements has no `return` and is called as a statement".
- Two tests with the same name are an error, at the second one.
- A variable written by an `always_ff` and by an `initial` block is an error.
- A pipeline stage's `valid` (the valid bit of the stage the statement is in) is readable, as `ov_valid_<stage>`.
- A pipeline reads a bundle port as it is now (`stall_when(!o.ready)`); only scalar inputs are carried from the instruction.
- A comment or a generated check in a pipeline body is kept, not refused.
- Test loops take `for (a, b) in rows`.
- A config named without a parameter (`Reg<Cfg.w>`) gives its field's value: the generic argument was left unresolved.
- A file that holds only a config has no output in `ovc build` (it has no hardware).
- A module defined in two files: "module 'X' is defined twice (first in FILE)".
- `.as(S)` of a value whose width is not the struct's width is an error.
- A memory map cannot reuse the name of a signal or bundle of the module.
- `tie_off()` takes a bundle signal of the module: a port is refused, and a whole `Vec` is refused (name an element).
- A `match` in a continuous assignment has coverage items (`--cover`), reported at the assignment's line.
- Invalid SystemVerilog for a select of a literal or of a parenthesised expression.
- A select past the width of its expression (`(a + b)[9:4]` of a 9-bit value) is an error.
- A function defined twice in one file is an error, at the second definition.
- A process writing to an input of an instance is an error (only an `assign` drives it).
- Channels of nested bundles get their own valid/ready checks (`ov_hs_m_ar_*`).
- A bare module name on the command line (`--top Top`) is accepted.
- `in` ranges `lo..hi` exclude `hi`.
- `.*` connections are expanded by name, as the Python compiler did.
- `isa`: the `max` length, duplicate fields, zero-width fields and `like` of an unknown format are errors; an unknown
  field of a view names its format.

## [0.1.7] - 2026-10-27

### Added
- `ovc --version`.

## [0.1.6] - 2026-10-26

### Added
- `ovc lint` on plain SystemVerilog projects, `ovc cover`, and the coverage items of `case` and `match`.

### Changed
- The Rust compiler is the reference; the Python compiler is no longer maintained.

## [0.1.5] - 2026-10-26

### Added
- `for` and generate loops in the Rust output.

## [0.1.4] - 2026-10-26

### Added
- Clock-domain crossing checks and the wiki pages for them.

## [0.1.3] - 2026-10-22

### Fixed
- Reset of `reg` declarations.

## [0.1.2] - 2026-10-22

### Fixed
- Struct formatting and array depth.

## [0.1.1] - 2026-10-22

### Fixed
- Continuous integration builds (Linux, macOS, Windows).

## [0.1.0] - 2026-10-21

### Added
- First release: the compiler from `.ov` to SystemVerilog, the `ovc build` project mode, and the test blocks.
