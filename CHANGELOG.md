# Changelog

All notable changes to `ovc` are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the versions follow [Semantic Versioning](https://semver.org/) (`0.x`: any release may change the language).

## [Unreleased]

### Added
- `ovc --version` (and `-V`) prints the version of the build.
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
- An exported struct, a bundle port and a struct view keep their SystemVerilog struct type (`pkg::t`, `.field`).
- Part-selects of an expression are lowered to shifts (`4'ha` for `8'hA5[7:4]`, `1'(8'(a & b) >> i)` for `(a & b)[i]`).

### Fixed
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
