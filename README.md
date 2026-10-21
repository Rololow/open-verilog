# ovc (Rust)

Compiler for **Open Verilog** (`.ov`), a SystemVerilog extension, to plain SystemVerilog. This is the Rust port of
the Python reference compiler (`ovc/` at the root of the repository), which stays the reference for behaviour and
output: a construct is done when the compiler's output for the examples is byte-identical to the reference's.

The crate uses the standard library only.

## Build and run

```
cargo build --release
target/release/ovc design.ov -o design.sv              # one file
target/release/ovc a.ov b.ov                            # several files, SystemVerilog concatenated on stdout
target/release/ovc design.ov --emit-tests tb.sv         # the `test` blocks as a testbench
target/release/ovc design.ov --emit-probes probes.sv    # the `probe` blocks, bound into the design
target/release/ovc design.ov --dump-pipeline            # the stage table of each pipeline (stderr)
target/release/ovc design.ov --no-asserts               # without the simulation-only checks
target/release/ovc design.ov --werror                   # warnings are errors
```

Files are compiled in parallel, one thread per file; the output keeps the order of the arguments.

## Status

All 22 examples in `../examples` compile. 19 of them produce output identical to the reference (`../expected`), and
these are checked by the golden tests in `tests/golden.rs`:

`adder`, `bits`, `bundle`, `cdc`, `classes`, `config`, `counter`, `fsm`, `hier`, `isa`, `loops`, `methods`, `mixed`,
`pipe`, `probes`, `systolic`, `testbench`, `types`, `widths`.

`chain`, `pipeline` and `qol` compile but are not golden: their pipelines name the valid bits and carried registers
after the stages (`ov_valid_add`, `s__add`) instead of by index, a deliberate change for readability. Their behaviour
was checked by simulation (see the project notes).

### Not yet implemented

- The standard library (`ovc/stdlib.ov`): the crate ships the part it supports (`Stream`, `Fifo`, `Slice`, in
  `src/stdlib/stdlib.ov`). `Arbiter` needs statement-level `val` and arrays of bundles; the AXI monitors need the
  `monitor` role.
- Output modes of the reference: `--emit-isa-sv`, `--emit-isa-py`, `--emit-doc`, `--emit-c`, `--emit-py`,
  `--emit-dts`, `--emit-tb`, `ovc build` (multi-file projects), `ovc lint`, `ovc cover`.
- Pipeline-level `clock` and `reset` directives (pipelines use the `clk` and `rst` ports).
- `memmap`, `regmap` and `export struct` (layouts shared with software).

Anything not supported is reported with a positioned error, not silently dropped.

## Layout

| Module | Role |
| --- | --- |
| `lexer`, `parser` | tokens; recursive descent over the whole grammar used by the examples |
| `ast` | the syntax tree, shared by elaboration and emission |
| `elab` | parameters, constant folding, generate, specialisation, structs, bundles, configs, enums, `fsm`, `val` widths |
| `hier` | connections across the hierarchy: reads, drives and method calls become ports |
| `pipeline` | stage lowering: carries, valid bits, stall and flush, simulation checks |
| `emit` | SystemVerilog text, with the reference's layout and parentheses rules |
| `testgen`, `probes` | the testbench and the probe modules |
| `library` | the standard library loader |

## Tests

```
cargo test
cargo clippy --all-targets
```

The golden tests compare with `../expected`. Unit tests in `tests/` cover diagnostics (positions, messages), the
pipeline, declarations, the testbench, the probes and the library.

## Development notes

- The reference is the Python compiler: when the two differ on a construct the examples use, the Rust output is
  changed to match.
- Every new feature starts from an example in `../examples` and its expected output in `../expected`.
