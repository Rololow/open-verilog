# Open Verilog

[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.24311669.svg)](https://doi.org/10.5281/zenodo.24311669)

**Write hardware in a language that catches design mistakes before they reach simulation, and keep every line of
SystemVerilog you already have.**

Open Verilog (`.ov`) is a SystemVerilog extension. Its compiler, `ovc`, turns it into plain SystemVerilog that any
simulator or synthesis tool accepts, and it lints existing SystemVerilog projects in place.

## Why Open Verilog

- **Clock-domain crossings are checked by the compiler.** A signal read in another clock domain is an error unless the
  crossing is explicit. Resets that do not match their clock domain are reported.
- **Undriven outputs are errors, not surprises.** A port nobody drives stops the build.
- **A linter that knows the hardware.** Width and sign mismatches, latches, multiple drivers, unbalanced pipeline
  latencies and blocking assignments in `always_ff` are reported with their position, and each category can be set to
  `allow`, `warn` or `deny`.
- **Compiles to readable SystemVerilog.** The output keeps a stable layout, so a generated file can be reviewed, diffed
  and debugged like hand-written code.
- **Built-in high-level constructs.** Pipelines with valid bits, stalls and flushes, finite state machines, structs,
  enums, bundles, configs, pure functions that are inlined at each call, testbenches and probes are part of the
  language, not a library you have to maintain.
- **Migrate at your own pace.** It accepts existing SystemVerilog: `ovc lint` checks a SystemVerilog project directly,
  and the parts it does not understand yet are reported, not silently skipped.
- **Editor support out of the box.** The language server gives diagnostics as you type, go to definition, and hover
  information. A VS Code extension installs and starts it.

## Install

Pre-built binaries for Windows, Linux and macOS (Intel and Apple Silicon) are attached to each release, with a
Windows installer and a `SHA256SUMS` file. From source:

```
cargo build --release
```

## Quick start

```
ovc design.ov -o design.sv        # compile to SystemVerilog
ovc build src/ -o out             # every .ov under src/, one .sv per file, and a file list
ovc lint design.sv                # check a SystemVerilog design
```

The full command line, the language and the lint categories are in the wiki:
<https://github.com/Rololow/open-verilog/wiki>

## Cite

If you use Open Verilog in academic work, see [`CITATION.cff`](CITATION.cff).

## Contributing

Architecture, test layout, development rules and the current status are in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Licence

MIT, see [`LICENSE`](LICENSE).
