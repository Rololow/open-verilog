# Vectors, methods and loops

## Bit helpers

A hardware value has methods for the usual bit operations. They return hardware, so they can be used anywhere a value
can:

- `a.reverse()`: the bits in reverse order;
- `a.fill(n)`: `a` repeated `n` times, as `{n{a}}`;
- `a.zext(w)`, `a.sext(w)`: zero- or sign-extension to `w` bits;
- `a.msb()`, `a.lsb()`: the first and last bit.

```systemverilog ov
module Bits(
  input  logic [7:0] a,
  input  logic [3:0] b,
  output logic [11:0] c,      // {a, b}    = {a, b}
  output logic [7:0] r,       // a.reverse() = bit-reversed a
  output logic [15:0] f,      // a.fill(2)   = {2{a}}
  output logic [15:0] z,      // a.zext(16)  = zero-extended
  output logic [15:0] s,      // a.sext(16)  = sign-extended
  output logic m,             // a.msb()
  output logic l              // a.lsb()
) {
  assign c = {a, b};
  assign r = a.reverse();
  assign f = a.fill(2);
  assign z = a.zext(16);
  assign s = a.sext(16);
  assign m = a.msb();
  assign l = a.lsb();
}
```

## Lists and reductions

`v.list()` turns an array signal into a list known at compile time: `v[0]`, `v[1]`, ... A list has `map`, `filter`,
`reduce` and `tree_reduce`. `tree_reduce` combines the elements as a balanced tree instead of a chain: for four inputs,
the adder below is `in[0] + in[1] + (in[2] + in[3])`. The tree is built at elaboration.

```systemverilog ov
module Adder<int N = 4, int W = 8>(
  input  logic [W-1:0] in [N],
  output logic [W+clog2(N)-1:0] sum
) {
  assign sum = (0..N).map(i => in[i]).tree_reduce(+);
}

module VecSum<int N = 4, int W = 8>(input Vec<logic [W-1:0], N> in, output logic [W+clog2(N)-1:0] sum) {
  assign sum = in.list().tree_reduce(+);
}
```

## Methods with arguments

A module can declare a method with `fn`. A call is combinational, and each call site gets its own ports, so several
reads can happen in the same cycle. Here a register file has two read ports and a third, fixed one.

```systemverilog ov
module RegFile(input clk, input rst, input logic we, input logic [1:0] wa, input logic [7:0] wd) {
  clock clk;
  reset rst;

  logic [7:0] mem [4];
  always_ff @(posedge clk) {
    if (we) { mem[wa] <= wd; }
  }
  fn read(logic [1:0] i) -> logic [7:0] { return mem[i]; }
}

module Dual(input clk, input rst, input logic we, input logic [1:0] wa, input logic [7:0] wd,
            input logic [1:0] ra, input logic [1:0] rb, output logic [7:0] a, output logic [7:0] b,
            output logic [7:0] last) {
  RegFile rf(.clk, .rst, .we, .wa, .wd);
  assign a = rf.read(ra);
  assign b = rf.read(rb);
  assign last = rf.read(2'd3);
}
```

## Methods on structs

A struct can have methods. Its field names are usable inside them.

```systemverilog ov
struct Pixel {
  logic [7:0] r;
  logic [7:0] g;
  logic [7:0] b;
  fn sum() -> logic [9:0] { return r + g + b; }
  fn brighter(logic [7:0] k) -> logic { return sum() > k; }
}

module Luma(input Pixel p, input logic [7:0] k, output logic [9:0] s, output logic bright) {
  assign s = p.sum();
  assign bright = p.brighter(k);
}
```

## Loops and generated logic

`for` at module level generates hardware, once per iteration. Inside `always_comb`, a `for` unrolls into logic. Several
values can share one `case` arm.

```systemverilog ov
module Cell<int W = 8>(input clk, input logic [W-1:0] d, output logic [W-1:0] q) {
  clock clk;

  always_ff @(posedge clk) { q <= d; }
}

module Shift<int N = 4, int W = 8>(input clk, input logic [W-1:0] d, output logic [W-1:0] q) {
  clock clk;

  logic [W-1:0] stage [N+1];
  assign stage[0] = d;
  for i in 0..N {
    Cell<W> c(.clk(clk), .d(stage[i]), .q(stage[i+1]));
  }
  assign q = stage[N];
}

module Parity<int W = 8>(input logic [W-1:0] x, output logic p) {
  always_comb {
    p = 0;
    for i in 0..W { p = p ^ x[i]; }
  }
}

module Sel(input logic [1:0] s, input logic [3:0] a, output logic y) {
  always_comb {
    case (s) {
      0, 1: y = a[0];
      2: y = a[1] & a[2] | a[3];
      default: y = (a[0] + a[1]) * 2 == 0 ? 1 : 0;
    }
  }
}
```
