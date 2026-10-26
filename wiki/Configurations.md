# Configurations

A `config` is a compile-time record. A module takes one as a parameter and reads its fields. Fields can depend on
earlier fields, a config can derive from another, and a use can override fields.

```ov
config CpuCfg {
  xlen = 32;
  lanes = 2;
  has_mul = false;
  acc_w = xlen + clog2(lanes);           // follows xlen and lanes
}

config BigCfg extends CpuCfg {
  xlen = 64;
  has_mul = true;
}

// One lane per element: add or multiply depending on the config.
module Datapath<config C : CpuCfg = CpuCfg>(
  input  logic [C.xlen-1:0] a [C.lanes],
  input  logic [C.xlen-1:0] b [C.lanes],
  output logic [C.xlen-1:0] y [C.lanes],
  output logic [C.acc_w-1:0] total
) {
  for i in 0..C.lanes {
    if (C.has_mul) {
      assign y[i] = (a[i] * b[i]).wrap();
    } else {
      assign y[i] = (a[i] + b[i]).wrap();
    }
  }
  assign total = y.list().tree_reduce(+);
}

// The same module, three configurations.
module Top(
  input  logic [31:0] a2 [2], input logic [31:0] b2 [2], output logic [31:0] y2 [2], output logic [32:0] t2,
  input  logic [31:0] a4 [4], input logic [31:0] b4 [4], output logic [31:0] y4 [4], output logic [33:0] t4,
  input  logic [63:0] a8 [2], input logic [63:0] b8 [2], output logic [63:0] y8 [2], output logic [64:0] t8
) {
  Datapath d2(.a(a2), .b(b2), .y(y2), .total(t2));
  Datapath<CpuCfg with { lanes = 4 }> d4(.a(a4), .b(b4), .y(y4), .total(t4));
  Datapath<BigCfg> d8(.a(a8), .b(b8), .y(y8), .total(t8));
}
```

## What each part does

- `config CpuCfg { ... }`: the fields. `acc_w` is computed from `xlen` and `lanes`, and changes with them.
- `extends`: `BigCfg` takes every field of `CpuCfg` and overrides two.
- `<config C : CpuCfg = CpuCfg>`: the module takes a config that must have the fields of `CpuCfg`. The default is
  `CpuCfg`.
- `with { lanes = 4 }`: one use overrides a field. The fields that depend on it, such as `acc_w`, are recomputed.
- `d2`, `d4`, `d8`: three instances of the same module, each with its own widths. Their ports have the widths of their
  configuration.

The configs of a design can also come from a TOML manifest: see [Language tour](Language-Tour#configurations).
