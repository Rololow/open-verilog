# Bundles

A `bundle` groups signals with their directions, and methods on them. It is declared from the master's point of view:
a field marked `output` goes from the master to the slave, an `input` goes back.

A bundle port is flattened in the SystemVerilog output: `out` of type `Stream` becomes `out_valid`, `out_ready` and
`out_data`. A bundle signal inside a module connects a master to a slave in one line.

## A valid/ready stream

Open Verilog. `Stream` declares the valid/ready fields and a method. `Top` connects a source and a sink through one
local signal.

```systemverilog ov
bundle Stream<type T> {
  output logic valid;
  input  logic ready;
  output T data;
  fn fire() -> logic { return valid & ready; }       // a transfer happens this cycle
}

module Source(input clk, input rst, master Stream<logic [7:0]> out) {
  clock clk;
  reset rst;

  reg [7:0] n = 0;
  always_ff {
    if (out.fire()) { n <= n + 1; }
  }
  assign out.valid = 1;
  assign out.data = n;
}

module Sink(input clk, input rst, input go, slave Stream<logic [7:0]> in,
            output logic got, output logic [7:0] got_data) {
  clock clk;
  reset rst;

  assign in.ready = go;
  assign got = in.fire();
  assign got_data = in.data;
}

// A local bundle signal connects a master port to a slave port in one go.
module Top(input clk, input rst, input go, output logic got, output logic [7:0] got_data) {
  Stream<logic [7:0]> link;
  Source src(.clk, .rst, .out(link));
  Sink snk(.clk, .rst, .go, .in(link), .got, .got_data);
}
```

SystemVerilog, written by hand, with the same ports. The protocol checks that the compiler generates are left out.

```systemverilog
module SourceH (
  input  logic       clk,
  input  logic       rst,
  output logic       out_valid,
  input  logic       out_ready,
  output logic [7:0] out_data
);
  logic [7:0] n;
  always_ff @(posedge clk) begin
    if (rst)                              n <= '0;
    else if (out_valid && out_ready)      n <= n + 1;
  end
  assign out_valid = 1'b1;
  assign out_data  = n;
endmodule

module SinkH (
  input  logic       clk,
  input  logic       rst,
  input  logic       go,
  input  logic       in_valid,
  output logic       in_ready,
  input  logic [7:0] in_data,
  output logic       got,
  output logic [7:0] got_data
);
  assign in_ready = go;
  assign got      = in_valid && in_ready;
  assign got_data = in_data;
endmodule

module TopH (
  input  logic       clk,
  input  logic       rst,
  input  logic       go,
  output logic       got,
  output logic [7:0] got_data
);
  logic       link_valid, link_ready;
  logic [7:0] link_data;
  SourceH src (.clk, .rst, .out_valid(link_valid), .out_ready(link_ready), .out_data(link_data));
  SinkH   snk (.clk, .rst, .go, .in_valid(link_valid), .in_ready(link_ready), .in_data(link_data),
               .got, .got_data);
endmodule
```

The two versions were compared in simulation over 2000 cycles with a random `go`: same `got` and same `got_data`.

## What the compiler adds

- **Wiring.** One connection per bundle instead of one per signal. `Top` takes 4 lines; the SystemVerilog version
  declares three wires and writes each connection.
- **Protocol checks.** A `valid` that is not held until `ready`, data that changes while waiting, and an unknown
  `valid` are reported by assertions inside `` `ifndef SYNTHESIS ``. `--no-asserts` removes them.
- **Methods.** `out.fire()` is a method of the bundle, not a bit expression to repeat.

## Going further

- A bundle can contain other bundles, and `input` flips the direction of a nested one. Example: an AXI read port is a
  request stream out and a response stream in.
- `Vec<Stream<...>, N>` declares N bundles, for example the ports of N memories.
- `tie_off(b)` gives the idle value to a master bundle that is not driven.
- A bundle can name a protocol monitor (`monitor AxiMonitor<...>;`), instantiated on every slave port in simulation.
- Bundles inside pipelines are not supported yet.

The full description is in the language specification (`docs/spec.md`, sections 6b and 7a').
