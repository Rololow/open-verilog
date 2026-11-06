# Changelog

All notable changes to `ovc` are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the versions follow [Semantic Versioning](https://semver.org/) (`0.x`: any release may change the language).

## [Unreleased]

### Added
- `ovc-lsp` answers hover: the declaration line of the identifier under the cursor, with its file and line.
- `ovc-lsp` applies the `[lints]` table of the workspace's `ovc.toml`, as the compiler does: a category at `warn` or
  `allow` is not an error in the editor.
- `ovc-lsp` completes the names visible in the module of the cursor, the names of the workspace, and the keywords.
- `ovc-lsp` lists the symbols of a file (document symbols) and of the workspace (workspace symbols, by name).
- `ovc-lsp` finds the references of a name and renames it: a name of a module is renamed in the workspace, a signal or a
  port in its module (a port's connections in the instances too).
- `ovc-lsp` shows the category of each finding as its code, and links it to its section of the wiki's `Lints` page
  (the rule and the correct way past it).
- `ovc-lsp` is faster on a project: a file is parsed again only when its text changes, and a project whose texts and
  lint levels did not change is not compiled again. The design is still elaborated as a whole when one file changes.
- `ovc-lsp` explains the methods (`.wrap()`, `.delay(n)`, ...) and the standard library entries on hover.
- `ovc fmt [--check] PATH...`: the layout of the `.ov` files. Only the whitespace changes, and the tokens are checked
  before a file is written. `--check` lists the files that would change and fails for CI.
- `ovc test [--sim iverilog|verilator|questa] [--out DIR] PATH...`: the `test` blocks run with a simulator: the design and
  the testbench are written to DIR, the simulator runs them, and the log gives the verdict of each test.
- `ovc --keywords zed|vscode`: the Open Verilog words as the pattern of the editor grammars. The editors and the language
  server share this list.

### Fixed
- The bit-loss checks (`width`, `carry`, `sizing`, `sign`) saw neither the statements of a pipeline stage, nor the
  initialisers of declarations (`logic [7:0] a = d + 1;`), nor the signals a pipeline lowering declares. They now do.
  Such a design is refused where it used to compile: write the truncation with `.wrap()`.
- A `.wrap()` exempts its own assignment only. It used to exempt every assignment to the same signal in the module.
- The `[lints]` table of `ovc.toml` had no effect on the command line: the default levels, given to the command line's
  overrides, overruled it (`width = "warn"` was still an error). Only the categories named on the command line now
  overrule the manifest.

## [0.1.21] - 2026-11-05

### Added
- `ovc audit FILES`: the places where a design asks the compiler to trust the author, without changing anything: the
  `async` inputs and declarations (not checked for a clock crossing), the `same_domain` declarations, the `// ovc: ignore`
  comments, the `.wrap()` truncations, and the lint levels below their defaults.
- `domain p : clk;` in a module: the port `p` (an input or an output) is in the clock domain of the input `clk`. The uses
  of `p` are checked against it, and so are the instances that connect it. The domain is inferred without it.
- The standard library has the clock-domain crossings: `Sync`, `PulseSync` (a pulse across clocks) and `AsyncFifo`.
- `--allow CAT,...`, `--warn CAT,...`, `--deny CAT,...`: the level of each category of check, on `ovc lint` and on the
  compiler. At `deny` a finding fails the lint, or the compilation (it is an error, at its position); `// ovc: ignore`
  still silences it. An unknown category name is refused with the list of the known ones. The categories are a typed
  enum (`src/diag.rs`), not the text of the messages. Without `--deny` the compilation is unchanged.
- `ovc.toml` in the working directory: a `[lints]` table sets the level of each category for `ovc build`, `ovc design.ov`
  and `ovc lint` (`width = "deny"`). The command line overrides it.
- lint `[sizing]`: a sum or a product shifted right, evaluated on fewer bits than it needs (`y = (a * b) >> 8` on 8 bits).
- lint `[syntax]`: a select of bits of an expression in SystemVerilog source (`(a & b)[3:0]`), found before elaboration.
- lint and compiler `[duplicate]`: a typedef defined in two files (the first definition is kept in the lint).
- lint `[sign]`: a signed operand mixed with an unsigned one, and a signed operand zero-extended in an unsigned context.
- `break;` in a `for` loop (SystemVerilog and `.ov`): the iterations after it do nothing. The lint reads the loops of plain
  SystemVerilog that use it, so `controller.sv` of tiny-gpu is analysed.
- lint: `// ovc: clock NAME` and `// ovc: reset NAME [low] [async]` in a plain SystemVerilog module declare its clock and
  reset, so the reset and clock-domain checks apply to it. The source is not changed.
- Forwarding is documented: `stage.x` reads the value that a later stage computes now, from the instruction in that stage.
- lint `reset`: a declared reset that the module tests with the other polarity (`reset rst;` and `if (!rst)`).

### Changed
- **Breaking:** a signal driven from two places (`driver` findings) is an error of the compiler, by default. Such a
  design is invalid SystemVerilog; `--warn driver` (or `[lints] driver = "warn"` in `ovc.toml`) restores the warning.
- **Breaking:** the bit-loss findings are errors of the compiler, by default: `width` (a value wider than its target),
  `carry` (an operation that needs more bits than its target), `sizing` (a sum or product shifted right on too few
  bits) and `sign` (a signed operand mixed with an unsigned one, where the bits change). Write the truncation on purpose
  with `.wrap()`, or widen the target. `--warn width` (or `[lints] width = "warn"`) restores the warning. The `latch`
  findings stay warnings by default.
- **Breaking:** an output port that nothing drives is an error of the compiler, not only of the lint (its value is X in
  simulation). Drive it, or `tie_off()` it. Nested channels are checked too.

### Fixed
- A `.wrap()` written in a generate loop is honoured by the width and carry checks: the copies of the signal are named
  with the loop index (`penalty_0`), and they were not matched with the source name (`penalty`).
- The `sign` check honours a `.wrap()`: a signed operand that is as wide as the truncated target does not change its
  bits, so the sum is not reported.
- `stage.x` names the stage that defines `x`: `dec.v`, with `v` defined in a later stage, is an error. It was silently
  resolved to that later `v`.
- An error in a pipeline is reported at the module's own file (it was `<project>`).
- A `localparam` in a generate loop is defined in each iteration: the lint saw its uses as unknown.
- An input that a module reads and that nothing connects is reported, one that it never reads is not (the lint).
- Unknown names say `unknown package 'X'`, `unknown monitor 'X'`, `unknown macro `X`, `unknown module 'X' in a test`, and
  `unknown module 'X'` for a probe (they said `not found` or `is not defined`).
- A `val` without a width is an inline value, not a signal: read from another stage (`ex.v` in `decode`, with `v` a
  `val`), it is refused at the read, with the `logic [N-1:0] v = ...` that makes it a signal. It was reported in the
  stage that reads the value, at no position.
- The driver check of the lint follows bundle vectors (`m[i].aw.valid` drives `m_aw_valid`) and a bundle connected
  whole to an instance (its outputs are driven by the instance). It reported ports that were driven.

## [0.1.9] - 2026-10-27

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
- **Breaking:** the clock and the reset are declared, never guessed. A module with clocked blocks needs `clock <name>;`,
  and a module that resets its registers or pipelines needs `reset <name> [low] [async];`. The names `clk`, `rst`,
  `rst_n` are no longer recognised from the ports. A clocked block without a declared clock is an error, with the line to
  add. The protocol monitors, the probes, the testbenches and the delay registers use the declared names.
- A reset is a declared signal only: an `if (x)` in a block is not a reset because of its name or its body (an enable
  that writes data is not one).
- `ovc build` writes `files.f` with paths relative to the working directory when they are below it, absolute otherwise.
  A source outside `--root` is an error (use `--root`).
- A `$error` in the design fails the tests too (the testbench counts it, as it counts the handshake checks).
- `--no-asserts` also leaves out the protocol monitors.
- `ovc build` refuses an option in a file list (`+incdir+...`): the sources of `build` are not preprocessed.- An exported struct, a bundle port and a struct view keep their SystemVerilog struct type (`pkg::t`, `.field`).
- Part-selects of an expression are lowered to shifts (`4'ha` for `8'hA5[7:4]`, `1'(8'(a & b) >> i)` for `(a & b)[i]`).

### Fixed
- A missing clock is reported at the module that contains the block (a `val` with `.delay()` passed to an instance was
  reported at the instance's module).
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
