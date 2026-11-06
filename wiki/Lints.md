# Lints

The compiler and `ovc lint` report their findings by category. Each category has a level: `allow` drops the finding,
`warn` reports it, `deny` reports it as an error and fails the build. The editor shows the category of a finding as
its code, and links to its section here.

## How to change a level

- In the project, in `ovc.toml`: `[lints]` then `width = "warn"`.
- On the command line: `--allow CAT`, `--warn CAT`, `--deny CAT` (several names separated by commas).
- For one line: `// ovc: ignore CAT`. The finding is silenced, and `ovc audit` lists the ignore so it stays visible.

A level is a decision about the design. Prefer fixing the code; use an ignore only when the code is right and the lint
cannot see it.

## cdc

A signal crosses from one clock domain to another without a synchronizer. Use `Sync`, `PulseSync` or `AsyncFifo` from
the standard library, see [Clock domains](Clock-Domains). If the crossing is safe for another reason, mark the port
`async` or the clocks `same_domain`, and say why in a comment.

## sign

A signed value is mixed with an unsigned one, or extended in an unsigned context, and the bits change. Cast the operand
on purpose (`$signed(x)`, `$unsigned(x)`), or make both operands the same kind.

## handshake

A `ready` depends combinationally on a `valid` (or the reverse). The protocol then can deadlock. Register one side, or
state the dependency is intended with an ignore.

## reset

A reset is declared with one polarity and tested with the other (`reset rst;` and `if (!rst)`), or a reset is used in a
module that does not declare it. Declare the reset with its real polarity: `reset rst low;`. See
[Reset and quality of life](Reset-and-Quality-of-Life#clock-and-reset).

## syntax

A SystemVerilog construct that is not valid as written, for example a bit select of a parenthesised expression
(`(a & b)[3:0]`). Put the expression in a named signal, then select from it.

## duplicate

A typedef, a module or a function is defined twice, in two files or in the same file. Keep one definition.

## carry

An operation needs more bits than its target: `y = a + b` with `y` as wide as `a`. The carry is lost. Either widen the
target, or write the truncation on purpose with `.wrap()`:

```
assign y = (a + b).wrap();   // the carry is dropped on purpose
```

An accumulator (`count <= count + 1`, `acc = acc + x`) and a shift register (`x = x << n`) are not reported: they drop
their bits on purpose by design. A sum assigned to a different signal is reported.

```
```

## sizing

A sum or a product is shifted right on too few bits: `y = (a * b) >> 8` on 8 bits computes the shift on a truncated
product. Compute the product on a wider signal, then shift. Or `.wrap()` the product if the truncation is intended.

## width

A value is wider than its target (`logic [3:0] y = a;` with an 8-bit `a`): the high bits are dropped. Select the bits
you want (`a[3:0]`), widen the target, or `.wrap()` when the drop is intended.

## latch

A combinational block does not assign a signal on every path, so the signal keeps its value: a latch. Give the signal a
default at the top of the block (`y = 0;`), or assign it in every branch. This is a warning by default.

## driver

A signal is driven from two places (two `assign`s, or an `assign` and a process), or never driven. Keep one driver: a
signal written by one process, or a mux that chooses the value. This is an error by default: such a design is invalid
SystemVerilog.

## connection

An instance port is not connected, or a connection names a port that the module does not have. Connect every input,
and check the port name against the module.

## process

A process mixes blocking (`=`) and non-blocking (`<=`) assignments to the same kind of signal, or gives a register an
initial value that synthesis ignores. Use `<=` in clocked blocks and `=` in combinational blocks.

## slice

A part select has its bounds the wrong way round (`[0:3]` on a `[3:0]` vector), or is out of range. Write the bounds
high to low, inside the vector.

## not-analyzed

A part of the file could not be read by the checks (a construct the lint does not follow). The rest of the file is
checked. Nothing to fix in most cases; the category shows where the checks did not look.

## other

A finding that no category above describes. Its message says what it is; a report with its message helps to give it a
category of its own.
