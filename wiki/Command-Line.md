# Command line

```
ovc design.ov -o design.sv              # one file
ovc a.ov b.ov                           # several files, SystemVerilog concatenated on stdout
ovc build src/ -o out                   # every .ov under src/: one .sv per file, and a file list
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

- `ovc lint FILE.sv... [-f FILES.f] [+incdir+DIR] [-DNAME[=VALUE]] [--top NAME] [--only CAT,...] [--ignore CAT,...] [--werror]`:
  the checks of plain SystemVerilog files, without rewriting them (see [SystemVerilog in .ov, and lint](SystemVerilog-Compatibility)).
- `ovc design.ov --cover`: adds the coverage items of the case statements, as flags printed at the end of a simulation
  run with `OVC_COVER` defined.
- `ovc cover LOG...`: the coverage report of those simulation logs. Items are covered when any run covered them; the
  report lists what nothing covered, by file and line.

Not in the Rust compiler yet: the coverage of `match` expressions (only `case` statements get items).
