# Command line

```
ovc design.ov -o design.sv              # one file
ovc a.ov b.ov                           # several files, SystemVerilog concatenated on stdout
ovc build src/ -o out                   # every .ov under src/: one .sv per file, and a file list
ovc build -f files.f --in-place         # the inputs from a file list, each .sv written next to its source (ovc_files.f)
ovc build src/ --root src -o out       # the root of the mirrored folders (a source outside it is an error)
ovc audit src/                          # the async ports, same_domain, ignores, .wrap() truncations, lowered levels
ovc fmt src/                            # the layout of the .ov files, in place (the design is checked to be the same)
ovc fmt --check src/                    # lists the files that would change, and fails if there are some (for CI)
ovc --keywords zed                      # the Open Verilog words, as the pattern of the editor grammars
ovc test src/                          # the test blocks, run with a simulator (iverilog, verilator or vsim on the PATH)
ovc test src/ --sim questa --out out/test  # the same with Questa; the design and the testbench are kept in out/test

The sources of `build` are not preprocessed: a file list with an option such as `+incdir+DIR` is refused. To check plain
.sv files with their include directories and defines, use `ovc lint -f files.f`.
ovc --version                          # the version of this build
ovc design.ov --emit-tests tb.sv        # the `test` blocks as a testbench
ovc design.ov --emit-probes probes.sv   # the `probe` blocks, bound into the design
ovc design.ov --emit-isa-sv isa.sv      # the field positions of each instruction set (SystemVerilog package)
ovc design.ov --emit-isa-py isa.py      # a Python encoder and decoder for the instruction sets
ovc design.ov --emit-doc doc.md         # the documentation of the instruction sets and register maps
ovc design.ov --dump-pipeline           # the stage table of each pipeline (stderr)
ovc design.ov --no-asserts              # without the simulation-only checks
ovc design.ov --werror                  # warnings are errors
```

Files are compiled in parallel, one thread per file. The output keeps the order of the arguments.

`ovc-lsp` is the language server. It speaks LSP on stdio: any editor with an LSP client can start it. The VS Code
extension does it for you.

## Errors

An error has a position (`file:line:column`), the message, and the source line with a caret under the problem. Anything
the compiler does not support is reported this way; nothing is dropped silently.

## Lint and coverage

- `ovc lint FILE.sv... [-f FILES.f] [+incdir+DIR] [-DNAME[=VALUE]] [--top NAME] [--only CAT,...] [--ignore CAT,...] [--allow CAT,...] [--warn CAT,...] [--deny CAT,...] [--werror]`:
  the checks of plain SystemVerilog files, without rewriting them (see [SystemVerilog in .ov, and lint](SystemVerilog-Compatibility)).
  The categories are at `warn` by default, except `driver` (a signal driven from two places), and the bit losses `width`,
  `carry`, `sizing` and `sign`, which are `deny`: a truncation written on purpose is `.wrap()`.
  `--allow` drops its findings, `--deny` reports them and fails the run (the exit code is 1); an unknown category name
  is an error.
- The same levels can be set for a project in `ovc.toml`, in the working directory, with a `[lints]` table:
  `width = "deny"`. The command line overrides it. It applies to `ovc build`, `ovc design.ov` and `ovc lint`.
- `ovc design.ov --cover`: adds the coverage items of the case statements, as flags printed at the end of a simulation
  run with `OVC_COVER` defined.
- `ovc cover LOG...`: the coverage report of those simulation logs. Items are covered when any run covered them; the
  report lists what nothing covered, by file and line.

Not in the Rust compiler yet: the coverage of a `match` in an `always` block (a `match` in a continuous assignment gets its
items, reported at the assignment's line).
