# Status and compatibility

## Examples

All the examples in `examples/` compile with `ovc`. Their output is checked byte for byte by the golden tests
(`expected/`), and the golden tests check that byte for byte:

`adder`, `bits`, `bundle`, `cdc`, `classes`, `config`, `counter`, `fsm`, `hier`, `isa`, `loops`, `methods`, `mixed`,
`nested`, `pipe`, `probes`, `structs`, `systolic`, `testbench`, `types`, `widths`.

`chain`, `pipeline` and `qol` compile but are not golden. Their pipelines name the valid bits and the carried registers
after the stages (`ov_valid_add`, `s__add`) instead of by index. That is a deliberate change, for readability. Their
behaviour was checked by simulation.

## The Python compiler

The Python compiler in `ovc/` is no longer maintained. This compiler is the reference: the golden files in
`expected/` are its output, checked by simulation. A few things were checked against the Python output when it was
still the reference, for example the clock-domain checks and the instruction-set outputs.

## Not implemented

- `lint` and `cover` subcommands.
