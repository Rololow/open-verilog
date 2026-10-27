# Language tour

This page shows the main constructs with small complete examples. Each `ov` block is compiled by the CI, so the
examples on this page stay valid.

## Generic modules

A module takes integer and type parameters. Bounds and widths come from them, once.

```systemverilog ov
module Adder<int W = 8>(input logic [W-1:0] a, input logic [W-1:0] b, output logic [W:0] s) {
  assign s = a + b;
}
```

A module with a configuration parameter reads its settings from a `config` (see [Configurations](#configurations)).

## Values

`val` names a value computed at compile time or a signal of the right width. The width is inferred, so you do not
write it twice.

```systemverilog ov
module Scale<int N = 4>(input logic [7:0] x, output logic [7:0] y) {
  val K = N * 2;
  assign y = (x * K).wrap();
}
```

`.wrap()` marks a truncation as intended: the arithmetic under it is computed exactly, then truncated to the width of the
target.

## Enums and FSMs

An enum is a type with named members. An `fsm` block lists the states and the transitions; it uses the clock and the
reset the module declares.

```systemverilog ov
enum State { IDLE, BUSY, DONE }

module Handshake(input clk, input rst, input req, output logic ack, output logic busy) {
  clock clk;
  reset rst;

  fsm State state = IDLE {
    ack = 0;                          // default values (avoid latches)
    busy = 0;
    IDLE: { if (req) goto BUSY; }
    BUSY: { busy = 1; goto DONE; }
    DONE: { ack = 1; goto IDLE; }
  }
}
```

In SystemVerilog the same FSM is a state register, a next-state `case`, an output `case` and the reset branch.

## Arrays and delays

Registers can be arrays, and `.delay(n)` builds a delay line. See [Arrays and delays](Arrays-and-Delays).

## Pipelines

A `pipeline` declares stages. The registers between stages, the valid bits and the carried values are generated. See
[Pipelines](Pipelines).

## Configurations

A `config` groups the constants of a design. It can be written in the source, or read from a TOML manifest:

```
config V3 from "v3.toml" { K = array.k; }
```

A module that takes `<config C = V3>` reads `C.K`, and a section of the manifest is read as `C.tables.name`. The
manifest is read from the folder of the source file.

## Tests

`test` blocks drive a module from the design file. `ovc --emit-tests tb.sv` writes them as a SystemVerilog testbench.
See [Command line](Command-Line).
