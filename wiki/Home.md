# Open Verilog

Open Verilog (`.ov`) is a SystemVerilog extension. It removes the boilerplate that SystemVerilog makes you write:
generic modules without repeated bounds, FSMs written as a table of states, pipelines whose valid bits and carried
values are generated, delay lines and tables as values, configurations read from a manifest, and tests inside the
design.

The compiler `ovc` translates each `.ov` file into plain SystemVerilog. The output is ordinary code: any simulator or
synthesis tool reads it.

## Pages

- [Installation](Installation): the release binaries, the Windows installer, the VS Code extension, building from source.
- [Language tour](Language-Tour): modules, values, enums and FSMs, configurations, tests.
- [Arrays and delays](Arrays-and-Delays): before and after for arrays of registers and delay lines.
- [Bundles](Bundles): valid/ready streams, flattened ports, and the checks the compiler adds.
- [Types and widths](Types-and-Widths): the width rule, packages, structs, mixing with SystemVerilog.
- [Vectors and methods](Vectors-and-Methods): bit helpers, lists and reductions, methods, loops.
- [Configurations](Configurations): config records, `extends`, `with`.
- [Classes](Classes): traits, `extends`, `override`, modules as parameters.
- [Clock domains](Clock-Domains): synchronizers, asynchronous FIFOs, crossings.
- [Reset and quality of life](Reset-and-Quality-of-Life): implicit clock and reset, inferred widths.
- [Debug and tests](Debug-and-Tests): hierarchy access, probes, test blocks.
- [Instruction sets](Instruction-Sets): one description of the formats.
- [Systolic array](Systolic-Array): a larger design, with input skew.
- [Pipelines](Pipelines): a three-stage multiply-add, before and after.
- [Command line](Command-Line): the options of `ovc`.
- [SystemVerilog in .ov, and lint](SystemVerilog-Compatibility): which plain SV constructs compile, and the state of lint.
- [Status](Status): what compiles, the checks, what is missing.
- [Contributing](Contributing): tests, continuous integration, releases, this wiki.

## Source

The compiler and the examples live in the `open-verilog` repository. The pages of this wiki are kept in its `wiki/`
folder and copied here by the `wiki` workflow.
